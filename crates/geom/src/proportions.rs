//! A11: true proportions and page size (specification A11). Owner: WP1.

use crate::GeomError;
use scancraft_core::camera::{Camera, FocalSource};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{MAX_PAGE_PIXELS, MAX_SIDE};
use scancraft_core::recipe::{FocalInput, Paper, Proportions, RatioSource};

/// The typical focal length of a phone's main camera (A11 step 3).
const TYPICAL_FOCAL: f64 = 0.75;
/// A focal length outside this range is not believable (A11 step 3).
const FOCAL_MIN: f64 = 0.35;
const FOCAL_MAX: f64 = 4.0;
/// The ratio's believable range (A11 step 4).
const RATIO_MIN: f64 = 1.0 / 20.0;
const RATIO_MAX: f64 = 20.0;
/// How close a ratio must be to a standard one to snap to it (A11 step 5).
const SNAP_TOLERANCE: f64 = 0.03;
/// The diagonal of a 35 mm frame, in millimetres (A11 step 3).
const FRAME_DIAGONAL_MM: f64 = 43.27;
/// The camera frame shapes EXIF's focal length may be read against (A11 step 3).
const FULL_FRAME_SHAPES: [f64; 3] = [4.0 / 3.0, 3.0 / 2.0, 16.0 / 9.0];

/// What A11 needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProportionsInput {
    /// Refined corners in photo pixels.
    pub corners: Quad,
    /// Photo size in pixels.
    pub photo: (u32, u32),
    pub focal: FocalInput,
    /// Snap to standard paper sizes (Settings, on by default).
    pub snap: bool,
    /// The default paper of Settings, for unsnapped pages and the ISO A series.
    pub default_paper: Paper,
}

/// What A11 gives back.
#[derive(Debug, Clone, PartialEq)]
pub struct ProportionsOutput {
    pub proportions: Proportions,
    /// The camera A11 settled on (step 3), which A27 starts from.
    pub camera: Camera,
    /// Physical size in millimetres (step 6).
    pub page_size_mm: (f64, f64),
}

/// A11 steps 1–7.
pub fn true_proportions(input: &ProportionsInput) -> Result<ProportionsOutput, GeomError> {
    let (w, h) = (f64::from(input.photo.0), f64::from(input.photo.1));
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err(GeomError::Degenerate("the photo has no pixels"));
    }
    if !input.corners.is_finite() {
        return Err(GeomError::Degenerate("the corners are not finite"));
    }
    let long = w.max(h);
    check_quad(&input.corners, long)?;

    // Steps 1 and 2: normalise the corners and find the two vanishing points.
    let (n2, n3) = vanishing_points(&input.corners, w, h, long);

    // Step 3: the focal length.
    let f_known = known_focal(&input.focal, w, h, long);
    let f_estimated = estimated_focal(n2, n3);
    let (f, focal_source) = choose_focal(f_known, f_estimated, &input.focal);

    // Step 4: the ratio.
    let (mut ratio, mut source) = ratio_from_vanishing(n2, n3, f, focal_source);
    if !(ratio.is_finite() && (RATIO_MIN..=RATIO_MAX).contains(&ratio))
        && let Some(side_ratio) = ratio_from_side_lengths(&input.corners)
    {
        ratio = side_ratio;
        source = RatioSource::SideLengths;
    }
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(GeomError::Degenerate("the page's ratio is not finite"));
    }

    // Step 5: snap to a standard paper.
    let mut paper = None;
    if input.snap
        && let Some(snapped) = snap_to_paper(ratio, &input.default_paper)
    {
        ratio = snapped.0;
        paper = Some(snapped.1);
    }

    // Step 6: the physical size.
    let page_size_mm = physical_size_mm(ratio, paper, &input.default_paper);

    // Step 7: the native pixel size. The ratio here is the snapped one: step 5 runs first.
    let native_size = native_size_px(&input.corners, ratio);

    Ok(ProportionsOutput {
        proportions: Proportions { ratio, source, paper, native_size },
        camera: Camera { width: w, height: h, f, source: focal_source },
        page_size_mm,
    })
}

/// Rejects a quadrilateral with three corners on a line (A12 needs a real one too).
fn check_quad(corners: &Quad, long: f64) -> Result<(), GeomError> {
    let p = corners.0;
    let eps = 1e-9 * long * long;
    for (i, j, k) in [(0, 1, 2), (1, 2, 3), (2, 3, 0), (3, 0, 1)] {
        let area = triangle_area(p[i], p[j], p[k]);
        if !area.is_finite() || area.abs() <= eps {
            return Err(GeomError::Degenerate("three corners lie on a line"));
        }
    }
    Ok(())
}

/// Twice the signed area of a triangle.
fn triangle_area(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// A11 steps 1 and 2: the corners in normalised photo coordinates and the two vanishing points.
///
/// `n2` is the vanishing point of the top and bottom edges, `n3` of the left and right ones.
fn vanishing_points(corners: &Quad, w: f64, h: f64, long: f64) -> ([f64; 3], [f64; 3]) {
    let m = |p: Point| [(p.x - w / 2.0) / long, (p.y - h / 2.0) / long, 1.0];
    // The order is a Z, not a walk round the page: m1 = TL, m2 = TR, m3 = BL, m4 = BR.
    let (m1, m2, m3, m4) = (m(corners.tl()), m(corners.tr()), m(corners.bl()), m(corners.br()));
    let k2 = dot3(cross3(m1, m4), m3) / dot3(cross3(m2, m4), m3);
    let k3 = dot3(cross3(m1, m4), m2) / dot3(cross3(m3, m4), m2);
    let n2 = [k2 * m2[0] - m1[0], k2 * m2[1] - m1[1], k2 * m2[2] - m1[2]];
    let n3 = [k3 * m3[0] - m1[0], k3 * m3[1] - m1[1], k3 * m3[2] - m1[2]];
    (n2, n3)
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A11 step 3, the known path: EXIF's 35 mm-equivalent focal length, read against the frame.
fn known_focal(focal: &FocalInput, w: f64, h: f64, long: f64) -> Option<f64> {
    let f35 = focal.f35_mm?;
    if !f35.is_finite() || f35 <= 0.0 || !has_frame_shape(w, h) {
        return None;
    }
    let diagonal = (w * w + h * h).sqrt();
    let f = f35 * diagonal / (FRAME_DIAGONAL_MM * long);
    (f.is_finite() && f > 0.0).then_some(f)
}

/// True when the photo has the shape of a full camera frame, either way round.
fn has_frame_shape(w: f64, h: f64) -> bool {
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return false;
    }
    let r = w / h;
    FULL_FRAME_SHAPES.iter().any(|&std| {
        let close = |value: f64| (value - std).abs() / std <= 0.01;
        close(r) || close(1.0 / r)
    })
}

/// A11 step 3, the estimated path: Zhang and He (2007), from the two vanishing points.
fn estimated_focal(n2: [f64; 3], n3: [f64; 3]) -> Option<f64> {
    if n2[2].abs() <= 1e-4 || n3[2].abs() <= 1e-4 {
        return None;
    }
    let f2 = -(n2[0] * n3[0] + n2[1] * n3[1]) / (n2[2] * n3[2]);
    if !f2.is_finite() || f2 <= 0.0 {
        return None;
    }
    let f = f2.sqrt();
    ((FOCAL_MIN..=FOCAL_MAX).contains(&f)).then_some(f)
}

/// A11 step 3: the known focal length wins; a digital zoom picks the closer of the two.
fn choose_focal(f_known: Option<f64>, f_estimated: Option<f64>, focal: &FocalInput) -> (f64, FocalSource) {
    let Some(known) = f_known else {
        return match f_estimated {
            Some(f) => (f, FocalSource::Estimated),
            None => (TYPICAL_FOCAL, FocalSource::Typical),
        };
    };
    let mut f = known;
    if let Some(zoom) = focal.digital_zoom
        && zoom.is_finite()
        && zoom > 1.0
    {
        let zoomed = known * zoom;
        if let Some(estimated) = f_estimated
            && (zoomed - estimated).abs() < (known - estimated).abs()
        {
            f = zoomed;
        }
    }
    (f, FocalSource::Known)
}

/// A11 step 4: the ratio from the two vanishing points and the focal length.
fn ratio_from_vanishing(n2: [f64; 3], n3: [f64; 3], f: f64, source: FocalSource) -> (f64, RatioSource) {
    let numerator = n2[0] * n2[0] + n2[1] * n2[1] + f * f * n2[2] * n2[2];
    let denominator = n3[0] * n3[0] + n3[1] * n3[1] + f * f * n3[2] * n3[2];
    if !denominator.is_finite() || denominator <= 0.0 {
        return (f64::NAN, ratio_source(source));
    }
    let ratio = (numerator / denominator).sqrt();
    (ratio, ratio_source(source))
}

fn ratio_source(source: FocalSource) -> RatioSource {
    match source {
        FocalSource::Known => RatioSource::KnownFocal,
        FocalSource::Estimated => RatioSource::EstimatedFocal,
        _ => RatioSource::TypicalFocal,
    }
}

/// A11 step 4's fallback: the ratio of the side lengths in the photo.
fn ratio_from_side_lengths(corners: &Quad) -> Option<f64> {
    let (tl, tr, br, bl) = (corners.tl(), corners.tr(), corners.br(), corners.bl());
    let across = tl.distance(tr) + bl.distance(br);
    let down = tl.distance(bl) + tr.distance(br);
    if !down.is_finite() || down <= 0.0 {
        return None;
    }
    let ratio = across / down;
    (ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

/// A11 step 5: the closest standard ratio within 3 %, keeping the page's orientation.
fn snap_to_paper(ratio: f64, default_paper: &Paper) -> Option<(f64, Paper)> {
    let rho = ratio.max(1.0 / ratio);
    let mut best: Option<(f64, f64, Paper)> = None;
    for (std_ratio, paper, _) in standard_papers(default_paper) {
        let distance = (rho - std_ratio).abs() / std_ratio;
        if distance > SNAP_TOLERANCE {
            continue;
        }
        let better = match best {
            Some((best_distance, _, _)) => distance < best_distance,
            None => true,
        };
        if better {
            best = Some((distance, std_ratio, paper));
        }
    }
    let (_, std_ratio, paper) = best?;
    Some((if ratio > 1.0 { std_ratio } else { 1.0 / std_ratio }, paper))
}

/// A11 step 6: the page's size in millimetres.
fn physical_size_mm(ratio: f64, paper: Option<Paper>, default_paper: &Paper) -> (f64, f64) {
    if let Some(paper) = paper {
        let (short, long) = paper_size_mm(&paper);
        return if ratio >= 1.0 { (long, short) } else { (short, long) };
    }
    // An unsnapped page fits inside the default paper, as large as possible with this ratio.
    let (short, long) = paper_size_mm(default_paper);
    let (wp, hp) = if ratio >= 1.0 { (long, short) } else { (short, long) };
    if ratio >= wp / hp { (wp, wp / ratio) } else { (ratio * hp, hp) }
}

/// A11 step 7: the native pixel size, capped at 40 megapixels and 32768 px a side.
fn native_size_px(corners: &Quad, ratio: f64) -> (u32, u32) {
    let (tl, tr, br, bl) = (corners.tl(), corners.tr(), corners.br(), corners.bl());
    let w_hat = tl.distance(tr).max(bl.distance(br));
    let h_hat = tl.distance(bl).max(tr.distance(br));
    if !h_hat.is_finite() || h_hat <= 0.0 || !ratio.is_finite() || ratio <= 0.0 {
        return (1, 1);
    }
    let (w, h) = if w_hat / h_hat >= ratio { (w_hat, w_hat / ratio) } else { (ratio * h_hat, h_hat) };
    let mut wn = w.round().max(1.0);
    let mut hn = h.round().max(1.0);
    let pixels = wn * hn;
    if pixels > MAX_PAGE_PIXELS as f64 {
        let k = (MAX_PAGE_PIXELS as f64 / pixels).sqrt();
        wn = (k * wn).round().max(1.0);
        hn = (k * hn).round().max(1.0);
        // Rounding to the nearest pixel can leave the pair a hair over the cap.
        while wn * hn > MAX_PAGE_PIXELS as f64 && wn + hn > 2.0 {
            if wn >= hn {
                wn -= 1.0;
            } else {
                hn -= 1.0;
            }
        }
    }
    let side = wn.max(hn);
    if side > MAX_SIDE as f64 {
        let k = MAX_SIDE as f64 / side;
        wn = (k * wn).round().max(1.0);
        hn = (k * hn).round().max(1.0);
    }
    (wn as u32, hn as u32)
}

/// The standard papers, with their long ÷ short ratio and their size in millimetres.
fn standard_papers(default_paper: &Paper) -> Vec<(f64, Paper, (f64, f64))> {
    let a_number = match default_paper {
        Paper::IsoA(n) if *n <= 8 => *n,
        _ => 4,
    };
    let (a_ratio, a_size) = iso_a(a_number);
    vec![
        (a_ratio, Paper::IsoA(a_number), a_size),
        (279.4 / 215.9, Paper::UsLetter, (215.9, 279.4)),
        (355.6 / 215.9, Paper::UsLegal, (215.9, 355.6)),
        (85.60 / 53.98, Paper::Id1, (53.98, 85.60)),
        (88.9 / 50.8, Paper::BusinessCard, (50.8, 88.9)),
    ]
}

/// A paper's short and long sides in millimetres.
fn paper_size_mm(paper: &Paper) -> (f64, f64) {
    match paper {
        Paper::IsoA(n) => iso_a(*n).1,
        Paper::UsLetter => (215.9, 279.4),
        Paper::UsLegal => (215.9, 355.6),
        Paper::Id1 => (53.98, 85.60),
        Paper::BusinessCard => (50.8, 88.9),
    }
}

/// The ISO A series' ratio and the size of the number `n`, short side first.
fn iso_a(n: u8) -> (f64, (f64, f64)) {
    let size = match n {
        0 => (841.0, 1189.0),
        1 => (594.0, 841.0),
        2 => (420.0, 594.0),
        3 => (297.0, 420.0),
        4 => (210.0, 297.0),
        5 => (148.0, 210.0),
        6 => (105.0, 148.0),
        7 => (74.0, 105.0),
        8 => (52.0, 74.0),
        _ => (210.0, 297.0),
    };
    (2.0f64.sqrt(), size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::Step;
    use scancraft_core::determinism::{Xorshift64Star, seed_for};

    /// The photo the synthetic scenes are rendered into.
    const PHOTO: (u32, u32) = (4000, 3000);
    /// The distance from the camera to the page's centre.
    const DISTANCE: f64 = 3.0;

    fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
        let mut out = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    out[i][j] += a[i][k] * b[k][j];
                }
            }
        }
        out
    }

    fn mul_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0; 3];
        for i in 0..3 {
            for k in 0..3 {
                out[i] += m[i][k] * v[k];
            }
        }
        out
    }

    /// Rotation about x, then y, then z (roll), row-major.
    fn rotation(ax: f64, ay: f64, az: f64) -> [[f64; 3]; 3] {
        let (cx, sx) = (ax.cos(), ax.sin());
        let (cy, sy) = (ay.cos(), ay.sin());
        let (cz, sz) = (az.cos(), az.sin());
        let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
        let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
        mat_mul(&rz, &mat_mul(&rx, &ry))
    }

    /// The corners of a page of the given width ÷ height, seen by a camera with the given focal
    /// length, tilt and roll. The page's centre is straight ahead at the distance `DISTANCE`.
    fn scene_corners(ratio: f64, f: f64, tilt: f64, roll: f64) -> Quad {
        let r = rotation(tilt, tilt * 0.6, roll);
        let (w, h) = (f64::from(PHOTO.0), f64::from(PHOTO.1));
        let long = w.max(h);
        let corners = [(0.0, 0.0), (ratio, 0.0), (ratio, 1.0), (0.0, 1.0)];
        Quad(corners.map(|(px, py)| {
            let local = [px - ratio / 2.0, py - 0.5, 0.0];
            let world = mul_vec(&r, local);
            let q = [world[0], world[1], world[2] + DISTANCE];
            Point::new(w / 2.0 + f * long * q[0] / q[2], h / 2.0 + f * long * q[1] / q[2])
        }))
    }

    /// The 35 mm-equivalent focal length of a camera whose focal length is `f` of the long side.
    fn f35_for(f: f64) -> f64 {
        let (w, h) = (f64::from(PHOTO.0), f64::from(PHOTO.1));
        f * FRAME_DIAGONAL_MM * w.max(h) / (w * w + h * h).sqrt()
    }

    /// Moves every corner by up to `sigma` pixels, from the only permitted random source (§7.6).
    fn noisy(corners: Quad, seed: u64, sigma: f64) -> Quad {
        let mut rng = Xorshift64Star::new(seed);
        Quad(corners.0.map(|p| {
            let dx = (rng.next_f64() - 0.5) * 2.0 * sigma;
            let dy = (rng.next_f64() - 0.5) * 2.0 * sigma;
            Point::new(p.x + dx, p.y + dy)
        }))
    }

    fn input(corners: Quad, focal: FocalInput, snap: bool) -> ProportionsInput {
        ProportionsInput { corners, photo: PHOTO, focal, snap, default_paper: Paper::IsoA(4) }
    }

    /// The `q` quantile of a set of values, by sorting them.
    fn percentile(values: &mut [f64], q: f64) -> f64 {
        values.sort_by(|a, b| a.total_cmp(b));
        if values.is_empty() {
            return f64::NAN;
        }
        let index = ((values.len() as f64 - 1.0) * q).round() as usize;
        values[index.min(values.len() - 1)]
    }

    /// The synthetic scenes of A11's tests: four ratios, four focal lengths, five tilts, three rolls.
    fn scenes() -> Vec<(f64, f64, f64, f64)> {
        let mut out = Vec::new();
        for ratio in [std::f64::consts::SQRT_2, 1.2941, 1.0, 2.5] {
            for f in [0.6, 0.75, 0.9, 1.2] {
                for tilt in [10.0f64, 20.0, 30.0, 40.0, 50.0] {
                    for roll in [0.0f64, 17.0, -43.0] {
                        out.push((ratio, f, tilt.to_radians(), roll.to_radians()));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn estimated_focal_recovers_the_ratio() {
        let mut errors = Vec::new();
        for (page, (ratio, f, tilt, roll)) in scenes().into_iter().enumerate() {
            let corners = scene_corners(ratio, f, tilt, roll);
            let corners = noisy(corners, seed_for(page as u64, Step::A11, 0), 0.5);
            let out = true_proportions(&input(corners, FocalInput::default(), false)).unwrap();
            assert!(out.proportions.ratio.is_finite());
            errors.push(((out.proportions.ratio - ratio) / ratio).abs());
        }
        let median = percentile(&mut errors.clone(), 0.5);
        let p95 = percentile(&mut errors.clone(), 0.95);
        println!("estimated focal: median {median:.5} P95 {p95:.5} over {} scenes", errors.len());
        assert!(median <= 0.01, "median error {median}");
        assert!(p95 <= 0.03, "P95 error {p95}");
    }

    #[test]
    fn known_focal_recovers_the_ratio() {
        let mut errors = Vec::new();
        for (page, (ratio, f, tilt, roll)) in scenes().into_iter().enumerate() {
            let corners = scene_corners(ratio, f, tilt, roll);
            let corners = noisy(corners, seed_for(page as u64, Step::A11, 1), 0.5);
            let focal = FocalInput { f35_mm: Some(f35_for(f)), digital_zoom: None };
            let out = true_proportions(&input(corners, focal, false)).unwrap();
            assert_eq!(out.camera.source, FocalSource::Known);
            assert_eq!(out.proportions.source, RatioSource::KnownFocal);
            errors.push(((out.proportions.ratio - ratio) / ratio).abs());
        }
        let median = percentile(&mut errors.clone(), 0.5);
        let p95 = percentile(&mut errors.clone(), 0.95);
        println!("known focal: median {median:.5} P95 {p95:.5} over {} scenes", errors.len());
        assert!(median <= 0.005, "median error {median}");
        assert!(p95 <= 0.015, "P95 error {p95}");
    }

    #[test]
    fn degenerate_inputs_give_a_finite_ratio() {
        // A head-on rectangle: both pairs of sides are parallel, so the vanishing points are at
        // infinity and the focal length cannot be estimated.
        let rectangle = Quad([Point::new(1200.0, 300.0), Point::new(2800.0, 300.0), Point::new(2800.0, 2700.0), Point::new(1200.0, 2700.0)]);
        let out = true_proportions(&input(rectangle, FocalInput::default(), false)).unwrap();
        assert!((out.proportions.ratio - 1600.0 / 2400.0).abs() < 1e-9);
        assert_eq!(out.proportions.source, RatioSource::TypicalFocal);

        // A trapezoid: only the top and bottom sides are parallel.
        let trapezoid = Quad([Point::new(1200.0, 400.0), Point::new(2800.0, 400.0), Point::new(3000.0, 2600.0), Point::new(1000.0, 2600.0)]);
        let out = true_proportions(&input(trapezoid, FocalInput::default(), false)).unwrap();
        assert!(out.proportions.ratio.is_finite());
        assert!(out.proportions.ratio > 0.0);
        assert_eq!(out.proportions.source, RatioSource::TypicalFocal);

        // A quadrilateral whose vanishing points give a negative f².
        let negative = Quad([Point::new(1000.0, 600.0), Point::new(1500.0, 700.0), Point::new(2000.0, 500.0), Point::new(2400.0, 900.0)]);
        let out = true_proportions(&input(negative, FocalInput::default(), false)).unwrap();
        assert!(out.proportions.ratio.is_finite());
        assert!(out.proportions.ratio > 0.0);
        assert_eq!(out.proportions.source, RatioSource::TypicalFocal);
    }

    #[test]
    fn scaling_the_photo_leaves_the_ratio_unchanged() {
        let corners = scene_corners(std::f64::consts::SQRT_2, 0.8, 0.35, 0.2);
        let before = true_proportions(&input(corners, FocalInput::default(), false)).unwrap();
        let k = 2.5;
        let mut scaled = input(corners.scaled(k), FocalInput::default(), false);
        scaled.photo = ((PHOTO.0 as f64 * k) as u32, (PHOTO.1 as f64 * k) as u32);
        let after = true_proportions(&scaled).unwrap();
        assert!((before.proportions.ratio - after.proportions.ratio).abs() < 1e-9);
    }

    #[test]
    fn turning_the_photo_by_ninety_degrees_inverts_the_ratio() {
        let corners = scene_corners(std::f64::consts::SQRT_2, 0.8, 0.35, 0.2);
        let before = true_proportions(&input(corners, FocalInput::default(), false)).unwrap();
        let (w, h) = (f64::from(PHOTO.0), f64::from(PHOTO.1));
        // The photo's content turned a quarter turn counter-clockwise, about the new photo's centre.
        let turned = corners.0.map(|p| {
            let x = p.x - w / 2.0;
            let y = p.y - h / 2.0;
            Point::new(h / 2.0 + y, w / 2.0 - x)
        });
        let mut turned_input = input(Quad::canonical(turned), FocalInput::default(), false);
        turned_input.photo = (PHOTO.1, PHOTO.0);
        let after = true_proportions(&turned_input).unwrap();
        assert!(
            (after.proportions.ratio - 1.0 / before.proportions.ratio).abs() < 1e-9,
            "{} against {}",
            after.proportions.ratio,
            1.0 / before.proportions.ratio
        );
    }

    #[test]
    fn ratios_within_three_percent_snap_to_a_paper() {
        let near = scene_corners(1.40, 0.8, 0.0, 0.0);
        let out = true_proportions(&input(near, FocalInput::default(), true)).unwrap();
        assert_eq!(out.proportions.paper, Some(Paper::IsoA(4)));
        assert!((out.proportions.ratio - 2.0f64.sqrt()).abs() < 1e-12);

        let far = scene_corners(1.36, 0.8, 0.0, 0.0);
        let out = true_proportions(&input(far, FocalInput::default(), true)).unwrap();
        assert_eq!(out.proportions.paper, None);
        assert!((out.proportions.ratio - 1.36).abs() < 1e-9);

        // A portrait page keeps its orientation.
        let portrait = scene_corners(0.70, 0.8, 0.0, 0.0);
        let out = true_proportions(&input(portrait, FocalInput::default(), true)).unwrap();
        assert_eq!(out.proportions.paper, Some(Paper::IsoA(4)));
        assert!((out.proportions.ratio - 1.0 / 2.0f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn a_page_snapped_to_a4_has_the_a4_size() {
        let corners = scene_corners(std::f64::consts::FRAC_1_SQRT_2, 0.8, 0.3, 0.1);
        let out = true_proportions(&input(corners, FocalInput::default(), true)).unwrap();
        assert_eq!(out.proportions.paper, Some(Paper::IsoA(4)));
        assert_eq!(out.page_size_mm, (210.0, 297.0));
        let points = |mm: f64| mm / 25.4 * 72.0;
        assert!((points(out.page_size_mm.0) - 595.28).abs() < 0.01);
        assert!((points(out.page_size_mm.1) - 841.89).abs() < 0.01);
    }

    #[test]
    fn the_native_size_is_capped_at_forty_megapixels() {
        let huge = Quad([Point::new(0.0, 0.0), Point::new(30000.0, 0.0), Point::new(30000.0, 30000.0), Point::new(0.0, 30000.0)]);
        let out = true_proportions(&input(huge, FocalInput::default(), false)).unwrap();
        let (w, h) = out.proportions.native_size;
        assert!(u64::from(w) * u64::from(h) <= MAX_PAGE_PIXELS, "{w} × {h}");
        assert!(w <= MAX_SIDE && h <= MAX_SIDE);
    }

    #[test]
    fn a_degenerate_quad_is_an_error_not_a_panic() {
        let collinear = Quad([Point::new(100.0, 100.0), Point::new(200.0, 200.0), Point::new(300.0, 300.0), Point::new(400.0, 250.0)]);
        assert!(true_proportions(&input(collinear, FocalInput::default(), false)).is_err());

        let broken = Quad([Point::new(f64::NAN, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)]);
        assert!(true_proportions(&input(broken, FocalInput::default(), false)).is_err());

        let mut empty = input(
            Quad([Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)]),
            FocalInput::default(),
            false,
        );
        empty.photo = (0, 0);
        assert!(true_proportions(&empty).is_err());
    }
}
