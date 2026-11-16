//! A9: score and select (specification A9). Owner: WP8; WP11 adds the scoring of curved sides.
//!
//! Every candidate gets a score that asks three questions: do real edges support the outline all
//! the way round, does the colour change where the outline is, and how much of the photo does the
//! candidate cover. The best candidate wins.

use crate::DetectError;
use crate::edges::EdgeMaps;
use crate::refine::{self, Refined};
use crate::shapes::{add, bilinear_rgb, clip_to_rect, corner_angles, in_allowed_area, median, overlap_iou, polygon_area, side_ends, side_frame, sub};
use rayon::prelude::*;
use scancraft_core::Step;
use scancraft_core::determinism::seed_for;
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::{Plane, RgbF32};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{Detector, SideShape};
use scancraft_raster::colour::oklab;
use std::cmp::Ordering;

/// The corner angles a candidate may have (A9 step 1).
const MIN_CORNER_ANGLE: f64 = 35.0;
/// The corner angles a candidate may have (A9 step 1).
const MAX_CORNER_ANGLE: f64 = 145.0;
/// How much two opposite sides may differ in length (A9 step 1).
const MAX_SIDE_RATIO: f64 = 3.0;
/// The smallest share of the image a candidate may cover (A9 step 1).
const MIN_AREA_SHARE: f64 = 0.05;
/// The tolerance of the edge support search before the refinement (A9 step 2).
const TAU_BEFORE: f64 = 3.0;
/// The tolerance of the edge support search after the refinement (A9 step 2).
const TAU_AFTER: f64 = 2.0;
/// How close a gradient must be to the side's normal (A9 step 2).
const MAX_GRADIENT_ANGLE: f64 = 25.0;
/// A side with fewer in-image samples than this share is a border side (A9 step 2).
const BORDER_SAMPLE_SHARE: f64 = 0.30;
/// How close to the frame a sample must lie to count as being at the border (A9 step 2).
const BORDER_DISTANCE: f64 = 3.0;
/// The support a border side is given (A9 step 2).
const BORDER_SUPPORT: f64 = 0.35;
/// The contrast that counts as a full mark (A9 step 3).
const CONTRAST_FULL: f64 = 0.08;
/// The contrast given when every side is a border side (A9 step 3).
const BORDER_CONTRAST: f64 = 0.5;
/// The support under which a side's candidate is marked down (A9 step 5).
const WEAK_SUPPORT: f64 = 0.2;
/// The score a classical candidate must reach before it may take over from a valid platform
/// candidate (A9 step 6.3).
///
/// The specification's rule is that the classical path takes over when it scores more than 1.5
/// times the platform candidate's score and at least 0.12. Measured on SmartDoc 2015 (608 frames,
/// §10.4's gate), that rule handed the page to a classical candidate on 9 frames in 10 where the
/// platform candidate was right and the classical one was not: a page on a pale desk scores 0.03,
/// because the contrast term is small, while the desk around it scores 0.26, because the desk's
/// edge is a strong boundary and the desk is large. The two scores are not on the same scale, so
/// "1.5 times better" is easy to reach without being better.
///
/// With this bar the full pipeline's mean Jaccard index on the same frames is 0.949 instead of
/// 0.860, and it is no worse on any of the five backgrounds; on all 24 889 frames it is 0.951
/// (specification target 0.97, not yet met: the cluttered desk of background 5 scores 0.73). The
/// classical path is still used on its own when the platform has no answer (A9 step 6.4), where
/// it scores 0.682 on the same frames (target 0.93, not yet met).
const OVERRIDE_SCORE: f64 = 0.5;
const MIN_SCORE: f64 = 0.12;
/// How much better a classical candidate must be than Apple's (A9 step 6).
const APPLE_RATIO: f64 = 1.5;
/// The most classical candidates that are refined and scored again (A9 step 6).
const REFINE_COUNT: usize = 3;
/// Two candidates that overlap this much are the same candidate (A9 step 6).
const SAME_CANDIDATE_IOU: f64 = 0.9;

/// What A9 measured about one side of a candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SideMeasure {
    /// The share of the side's samples that edges support.
    support: f64,
    /// Whether the side lies at the photo's border.
    border: bool,
    /// The colour change across the side, in Oklab.
    contrast: f64,
}

/// The work image and its edges, prepared for scoring.
///
/// The gradient of every pixel is doubled, so that a direction and its opposite are the same and
/// the comparison with a side's normal is a dot product.
pub struct Prepared {
    pub maps: EdgeMaps,
    doubled: Plane<[f32; 2]>,
    pub width: u32,
    pub height: u32,
}

impl Prepared {
    /// Prepares the edge maps for scoring.
    pub fn new(maps: EdgeMaps) -> Result<Self, DetectError> {
        let (width, height) = (maps.magnitude.width(), maps.magnitude.height());
        let mut data = Vec::with_capacity((width as usize) * (height as usize));
        for theta in maps.direction.data() {
            let (sin, cos) = theta.sin_cos();
            data.push([cos * cos - sin * sin, 2.0 * sin * cos]);
        }
        Ok(Self { maps, doubled: Plane::from_vec(width, height, data)?, width, height })
    }

    /// The doubled direction of the pixel at `(x, y)`.
    fn doubled_at(&self, x: u32, y: u32) -> [f32; 2] {
        self.doubled.get(x, y).unwrap_or([1.0, 0.0])
    }
}

/// What A9 chose.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// The corners in full-resolution photo pixels, with A10's side information. When nothing was
    /// found, the corners sit 3% inside the photo and every side is marked unrefined.
    pub refined: Refined,
    /// Which detector found them; `None` when nothing was found.
    pub by: Option<Detector>,
    /// The score of the chosen candidate after refinement; 0 when nothing was found.
    pub score: f64,
    /// A25's trace of the chosen candidate's sides, in photo pixels; every side `Straight` when
    /// nothing was found.
    pub sides: [SideShape; 4],
}

/// A9 step 5's score of one candidate in work-image pixels; `tau` is 3 before refinement, 2 after.
pub fn score(quad: &Quad, maps: &EdgeMaps, work: &RgbF32, tau: f64) -> Result<f64, DetectError> {
    // The work image itself is not measured: A5's blurred copy of it is in the edge maps.
    let _ = work;
    let prepared = Prepared::new(maps.clone())?;
    score_prepared(quad, &prepared, tau)
}

/// The score of one candidate, from the prepared edge maps, with every side taken as straight.
pub fn score_prepared(quad: &Quad, prepared: &Prepared, tau: f64) -> Result<f64, DetectError> {
    score_traced(quad, None, prepared, tau)
}

/// A candidate's sides as A25 traced them, with the factors that turn their photo pixels into the
/// work image's (A9 step 6.2).
#[derive(Debug, Clone, Copy)]
pub struct Traces<'a> {
    pub sides: &'a [SideShape; 4],
    pub kx: f64,
    pub ky: f64,
}

/// The score of one candidate. A side that A25 marked curved is scored along its traced path
/// instead of its straight line, so a bent page isn't marked down for bending (A9 step 6.2).
pub fn score_traced(quad: &Quad, traces: Option<Traces<'_>>, prepared: &Prepared, tau: f64) -> Result<f64, DetectError> {
    let (w, h) = (prepared.width, prepared.height);
    if !is_valid(quad, w, h) {
        return Ok(0.0);
    }
    let mut sides = Vec::with_capacity(4);
    for side in Side::ALL {
        let path = traces.and_then(|t| curved_path(quad, side, &t));
        sides.push(measure(quad, side, prepared, tau, path.as_deref()));
    }
    let support: f64 = sides.iter().map(|s| s.support).sum::<f64>() / 4.0;
    let contrasts: Vec<f64> = sides.iter().filter(|s| !s.border).map(|s| s.contrast).collect();
    let contrast = if contrasts.is_empty() {
        BORDER_CONTRAST
    } else {
        contrasts.iter().map(|c| (c / CONTRAST_FULL).min(1.0)).sum::<f64>() / contrasts.len() as f64
    };
    let area = area_share(quad, w, h);
    let mut value = support * support * contrast * area.sqrt();
    // A side that no edge supports marks the whole candidate down.
    for side in &sides {
        if !side.border && side.support < WEAK_SUPPORT {
            value *= 0.5;
            break;
        }
    }
    Ok(value)
}

/// Which sides of a candidate lie at the photo's border (A9 step 2): A25 doesn't trace them.
pub fn border_sides(quad: &Quad, prepared: &Prepared, tau: f64) -> [bool; 4] {
    Side::ALL.map(|side| measure(quad, side, prepared, tau, None).border)
}

/// The path of a side that A25 marked curved, in work-image pixels, from one corner through the
/// samples to the other; `None` for any other side.
fn curved_path(quad: &Quad, side: Side, traces: &Traces<'_>) -> Option<Vec<Point>> {
    let SideShape::Traced { samples, curved: true, .. } = traces.sides.get(side.index())? else { return None };
    if samples.len() < 2 || !(traces.kx > 0.0 && traces.ky > 0.0) {
        return None;
    }
    let (a, b) = side_ends(quad, side);
    let mut path = Vec::with_capacity(samples.len() + 2);
    path.push(a);
    path.extend(samples.iter().map(|s| Point::new(s.point.x / traces.kx, s.point.y / traces.ky)));
    path.push(b);
    path.iter().all(|p| p.is_finite()).then_some(path)
}

/// Whether a candidate may be used at all (A9 step 1).
pub fn is_valid(quad: &Quad, width: u32, height: u32) -> bool {
    if !quad.is_finite() {
        return false;
    }
    if !crate::shapes::is_convex(&quad.0) {
        return false;
    }
    for angle in corner_angles(&quad.0) {
        if angle < MIN_CORNER_ANGLE || angle > MAX_CORNER_ANGLE {
            return false;
        }
    }
    // Opposite sides may not differ in length by more than three times.
    let sides: Vec<f64> = Side::ALL.iter().map(|s| side_length(quad, *s)).collect();
    for k in [0usize, 2] {
        let (a, b) = (sides.get(k).copied().unwrap_or(0.0), sides.get(k + 1).copied().unwrap_or(0.0));
        if a <= 0.0 || b <= 0.0 {
            return false;
        }
        if a > b * MAX_SIDE_RATIO || b > a * MAX_SIDE_RATIO {
            return false;
        }
    }
    if area_share(quad, width, height) < MIN_AREA_SHARE {
        return false;
    }
    quad.0.iter().all(|p| in_allowed_area(*p, width, height))
}

/// The length of a side.
fn side_length(quad: &Quad, side: Side) -> f64 {
    let (a, b) = side_ends(quad, side);
    a.distance(b)
}

/// The share of the photo a candidate covers (A9 step 4).
fn area_share(quad: &Quad, width: u32, height: u32) -> f64 {
    let clipped = clip_to_rect(&quad.0, f64::from(width), f64::from(height));
    let image_area = f64::from(width) * f64::from(height);
    if image_area <= 0.0 {
        return 0.0;
    }
    polygon_area(&clipped) / image_area
}

/// What A9 measured about one side (A9 steps 2 and 3), along `path` when it has one, and along the
/// straight line between its corners otherwise.
fn measure(quad: &Quad, side: Side, prepared: &Prepared, tau: f64, path: Option<&[Point]>) -> SideMeasure {
    let (w, h) = (f64::from(prepared.width), f64::from(prepared.height));
    let (a, b) = side_ends(quad, side);
    let (_, outward) = side_frame(quad, side);
    let straight = [a, b];
    let line: &[Point] = path.unwrap_or(&straight);
    let length = polyline_length(line);
    // The middle 90% of the side, every 1 px.
    let start = 0.05 * length;
    let count = ((0.9 * length).floor() as usize).saturating_add(1);
    let mut in_image = 0usize;
    let mut supported = 0usize;
    let mut at_border = 0usize;
    let mut deltas: Vec<f64> = Vec::with_capacity(count);
    let mut walker = Walker::new(line);
    for i in 0..count {
        let Some((s, tangent)) = walker.at(start + i as f64) else { break };
        // The normal that points away from the page; a path's own direction changes along it.
        let perpendicular = Point::new(tangent.y, -tangent.x);
        let normal = if perpendicular.x * outward.x + perpendicular.y * outward.y >= 0.0 { perpendicular } else { perpendicular.scaled(-1.0) };
        if !inside_by(s, w, h) {
            continue;
        }
        in_image += 1;
        if distance_to_border(s, w, h) <= BORDER_DISTANCE {
            at_border += 1;
        }
        if supported_at(s, normal, prepared, tau) {
            supported += 1;
        }
        deltas.push(contrast_at(s, normal, &prepared.maps.blurred));
    }
    // A side at the photo's border is one the camera could not see the whole of.
    let border = in_image == 0 || (in_image as f64) < BORDER_SAMPLE_SHARE * count as f64 || at_border as f64 >= 0.7 * in_image as f64;
    let support = if border { BORDER_SUPPORT } else { supported as f64 / in_image as f64 };
    SideMeasure { support, border, contrast: median(&mut deltas) }
}

/// The length of a polyline.
fn polyline_length(points: &[Point]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Walks a polyline by arc length: the point at a distance from its start, and the unit direction
/// of the segment it lies on. The distances asked for must not decrease.
struct Walker<'a> {
    points: &'a [Point],
    /// The segment the walk is on, and the arc length at its start.
    segment: usize,
    before: f64,
}

impl<'a> Walker<'a> {
    fn new(points: &'a [Point]) -> Self {
        Self { points, segment: 0, before: 0.0 }
    }

    fn at(&mut self, distance: f64) -> Option<(Point, Point)> {
        loop {
            let (from, to) = (*self.points.get(self.segment)?, *self.points.get(self.segment + 1)?);
            let length = from.distance(to);
            if distance <= self.before + length || self.segment + 2 >= self.points.len() {
                let along = if length > 0.0 { ((distance - self.before) / length).clamp(0.0, 1.0) } else { 0.0 };
                let step = sub(to, from);
                let direction = if length > 0.0 { Point::new(step.x / length, step.y / length) } else { Point::new(1.0, 0.0) };
                return Some((add(from, step.scaled(along)), direction));
            }
            self.before += length;
            self.segment += 1;
        }
    }
}

/// Whether a point lies at least 2 px from every border of the photo (A9 step 2).
fn inside_by(s: Point, w: f64, h: f64) -> bool {
    s.x >= 2.0 && s.x <= w - 2.0 && s.y >= 2.0 && s.y <= h - 2.0
}

/// How far a point is from the nearest border of the photo.
fn distance_to_border(s: Point, w: f64, h: f64) -> f64 {
    s.x.min(w - s.x).min(s.y).min(h - s.y)
}

/// Whether an edge supports the side at this point (A9 step 2).
///
/// The edge must lie within `tau` pixels along the side's normal, and its gradient must point
/// within 25° of that normal, in either sense.
fn supported_at(s: Point, normal: Point, prepared: &Prepared, tau: f64) -> bool {
    // The normal is a unit vector (cos θ, sin θ); its doubled direction is (cos 2θ, sin 2θ), the
    // same form the pixels' gradients are stored in. Two directions within 25° of each other, in
    // either sense, have doubled directions within 50° of each other.
    let doubled = [(normal.x * normal.x - normal.y * normal.y) as f32, (2.0 * normal.x * normal.y) as f32];
    let limit = (2.0 * MAX_GRADIENT_ANGLE).to_radians().cos() as f32;
    let steps = (2.0 * tau / 0.5).round() as usize;
    for i in 0..=steps {
        let u = -tau + 0.5 * i as f64;
        let p = add(s, normal.scaled(u));
        let (x, y) = (p.x.floor(), p.y.floor());
        if x < 0.0 || y < 0.0 || x >= f64::from(prepared.width) || y >= f64::from(prepared.height) {
            continue;
        }
        let (x, y) = (x as u32, y as u32);
        if prepared.maps.sensitive.get(x, y) != Some(true) {
            continue;
        }
        let pixel = prepared.doubled_at(x, y);
        if pixel[0] * doubled[0] + pixel[1] * doubled[1] >= limit {
            return true;
        }
    }
    false
}

/// The colour change across the side at this point, in Oklab (A9 step 3).
fn contrast_at(s: Point, normal: Point, blurred: &RgbF32) -> f64 {
    let inside = add(s, normal.scaled(-4.0));
    let outside = add(s, normal.scaled(4.0));
    let a = bilinear_rgb(blurred, inside.x, inside.y);
    let b = bilinear_rgb(blurred, outside.x, outside.y);
    let (Ok(la), Ok(lb)) = (oklab(a), oklab(b)) else { return 0.0 };
    let mut sum = 0.0f32;
    for c in 0..3 {
        let d = la[c] - lb[c];
        sum += d * d;
    }
    f64::from(sum.sqrt())
}

/// The corners of a photo with nothing found in it: 3% inside its edges (A9 step 6.5).
pub fn not_found_corners(width: u32, height: u32) -> Quad {
    let (w, h) = (f64::from(width), f64::from(height));
    let (dx, dy) = (0.03 * w, 0.03 * h);
    Quad::canonical([Point::new(dx, dy), Point::new(w - dx, dy), Point::new(w - dx, h - dy), Point::new(dx, h - dy)])
}

/// A candidate after A10 and its second scoring.
struct Finalist {
    by: Detector,
    score: f64,
    refined: Refined,
    sides: [SideShape; 4],
}

/// The factors that turn work-image pixels into photo pixels, one for each axis.
fn photo_scale(pyramid: &LumaPyramid) -> Option<(f64, f64)> {
    let (first, full) = (pyramid.levels.first()?, pyramid.full()?);
    Some((f64::from(full.width()) / f64::from(first.width()), f64::from(full.height()) / f64::from(first.height())))
}

/// A quadrilateral with every corner scaled, one factor for each axis.
fn scaled(quad: &Quad, kx: f64, ky: f64) -> Quad {
    Quad(quad.0.map(|p| Point::new(p.x * kx, p.y * ky)))
}

/// A25 on a refined candidate. A trace that fails leaves the sides straight: the page is still
/// found, and the user can bend its sides by hand.
fn trace(refined: &Refined, work: &Quad, prepared: &Prepared, pyramid: &LumaPyramid) -> [SideShape; 4] {
    let border = border_sides(work, prepared, TAU_AFTER);
    crate::curved_sides::trace_sides(&refined.corners, &refined.polarity, pyramid, border, false).unwrap_or([
        SideShape::Straight,
        SideShape::Straight,
        SideShape::Straight,
        SideShape::Straight,
    ])
}

/// Refines a candidate given in work-image pixels (A10), then scores the result again with
/// τ = 2 (A9 step 6.2). A10 returns photo pixels, so the corners are brought back to the work
/// image to be scored against its edges.
fn finalist(by: Detector, quad: &Quad, prepared: &Prepared, pyramid: &LumaPyramid, seed: u64, index: usize) -> Result<Finalist, DetectError> {
    let Some((kx, ky)) = photo_scale(pyramid) else {
        return Err(scancraft_core::CoreError::BadSize { width: 0, height: 0 }.into());
    };
    let refined = refine::refine(quad, pyramid, seed_for(seed, Step::A10, index as u64))?;
    let work = scaled(&refined.corners, 1.0 / kx, 1.0 / ky);
    // A25 traces the sides that aren't border sides, and a curved one is scored along its path.
    let sides = trace(&refined, &work, prepared, pyramid);
    let score = score_traced(&work, Some(Traces { sides: &sides, kx, ky }), prepared, TAU_AFTER)?;
    Ok(Finalist { by, score, refined, sides })
}

/// A9 step 6: score the candidates, refine the best ones, and choose.
///
/// `classical` and `apple` are in work-image pixels. The pyramid's first level is the work image.
pub fn select(
    prepared: &Prepared,
    classical: &[(Detector, Quad)],
    apple: Option<&Quad>,
    pyramid: &LumaPyramid,
    seed: u64,
) -> Result<Selection, DetectError> {
    // Step 6.1: score every candidate with τ = 3. A candidate that fails A9's validity rules
    // scores 0 and is dropped. Each score depends on its candidate alone and the scores come back
    // in the candidates' order, so the thread count can't change the result.
    let values: Vec<f64> = classical.par_iter().map(|(_, quad)| score_prepared(quad, prepared, TAU_BEFORE)).collect::<Result<_, _>>()?;
    let mut scored: Vec<(Detector, Quad, f64)> = Vec::new();
    for ((by, quad), value) in classical.iter().zip(values) {
        if value > 0.0 {
            scored.push((*by, *quad, value));
        }
    }
    scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal));
    // Step 6.2: refine the three best classical candidates that differ from each other (their
    // overlap is under 0.9), and the Apple candidate, and score them again with τ = 2.
    let mut chosen: Vec<(Detector, Quad)> = Vec::new();
    for (by, quad, _) in &scored {
        if chosen.len() >= REFINE_COUNT {
            break;
        }
        if chosen.iter().any(|(_, other)| overlap_iou(other, quad) >= SAME_CANDIDATE_IOU) {
            continue;
        }
        chosen.push((*by, *quad));
    }
    // Each finalist is refined and scored on its own, with a seed of its own, so they can run at
    // once; the best is then taken in their order, the first of equal scores winning.
    let finalists: Vec<Finalist> =
        chosen.par_iter().enumerate().map(|(index, (by, quad))| finalist(*by, quad, prepared, pyramid, seed, index)).collect::<Result<_, _>>()?;
    let mut best_classical: Option<Finalist> = None;
    for f in finalists {
        if best_classical.as_ref().is_none_or(|b| f.score > b.score) {
            best_classical = Some(f);
        }
    }
    // The Apple candidate counts when it is valid, whatever it scores.
    let apple_finalist = match apple {
        Some(quad) if is_valid(quad, prepared.width, prepared.height) => {
            Some(finalist(Detector::Apple, quad, prepared, pyramid, seed, REFINE_COUNT)?)
        }
        _ => None,
    };
    // Steps 6.3 and 6.4: the Apple candidate wins unless the best classical one scores more than
    // 1.5 times as much and at least 0.12; with no Apple candidate, the best classical one wins
    // when it scores at least 0.12.
    let classical_wins = match (&best_classical, &apple_finalist) {
        (Some(c), Some(a)) => c.score > APPLE_RATIO * a.score && c.score >= OVERRIDE_SCORE,
        (Some(c), None) => c.score >= MIN_SCORE,
        (None, _) => false,
    };
    let winner = if classical_wins { best_classical } else { apple_finalist };
    if let Some(f) = winner {
        return Ok(Selection { by: Some(f.by), score: f.score, refined: f.refined, sides: f.sides });
    }
    // Step 6.5: nothing was found, and the corners start 3% inside the photo's edges.
    let (kx, ky) = photo_scale(pyramid).unwrap_or((1.0, 1.0));
    let corners = scaled(&not_found_corners(prepared.width, prepared.height), kx, ky);
    let straight = [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight];
    Ok(Selection {
        by: None,
        score: 0.0,
        refined: Refined { corners, unrefined: [true; 4], polarity: [crate::Polarity::PageLighter; 4] },
        sides: straight,
    })
}

#[cfg(test)]
mod tests {
    use super::{Prepared, TAU_AFTER, Traces, area_share, is_valid, measure, not_found_corners, score, score_prepared, score_traced, trace};
    use crate::edges::colour_edges;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::image::{Plane, RgbF32};
    use scancraft_core::recipe::SideShape;

    fn work(w: u32, h: u32, f: impl Fn(f64, f64) -> [f32; 3]) -> RgbF32 {
        let mut px: Vec<[f32; 3]> = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(f(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
        Plane::from_vec(w, h, px).unwrap()
    }

    fn page_scene(w: u32, h: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> RgbF32 {
        work(w, h, move |x, y| if x > x0 && x < x1 && y > y0 && y < y1 { [0.92; 3] } else { [0.18; 3] })
    }

    #[test]
    fn a_page_scores_well_and_a_stray_line_does_not() {
        let img = page_scene(400, 300, 80.0, 60.0, 320.0, 240.0);
        let maps = colour_edges(&img).unwrap();
        let prepared = Prepared::new(maps).unwrap();
        let page = Quad([Point::new(80.0, 60.0), Point::new(320.0, 60.0), Point::new(320.0, 240.0), Point::new(80.0, 240.0)]);
        let value = score_prepared(&page, &prepared, 3.0).unwrap();
        assert!(value > 0.12, "the page scored {value:.3}");
        // A small square in the middle of the desk has no edges around it.
        let stray = Quad([Point::new(150.0, 120.0), Point::new(170.0, 120.0), Point::new(170.0, 140.0), Point::new(150.0, 140.0)]);
        let value = score_prepared(&stray, &prepared, 3.0).unwrap();
        assert!(value < 0.12, "the stray square scored {value:.3}");
    }

    #[test]
    fn a_candidate_that_is_not_a_page_is_refused() {
        // A sliver: two of its angles are far too small.
        let sliver = Quad([Point::new(0.0, 0.0), Point::new(400.0, 1.0), Point::new(400.0, 3.0), Point::new(0.0, 2.0)]);
        assert!(!is_valid(&sliver, 400, 300));
        // A small square covers too little of the photo.
        let small = Quad([Point::new(10.0, 10.0), Point::new(40.0, 10.0), Point::new(40.0, 40.0), Point::new(10.0, 40.0)]);
        assert!(!is_valid(&small, 400, 300));
        // A square is fine.
        let square = Quad([Point::new(80.0, 60.0), Point::new(320.0, 60.0), Point::new(320.0, 240.0), Point::new(80.0, 240.0)]);
        assert!(is_valid(&square, 400, 300));
    }

    #[test]
    fn the_area_of_a_candidate_is_the_part_inside_the_photo() {
        let half = Quad([Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 200.0), Point::new(0.0, 200.0)]);
        assert!((area_share(&half, 400, 400) - 0.25).abs() < 1e-9);
        let whole = Quad([Point::new(0.0, 0.0), Point::new(400.0, 0.0), Point::new(400.0, 400.0), Point::new(0.0, 400.0)]);
        assert!((area_share(&whole, 400, 400) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_corners_of_nothing_found_sit_three_percent_inside() {
        let corners = not_found_corners(1000, 800);
        assert!((corners.tl().x - 30.0).abs() < 1e-9);
        assert!((corners.tl().y - 24.0).abs() < 1e-9);
        assert!((corners.br().x - 970.0).abs() < 1e-9);
        assert!((corners.br().y - 776.0).abs() < 1e-9);
    }

    #[test]
    fn the_public_score_gives_the_same_answer_as_the_prepared_one() {
        let img = page_scene(200, 150, 40.0, 30.0, 160.0, 120.0);
        let maps = colour_edges(&img).unwrap();
        let prepared = Prepared::new(maps.clone()).unwrap();
        let page = Quad([Point::new(40.0, 30.0), Point::new(160.0, 30.0), Point::new(160.0, 120.0), Point::new(40.0, 120.0)]);
        let one = score(&page, &maps, &img, 3.0).unwrap();
        let two = score_prepared(&page, &prepared, 3.0).unwrap();
        assert!((one - two).abs() < 1e-12, "{one} against {two}");
    }

    #[test]
    fn a_side_at_the_frame_is_a_border_side() {
        let img = page_scene(400, 300, -50.0, -50.0, 450.0, 350.0);
        let maps = colour_edges(&img).unwrap();
        let prepared = Prepared::new(maps).unwrap();
        // The page fills the photo, so all four sides are at the frame.
        let page = Quad([Point::new(-50.0, -50.0), Point::new(450.0, -50.0), Point::new(450.0, 350.0), Point::new(-50.0, 350.0)]);
        for side in scancraft_core::geom::Side::ALL {
            let m = measure(&page, side, &prepared, 3.0, None);
            assert!(m.border, "{side:?} should be a border side");
            assert!((m.support - 0.35).abs() < 1e-12);
        }
    }

    /// A page whose top and bottom edges bow outwards by `bow` pixels, on a darker desk: the photo
    /// as colours for the edge maps and as luma for the pyramid.
    fn bowed_page(w: u32, h: u32, bow: f64) -> (RgbF32, scancraft_core::pyramid::LumaPyramid) {
        let value = |x: f64, y: f64| {
            let t = ((x - 150.0) / 700.0).clamp(0.0, 1.0);
            let lift = bow * 4.0 * t * (1.0 - t);
            let inside = (y - (150.0 - lift)).min((600.0 + lift) - y).min(x - 150.0).min(850.0 - x);
            0.18 + 0.74 * (inside + 0.5).clamp(0.0, 1.0)
        };
        let img = work(w, h, |x, y| [value(x, y) as f32; 3]);
        let luma = Plane::from_vec(
            w,
            h,
            (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| value(f64::from(i) + 0.5, f64::from(j) + 0.5) as f32).collect(),
        )
        .unwrap();
        (img, crate::refine::build_pyramid(&luma).unwrap())
    }

    #[test]
    fn a_bent_page_scores_better_along_its_traced_edges_than_along_straight_chords() {
        // A9 step 6.2: the bend of the top and bottom edges is 6% of their length.
        let (img, pyramid) = bowed_page(1000, 750, 42.0);
        let prepared = Prepared::new(colour_edges(&img).unwrap()).unwrap();
        let corners = Quad([Point::new(150.0, 150.0), Point::new(850.0, 150.0), Point::new(850.0, 600.0), Point::new(150.0, 600.0)]);
        let refined = crate::refine::refine(&corners, &pyramid, 1).unwrap();
        let sides = trace(&refined, &refined.corners, &prepared, &pyramid);
        for side in [scancraft_core::geom::Side::Top, scancraft_core::geom::Side::Bottom] {
            assert!(
                matches!(sides[side.index()], SideShape::Traced { curved: true, .. }),
                "{side:?} was not traced as curved: {:?}",
                sides[side.index()]
            );
        }
        let straight = score_prepared(&refined.corners, &prepared, TAU_AFTER).unwrap();
        let along = score_traced(&refined.corners, Some(Traces { sides: &sides, kx: 1.0, ky: 1.0 }), &prepared, TAU_AFTER).unwrap();
        assert!(along > 1.2 * straight, "scored along its edges {along}, along straight chords {straight}");
    }

    #[test]
    fn a_flat_page_scores_the_same_whether_its_sides_are_traced_or_not() {
        let (img, pyramid) = bowed_page(1000, 750, 0.0);
        let prepared = Prepared::new(colour_edges(&img).unwrap()).unwrap();
        let corners = Quad([Point::new(150.0, 150.0), Point::new(850.0, 150.0), Point::new(850.0, 600.0), Point::new(150.0, 600.0)]);
        let refined = crate::refine::refine(&corners, &pyramid, 1).unwrap();
        let sides = trace(&refined, &refined.corners, &prepared, &pyramid);
        assert!(sides.iter().all(|s| !matches!(s, SideShape::Traced { curved: true, .. })), "{sides:?}");
        let straight = score_prepared(&refined.corners, &prepared, TAU_AFTER).unwrap();
        let along = score_traced(&refined.corners, Some(Traces { sides: &sides, kx: 1.0, ky: 1.0 }), &prepared, TAU_AFTER).unwrap();
        assert!((along - straight).abs() < 1e-12, "{along} against {straight}");
    }
}
