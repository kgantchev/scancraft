//! Tests for A27 and A28 on synthetic evidence, as in specification A27's prototype. The scenes
//! are made here from the sheet model directly, not through the code under test.

use super::*;
use crate::homography::Homography;
use crate::solver::rodrigues;
use scancraft_core::camera::FocalSource;
use scancraft_core::determinism::Xorshift64Star;

const W: f64 = 4000.0;
const H: f64 = 3000.0;

/// A true sheet and the camera that saw it.
struct Scene {
    sheet: Sheet,
    camera: Camera,
}

fn camera(f: f64) -> Camera {
    Camera { width: W, height: H, f, source: FocalSource::Typical }
}

/// A smooth profile whose height is `depth` of the chord at its middle, and `skew` moves the peak.
fn bump(profile: &Profile, depth: f64, skew: f64) -> [f64; 10] {
    let mut y = [0.0; 10];
    for (k, value) in y.iter_mut().enumerate() {
        let t = profile.knots()[k + 1];
        *value = depth * (std::f64::consts::PI * t.powf(1.0 + skew)).sin();
    }
    y
}

/// A scene: a sheet bent in `mode`, seen with focal length `f` from a tilted viewpoint.
fn scene(mode: BendMode, profile: [f64; 10], f: f64, tilt: [f64; 3]) -> Scene {
    let chord = if mode == BendMode::Across { 0.72 } else { 1.0 / 0.72 };
    let r = rodrigues(tilt);
    let rows = [r[0], r[1], r[2]];
    let rotation = [rows[0][0], rows[0][1], rows[0][2], rows[1][0], rows[1][1], rows[1][2], rows[2][0], rows[2][1], rows[2][2]];
    let mut sheet = Sheet { mode, rotation, translation: [0.0; 3], chord, profile, focal: f, focal_source: FocalSource::Typical, cost: 0.0 };
    // Put the page's middle on the optical axis, 1.7 ruling lengths away.
    let unrolled = unroll(&sheet, &camera(f)).unwrap();
    let middle = unrolled.camera_point(0.5, 0.5);
    let z = if mode == BendMode::Across { 1.7 } else { 1.7 * 0.72 };
    sheet.translation = [-(middle[0] - sheet.translation[0]), -(middle[1] - sheet.translation[1]), z - (middle[2] - sheet.translation[2])];
    Scene { sheet, camera: camera(f) }
}

/// The truth photo position of sheet point (t, b), by the model written out directly.
fn truth_point(scene: &Scene, t: f64, b: f64) -> Point {
    let profile = Profile::new();
    let s = &scene.sheet;
    let p = sheet_point(s.mode, t, b, s.chord, profile.value(&s.profile, t));
    let r = &s.rotation;
    let q = [
        r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + s.translation[0],
        r[3] * p[0] + r[4] * p[1] + r[5] * p[2] + s.translation[1],
        r[6] * p[0] + r[7] * p[1] + r[8] * p[2] + s.translation[2],
    ];
    scene.camera.project(q).unwrap()
}

fn gauss(rng: &mut Xorshift64Star) -> f64 {
    let (a, b) = (rng.next_f64().max(1e-12), rng.next_f64());
    (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
}

fn noisy(p: Point, sigma: f64, rng: &mut Xorshift64Star) -> Point {
    if sigma == 0.0 {
        return p;
    }
    Point::new(p.x + sigma * gauss(rng), p.y + sigma * gauss(rng))
}

/// How the evidence is spoiled.
#[derive(Clone, Copy, Default)]
struct Spoil {
    /// The share of text points moved by up to 25 px.
    outliers: f64,
    /// Hide half of one side.
    half_side: bool,
    /// No text at all.
    no_text: bool,
    /// No noise: the exact projections.
    exact: bool,
    /// Words shifted this many pixels along their lines, each by a random sign.
    shift: f64,
}

/// The evidence a detector would give for `scene` (the noise of A27's prototype).
fn evidence(scene: &Scene, direction: TextDirection, seed: u64, spoil: Spoil) -> SheetEvidence {
    evidence_with_truth(scene, direction, seed, spoil).0
}

/// The evidence, and the true sheet coordinates behind it: each line's shared one, and each
/// point's own.
fn evidence_with_truth(scene: &Scene, direction: TextDirection, seed: u64, spoil: Spoil) -> (SheetEvidence, Vec<f64>, Vec<f64>) {
    let (mut shared_truth, mut own_truth) = (Vec::new(), Vec::new());
    let mut rng = Xorshift64Star::new(seed);
    let mode = scene.sheet.mode;
    let noise = if spoil.exact { 0.0 } else { 1.0 };
    // 5% of text points are moved by up to 25 px unless the case asks for more.
    let outlier_rate = if spoil.exact { 0.0 } else { spoil.outliers.max(0.05) };
    let coordinates = corner_sheet_coordinates(mode);
    let corners = std::array::from_fn(|k| CornerEvidence {
        point: noisy(truth_point(scene, coordinates[k].0, coordinates[k].1), 2.0 * noise, &mut rng),
        sigma: 2.0,
    });
    // Text running across the page keeps b (bend across) or t (bend down) fixed along a line.
    let shared_is_t = line_is_t(mode, direction);
    let mut lines = Vec::new();
    if !spoil.no_text {
        // The prototype's text: 24 lines 0.035 apart, 65 points along each between 0.10 and 0.90.
        for i in 0..24 {
            let shared = 0.08 + 0.035 * f64::from(i);
            let mut points = Vec::new();
            for j in 0..65 {
                let along = 0.10 + 0.80 * f64::from(j) / 64.0;
                let (t, b) = if shared_is_t { (shared, along) } else { (along, shared) };
                let mut p = noisy(truth_point(scene, t, b), 2.0 * noise, &mut rng);
                if spoil.shift != 0.0 {
                    // Along the line: towards the next point along it.
                    let (nt, nb) = if shared_is_t { (t, along + 0.01) } else { (along + 0.01, b) };
                    let (here, next) = (truth_point(scene, t, b), truth_point(scene, nt, nb));
                    let length = here.distance(next).max(1e-9);
                    let sign = if rng.next_f64() < 0.5 { -1.0 } else { 1.0 };
                    p = Point::new(p.x + sign * spoil.shift * (next.x - here.x) / length, p.y + sign * spoil.shift * (next.y - here.y) / length);
                }
                if rng.next_f64() < outlier_rate {
                    p = Point::new(p.x + 25.0 * (2.0 * rng.next_f64() - 1.0), p.y + 25.0 * (2.0 * rng.next_f64() - 1.0));
                }
                own_truth.push(along);
                points.push(p);
            }
            shared_truth.push(shared);
            let sigma = vec![2.0; points.len()];
            lines.push(LineEvidence { points, sigma });
        }
    }
    let mut sides = Vec::new();
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        let (fixed_is_t, fixed) = side_placement(side, mode);
        // 64 samples from 0.03 to 0.97; half of one side hidden leaves the first 32.
        let count = if spoil.half_side && side == Side::Left { 32 } else { 64 };
        let mut points = Vec::new();
        for k in 0..count {
            let along = 0.03 + 0.94 * f64::from(k) / 63.0;
            let (t, b) = if fixed_is_t { (fixed, along) } else { (along, fixed) };
            own_truth.push(along);
            points.push(noisy(truth_point(scene, t, b), 1.5 * noise, &mut rng));
        }
        sides.push(SideEvidence { side, points, sigma: 1.5 });
    }
    (SheetEvidence { corners, lines, text_direction: direction, sides }, shared_truth, own_truth)
}

/// The flattening error of §10.3: the 95th percentile, over the page, of the distance between
/// where the fitted sheet and the true sheet put each flat-page point, as a share of the page's
/// diagonal in the photo.
fn flatten_error(truth: &Scene, fitted: &Sheet) -> f64 {
    let a = unroll(&truth.sheet, &truth.camera).unwrap();
    let b = unroll(fitted, &truth.camera).unwrap();
    let corners = a.corners().unwrap();
    let diagonal = corners.tl().distance(corners.br());
    let mut errors = Vec::new();
    for j in 0..21 {
        for i in 0..21 {
            let (u, v) = (f64::from(i) / 20.0, f64::from(j) / 20.0);
            match (a.map(u, v), b.map(u, v)) {
                (Some(p), Some(q)) => errors.push(p.distance(q) / diagonal),
                _ => errors.push(1.0),
            }
        }
    }
    errors.sort_by(f64::total_cmp);
    errors[(errors.len() * 95 / 100).min(errors.len() - 1)]
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Eight bent scenes of one shape, each fitted; returns the errors and checks the direction.
fn run_scenes(mode: BendMode, depth: f64, true_f: f64, fit_f: f64, fit_focal: bool, spoil: Spoil) -> (Vec<f64>, Vec<Sheet>) {
    let profile = Profile::new();
    let mut errors = Vec::new();
    let mut fits = Vec::new();
    for seed in 0..8u64 {
        let y = bump(&profile, depth * (0.8 + 0.05 * seed as f64), 0.15 * (seed % 3) as f64);
        let tilt = [0.12 + 0.04 * (seed % 4) as f64, 0.1 - 0.05 * (seed % 3) as f64, 0.03 * seed as f64 - 0.1];
        let truth = scene(mode, y, true_f, tilt);
        let ev = evidence(&truth, TextDirection::Across, 100 + seed, spoil);
        let fits_for = fit_sheets(&ev, &camera(fit_f), fit_focal).unwrap();
        let curved = fits_for.curved.expect("a curved fit");
        assert_eq!(curved.mode, mode, "scene {seed} found the wrong bend direction");
        errors.push(flatten_error(&truth, &curved));
        fits.push(curved);
    }
    (errors, fits)
}

#[test]
fn book_pages_are_flattened_within_the_specifications_bound() {
    let (errors, _) = run_scenes(BendMode::Across, 0.14, 0.75, 0.75, false, Spoil::default());
    assert!(median(errors.clone()) < 0.0015, "{errors:?}");
}

#[test]
fn a_letter_folded_in_three_is_found_bending_down() {
    let (errors, _) = run_scenes(BendMode::Down, 0.07, 0.75, 0.75, false, Spoil::default());
    assert!(median(errors.clone()) < 0.0015, "{errors:?}");
}

#[test]
fn fitting_the_focal_length_recovers_from_a_wrong_start() {
    // The photo has no EXIF: the start is the typical 0.75, the truth is 0.6.
    let (errors, fits) = run_scenes(BendMode::Across, 0.14, 0.6, 0.75, true, Spoil::default());
    assert!(median(errors.clone()) < 0.0015, "median {} {errors:?}", median(errors.clone()));
    assert!(
        fits.iter().all(|s| s.focal_source == FocalSource::Fitted && (s.focal - 0.6).abs() < 0.12),
        "{:?}",
        fits.iter().map(|s| s.focal).collect::<Vec<_>>()
    );
}

#[test]
fn spoiled_evidence_still_flattens() {
    // A27's tests: 15% of text points wrong, words shifted 3 px within their lines, half of one
    // side hidden. The bound is the spec's, a median under 0.15%. It is measured here on eight
    // scenes a case, and the specification's 40 are WP12's to check on the synthetic sets. The
    // bound was 0.25% until a review found the solver leaving the corner evidence out of its
    // steps (which cost about 0.03% on every case); at 0.25% that went unseen.
    let cases =
        [Spoil { outliers: 0.15, ..Spoil::default() }, Spoil { shift: 3.0, ..Spoil::default() }, Spoil { half_side: true, ..Spoil::default() }];
    for spoil in cases {
        let (errors, _) = run_scenes(BendMode::Across, 0.14, 0.75, 0.75, false, spoil);
        assert!(median(errors.clone()) < 0.0015, "{errors:?}");
    }
}

#[test]
fn a_page_without_text_still_flattens_from_its_outline() {
    // Outline and corners only: coarser than with text, but still far under what the corners
    // alone give (about 1.4% on a book page).
    let (errors, _) = run_scenes(BendMode::Across, 0.14, 0.75, 0.75, false, Spoil { no_text: true, ..Spoil::default() });
    assert!(median(errors.clone()) < 0.0015, "{errors:?}");
}

#[test]
fn a_flat_page_barely_moves_under_the_curved_fit() {
    let truth = scene(BendMode::Across, [0.0; 10], 0.75, [0.2, 0.1, 0.0]);
    let mut worst = 0.0f64;
    for seed in 0..6 {
        let ev = evidence(&truth, TextDirection::Across, 7 + seed, Spoil::default());
        let fits = fit_sheets(&ev, &camera(0.75), false).unwrap();
        let curved = fits.curved.unwrap();
        worst = worst.max(flatten_error(&truth, &curved));
        let plane = fits.plane.unwrap();
        assert!(plane.profile.iter().all(|&v| v == 0.0));
        assert!(flatten_error(&truth, &plane) < 0.002);
    }
    assert!(worst < 0.002, "{worst}");
}

#[test]
fn the_plane_fit_costs_more_on_a_bent_page() {
    let profile = Profile::new();
    let truth = scene(BendMode::Across, bump(&profile, 0.16, 0.0), 0.75, [0.15, 0.1, 0.0]);
    let ev = evidence(&truth, TextDirection::Across, 5, Spoil::default());
    let fits = fit_sheets(&ev, &camera(0.75), false).unwrap();
    let (plane, curved) = (fits.plane.unwrap(), fits.curved.unwrap());
    assert!(plane.cost > 3.0 * curved.cost, "plane {} curved {}", plane.cost, curved.cost);
}

#[test]
fn text_running_down_the_page_fits_too() {
    let profile = Profile::new();
    let truth = scene(BendMode::Down, bump(&profile, 0.08, 0.0), 0.75, [0.1, 0.12, 0.0]);
    // Text running down the page, in bend-down mode: the lines keep b fixed.
    let ev = evidence(&truth, TextDirection::Down, 9, Spoil::default());
    let curved = fit_sheets(&ev, &camera(0.75), false).unwrap().curved.unwrap();
    assert_eq!(curved.mode, BendMode::Down);
    assert!(flatten_error(&truth, &curved) < 0.003);
}

#[test]
fn the_fit_is_the_same_on_any_thread() {
    let profile = Profile::new();
    let truth = scene(BendMode::Across, bump(&profile, 0.12, 0.0), 0.75, [0.1, 0.1, 0.0]);
    let ev = evidence(&truth, TextDirection::Across, 3, Spoil::default());
    let here = fit_sheets(&ev, &camera(0.75), true).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let ev = ev.clone();
            std::thread::spawn(move || fit_sheets(&ev, &camera(0.75), true).unwrap())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), here);
    }
}

#[test]
fn degenerate_evidence_is_an_error_or_no_fit_and_never_a_panic() {
    let profile = Profile::new();
    let truth = scene(BendMode::Across, bump(&profile, 0.1, 0.0), 0.75, [0.1, 0.1, 0.0]);
    let good = evidence(&truth, TextDirection::Across, 1, Spoil::default());

    // Not finite.
    let mut bad = good.clone();
    bad.corners[2].point = Point::new(f64::NAN, 3.0);
    assert!(fit_sheets(&bad, &camera(0.75), false).is_err());

    // All four corners and every point on one line.
    let mut line = good.clone();
    for (k, corner) in line.corners.iter_mut().enumerate() {
        corner.point = Point::new(100.0 + 50.0 * k as f64, 200.0 + 25.0 * k as f64);
    }
    for l in &mut line.lines {
        for (k, p) in l.points.iter_mut().enumerate() {
            *p = Point::new(100.0 + 3.0 * k as f64, 200.0 + 1.5 * k as f64);
        }
    }
    for s in &mut line.sides {
        for (k, p) in s.points.iter_mut().enumerate() {
            *p = Point::new(100.0 + 3.0 * k as f64, 200.0 + 1.5 * k as f64);
        }
    }
    if let Ok(fits) = fit_sheets(&line, &camera(0.75), true) {
        // Whatever it returns must at least be finite.
        for sheet in fits.plane.iter().chain(&fits.curved) {
            assert!(sheet.cost.is_finite() && sheet.translation.iter().all(|v| v.is_finite()));
        }
    }

    // A corner far outside the photo, and one repeated.
    let mut far = good.clone();
    far.corners[1].point = Point::new(1e9, -1e9);
    let _ = fit_sheets(&far, &camera(0.75), true);
    let mut twice = good.clone();
    twice.corners[3] = twice.corners[0];
    let _ = fit_sheets(&twice, &camera(0.75), true);

    // No evidence but the corners; and an empty line.
    let mut bare = good;
    bare.lines = vec![LineEvidence { points: vec![], sigma: vec![] }];
    bare.sides.clear();
    let _ = fit_sheets(&bare, &camera(0.75), false);
}

// ---- A28 ----

fn assert_close(a: Point, b: Point, tolerance: f64, what: &str) {
    assert!(a.distance(b) < tolerance, "{what}: {a:?} vs {b:?}");
}

#[test]
fn the_mapping_matches_direct_evaluation() {
    let profile = Profile::new();
    for mode in [BendMode::Across, BendMode::Down] {
        let truth = scene(mode, bump(&profile, 0.15, 0.2), 0.75, [0.15, -0.1, 0.05]);
        let unrolled = unroll(&truth.sheet, &truth.camera).unwrap();
        let s = unrolled.length();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.8), (0.97, 0.02), (0.5, 0.5)] {
            // Find t for the arc length by bisection on a fine integral, independently of the table.
            let along = if mode == BendMode::Across { u } else { v };
            let b = if mode == BendMode::Across { v } else { u };
            let arc = |t: f64| {
                let n = 20_000;
                (0..n)
                    .map(|i| {
                        let x = t * (f64::from(i) + 0.5) / f64::from(n);
                        truth.sheet.chord * (1.0 + profile.derivative(&truth.sheet.profile, x).powi(2)).sqrt() * t / f64::from(n)
                    })
                    .sum::<f64>()
            };
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if arc(mid) < along * s {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let expected = truth_point(&truth, 0.5 * (lo + hi), b);
            assert_close(unrolled.map(u, v).unwrap(), expected, 1e-3, "map");
        }
    }
}

#[test]
fn incremental_spans_match_the_mapping() {
    let profile = Profile::new();
    for mode in [BendMode::Across, BendMode::Down] {
        let truth = scene(mode, bump(&profile, 0.15, 0.2), 0.75, [0.15, -0.1, 0.05]);
        for turns in 0..4u8 {
            let unrolled = unroll(&truth.sheet, &truth.camera).unwrap().turned(turns);
            for order in [ScanOrder::Rows, ScanOrder::Columns] {
                for fixed in [0.013, 0.5, 0.987] {
                    let mut out = vec![None; 301];
                    unrolled.map_span(order, fixed, &mut out);
                    for (k, got) in out.iter().enumerate() {
                        let t = (k as f64 + 0.5) / 301.0;
                        let direct = match order {
                            ScanOrder::Rows => unrolled.map(t, fixed),
                            ScanOrder::Columns => unrolled.map(fixed, t),
                        };
                        assert_close(got.unwrap(), direct.unwrap(), 1e-4, &format!("{mode:?} turns {turns} {order:?}"));
                    }
                }
            }
        }
    }
}

#[test]
fn a_plane_gives_the_homographys_pixels() {
    let truth = scene(BendMode::Across, [0.0; 10], 0.75, [0.25, 0.1, 0.05]);
    for turns in 0..4u8 {
        let unrolled = unroll(&truth.sheet, &truth.camera).unwrap();
        let corners = unrolled.corners().unwrap();
        let h = Homography::from_unit_square_turned(&corners, truth.camera.long_side(), turns).unwrap();
        let turned = unrolled.turned(turns);
        for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.9, 0.7), (0.3, 0.95)] {
            assert_close(turned.map(u, v).unwrap(), h.apply(u, v).unwrap(), 1e-6, &format!("turns {turns}"));
        }
        let (w, hh) = turned.side_lengths_px().unwrap();
        assert!(w > 0.0 && hh > 0.0);
        let flat = unroll(&truth.sheet, &truth.camera).unwrap().ratio().unwrap();
        assert!((turned.ratio().unwrap() - if turns % 2 == 1 { 1.0 / flat } else { flat }).abs() < 1e-12);
    }
}

#[test]
fn a_profile_of_known_length_gives_its_length() {
    let profile = Profile::new();
    // A flat profile is as long as its chord.
    let flat = unroll(&scene(BendMode::Across, [0.0; 10], 0.75, [0.0; 3]).sheet, &camera(0.75)).unwrap();
    assert!((flat.length() - 0.72).abs() < 1e-5 * 0.72);
    assert!((flat.ratio().unwrap() - 0.72).abs() < 1e-5);
    // A bent one: against a Simpson integral of the spline's own slope.
    let truth = scene(BendMode::Across, bump(&profile, 0.2, 0.3), 0.75, [0.0; 3]);
    let unrolled = unroll(&truth.sheet, &truth.camera).unwrap();
    let n = 200_000;
    let step = 1.0 / f64::from(n);
    let f = |x: f64| truth.sheet.chord * (1.0 + profile.derivative(&truth.sheet.profile, x).powi(2)).sqrt();
    let mut total = f(0.0) + f(1.0);
    for i in 1..n {
        total += f(f64::from(i) * step) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    let exact = total * step / 3.0;
    assert!((unrolled.length() - exact).abs() < 1e-5 * exact, "{} vs {exact}", unrolled.length());
    assert!(unrolled.length() > truth.sheet.chord);
    // A bend-down sheet's ratio is 1/S.
    let down = unroll(&scene(BendMode::Down, bump(&profile, 0.2, 0.3), 0.75, [0.0; 3]).sheet, &camera(0.75)).unwrap();
    assert!((down.ratio().unwrap() * down.length() - 1.0).abs() < 1e-12);
}

#[test]
fn text_by_the_spine_is_as_wide_as_text_in_the_middle() {
    // A book page rising steeply from the spine at u = 0: equal steps of u cover equal lengths
    // of paper, so equal-width words come out equally wide.
    let profile = Profile::new();
    let mut y = [0.0; 10];
    for (k, value) in y.iter_mut().enumerate() {
        let t = profile.knots()[k + 1];
        *value = 0.25 * (1.0 - t).powi(2) * (std::f64::consts::PI * t.sqrt()).sin();
    }
    let truth = scene(BendMode::Across, y, 0.75, [0.1, 0.0, 0.0]);
    let unrolled = unroll(&truth.sheet, &truth.camera).unwrap();
    let span = |u: f64| {
        let (a, b) = (unrolled.camera_point(u, 0.5), unrolled.camera_point(u + 0.01, 0.5));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let (near, middle) = (span(0.01), span(0.5));
    assert!((near / middle - 1.0).abs() < 0.02, "{near} vs {middle}");
}

#[test]
fn the_fitted_outline_has_256_points_per_side_and_ends_at_the_corners() {
    let profile = Profile::new();
    let truth = scene(BendMode::Across, bump(&profile, 0.15, 0.0), 0.75, [0.15, 0.0, 0.0]);
    let unrolled = unroll(&truth.sheet, &truth.camera).unwrap();
    let outline = unrolled.outline().unwrap();
    assert!(outline.iter().all(|side| side.len() == 256));
    let corners = unrolled.corners().unwrap();
    assert_close(outline[0][0], corners.tl(), 1e-9, "top starts at TL");
    assert_close(outline[0][255], corners.tr(), 1e-9, "top ends at TR");
    assert_close(outline[1][255], corners.br(), 1e-9, "right ends at BR");
    assert_close(outline[2][255], corners.bl(), 1e-9, "bottom ends at BL");
    assert_close(outline[3][255], corners.tl(), 1e-9, "left ends at TL");
    // The corners are the true ones.
    assert_close(corners.tl(), truth_point(&truth, 0.0, 0.0), 1e-6, "TL");
    assert_close(corners.br(), truth_point(&truth, 1.0, 1.0), 1e-6, "BR");
}

#[test]
fn unrolling_rejects_unusable_sheets_without_panicking() {
    let mut sheet = scene(BendMode::Across, [0.0; 10], 0.75, [0.0; 3]).sheet;
    sheet.chord = f64::NAN;
    assert!(unroll(&sheet, &camera(0.75)).is_err());
    sheet.chord = 0.0;
    assert!(unroll(&sheet, &camera(0.75)).is_err());
    let mut sheet = scene(BendMode::Across, [0.0; 10], 0.75, [0.0; 3]).sheet;
    sheet.profile[3] = f64::INFINITY;
    assert!(unroll(&sheet, &camera(0.75)).is_err());
    // A sheet behind the camera maps to nothing.
    let mut behind = scene(BendMode::Across, [0.0; 10], 0.75, [0.0; 3]).sheet;
    behind.translation[2] = -5.0;
    let unrolled = unroll(&behind, &camera(0.75)).unwrap();
    assert!(unrolled.map(0.5, 0.5).is_none());
    assert!(unrolled.outline().is_err());
    let mut out = vec![Some(Point::new(0.0, 0.0)); 8];
    unrolled.map_span(ScanOrder::Columns, 0.5, &mut out);
    assert!(out.iter().all(Option::is_none));
    assert!(unrolled.map(f64::NAN, 0.5).is_none());
}

// ---- the Jacobian ----

fn residuals(state: &State, prepared: &Prepared, camera: &Camera, profile: &Profile, fit_focal: bool, plane: bool) -> Vec<[f64; 2]> {
    build_rows(state, prepared, camera, profile, fit_focal, plane).unwrap().0.iter().map(|row| row.e).collect()
}

/// Every analytic derivative of the residuals agrees with a central difference.
#[test]
fn the_jacobian_matches_finite_differences() {
    let profile = Profile::new();
    for (mode, fit_focal, plane) in [(BendMode::Across, true, false), (BendMode::Down, false, false), (BendMode::Across, false, true)] {
        let truth = scene(mode, bump(&profile, 0.12, 0.1), 0.75, [0.15, -0.1, 0.05]);
        let ev = evidence(&truth, TextDirection::Across, 11, Spoil::default());
        let cam = camera(0.75);
        let prepared = prepare(&ev, &cam, mode).unwrap();
        let (rotation, translation, chord) = start_pose(&evidence_quad(&ev), &cam, mode).unwrap();
        let state = State {
            mode,
            rotation,
            translation,
            chord,
            profile: bump(&profile, 0.05, 0.0),
            focal: 0.8,
            lines: prepared.line_starts.clone(),
            points: prepared.point_starts.clone(),
        };
        let (rows, _) = build_rows(&state, &prepared, &cam, &profile, fit_focal, plane).unwrap();
        let first_line = shared_count(fit_focal, 0);
        let h = 1e-6;
        // (column index, how to move the state by ±h)
        let mut moves: Vec<(usize, Box<dyn Fn(&mut State, f64)>)> = Vec::new();
        for k in 0..3 {
            moves.push((
                ROTATION + k,
                Box::new(move |s: &mut State, d: f64| {
                    let mut w = [0.0; 3];
                    w[k] = d;
                    s.rotation = crate::solver::mat_mul(&rodrigues(w), &s.rotation);
                }),
            ));
            moves.push((TRANSLATION + k, Box::new(move |s: &mut State, d: f64| s.translation[k] += d)));
        }
        moves.push((CHORD, Box::new(|s: &mut State, d: f64| s.chord += d)));
        if !plane {
            for j in 0..10 {
                moves.push((PROFILE + j, Box::new(move |s: &mut State, d: f64| s.profile[j] += d)));
            }
        }
        if fit_focal {
            moves.push((FOCAL, Box::new(|s: &mut State, d: f64| s.focal += d)));
        }
        for line in 0..state.lines.len().min(3) {
            moves.push((first_line + line, Box::new(move |s: &mut State, d: f64| s.lines[line] += d)));
        }
        for (column, apply_move) in &moves {
            let (mut plus, mut minus) = (state.clone(), state.clone());
            apply_move(&mut plus, h);
            apply_move(&mut minus, -h);
            let (rp, rm) =
                (residuals(&plus, &prepared, &cam, &profile, fit_focal, plane), residuals(&minus, &prepared, &cam, &profile, fit_focal, plane));
            for (index, row) in rows.iter().enumerate() {
                let numeric = [(rp[index][0] - rm[index][0]) / (2.0 * h), (rp[index][1] - rm[index][1]) / (2.0 * h)];
                let analytic = row.columns.iter().find(|c| c.0 == *column).map_or([0.0, 0.0], |c| [c.1, c.2]);
                for axis in 0..2 {
                    let scale = numeric[axis].abs().max(analytic[axis].abs()).max(1.0);
                    assert!(
                        (numeric[axis] - analytic[axis]).abs() < 1e-4 * scale,
                        "{mode:?} focal {fit_focal} plane {plane}: column {column}, row {index}, axis {axis}: numeric {} analytic {}",
                        numeric[axis],
                        analytic[axis]
                    );
                }
            }
        }
        // The points' own coordinates.
        for (index, row) in rows.iter().enumerate().take(200) {
            let Some(own) = prepared.observations[index].own else { continue };
            let (mut plus, mut minus) = (state.clone(), state.clone());
            plus.points[own] += h;
            minus.points[own] -= h;
            let (rp, rm) =
                (residuals(&plus, &prepared, &cam, &profile, fit_focal, plane), residuals(&minus, &prepared, &cam, &profile, fit_focal, plane));
            for axis in 0..2 {
                let numeric = (rp[index][axis] - rm[index][axis]) / (2.0 * h);
                assert!((numeric - row.a[axis]).abs() < 1e-4 * numeric.abs().max(1.0), "own coordinate, row {index}: {numeric} vs {}", row.a[axis]);
            }
        }
    }
}

/// The start pose reproduces the corners.
#[test]
fn the_start_pose_places_the_corners() {
    let profile = Profile::new();
    for mode in [BendMode::Across, BendMode::Down] {
        let truth = scene(mode, [0.0; 10], 0.75, [0.2, -0.1, 0.05]);
        let mut ev = evidence(&truth, TextDirection::Across, 2, Spoil::default());
        for (k, corner) in ev.corners.iter_mut().enumerate() {
            let (t, b) = corner_sheet_coordinates(mode)[k];
            corner.point = truth_point(&truth, t, b);
        }
        let cam = camera(0.75);
        let (rotation, translation, chord) = start_pose(&evidence_quad(&ev), &cam, mode).unwrap();
        for (k, corner) in ev.corners.iter().enumerate() {
            let (t, b) = corner_sheet_coordinates(mode)[k];
            let q = camera_space(&rotation, &translation, &sheet_point(mode, t, b, chord, profile.value(&[0.0; 10], t)));
            assert_close(cam.project(q).unwrap(), corner.point, 1e-6, &format!("{mode:?} corner {k}"));
        }
        assert!((chord - truth.sheet.chord).abs() < 1e-6, "{mode:?}: chord {chord}");
    }
}

/// The penalties' gradient and second derivative, as the solver adds them, are those of their costs.
#[test]
fn the_penalty_terms_match_their_costs() {
    let profile = Profile::new();
    let cam = camera(0.75);
    let state = State {
        mode: BendMode::Across,
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [0.0, 0.0, 1.0],
        chord: 0.7,
        profile: bump(&profile, 0.1, 0.2),
        focal: 0.9,
        lines: vec![],
        points: vec![],
    };
    let total = |s: &State| bending_cost(s, &profile, false) + focal_cost(s, &cam, true);
    let terms = extra_terms(&state, &profile, &cam, true, false);
    let h = 1e-5;
    let columns: Vec<usize> = (PROFILE..PROFILE + 10).chain([FOCAL]).collect();
    let nudge = |s: &State, column: usize, d: f64| {
        let mut s = s.clone();
        if column == FOCAL {
            s.focal += d;
        } else {
            s.profile[column - PROFILE] += d;
        }
        s
    };
    for &i in &columns {
        let first: f64 = terms.iter().filter(|t| t.row == i).map(|t| t.first).sum();
        let numeric = (total(&nudge(&state, i, h)) - total(&nudge(&state, i, -h))) / (2.0 * h);
        assert!((first - numeric).abs() < 1e-5 * numeric.abs().max(1.0), "gradient {i}: {first} vs {numeric}");
        for &j in &columns {
            let second: f64 = terms.iter().filter(|t| t.row == i && t.column == j).map(|t| t.second).sum();
            let numeric =
                (total(&nudge(&nudge(&state, i, h), j, h)) - total(&nudge(&nudge(&state, i, h), j, -h)) - total(&nudge(&nudge(&state, i, -h), j, h))
                    + total(&nudge(&nudge(&state, i, -h), j, -h)))
                    / (4.0 * h * h);
            assert!((second - numeric).abs() < 1e-4 * numeric.abs().max(1.0), "second {i},{j}: {second} vs {numeric}");
        }
    }
}

#[test]
fn exact_evidence_recovers_the_sheet() {
    let (errors, fits) = run_scenes(BendMode::Across, 0.14, 0.75, 0.75, false, Spoil { exact: true, ..Spoil::default() });
    assert!(errors.iter().all(|&e| e < 0.0008), "{errors:?}");
    let (errors, _) = run_scenes(BendMode::Down, 0.07, 0.75, 0.75, false, Spoil { exact: true, ..Spoil::default() });
    assert!(errors.iter().all(|&e| e < 0.0008), "{errors:?}");
    assert!(fits.iter().all(|s| s.cost < 1.0), "{:?}", fits.iter().map(|s| s.cost).collect::<Vec<_>>());
}

/// The fit's cost is no higher than the cost at the truth, so any error is the evidence's, not the
/// optimiser's.
#[test]
fn the_fit_finds_a_cost_no_higher_than_the_truths() {
    let profile = Profile::new();
    for (mode, depth) in [(BendMode::Across, 0.14), (BendMode::Down, 0.07)] {
        for seed in 0..8u64 {
            let truth = scene(
                mode,
                bump(&profile, depth * (0.8 + 0.05 * seed as f64), 0.15 * (seed % 3) as f64),
                0.75,
                [0.12 + 0.04 * (seed % 4) as f64, 0.1 - 0.05 * (seed % 3) as f64, 0.03 * seed as f64 - 0.1],
            );
            let (ev, lines, own) = evidence_with_truth(&truth, TextDirection::Across, 100 + seed, Spoil::default());
            let cam = camera(0.75);
            let fitted = fit_sheets(&ev, &cam, false).unwrap().curved.unwrap();
            let prepared = prepare(&ev, &cam, mode).unwrap();
            let r = &truth.sheet.rotation;
            let state = State {
                mode,
                rotation: [[r[0], r[1], r[2]], [r[3], r[4], r[5]], [r[6], r[7], r[8]]],
                translation: truth.sheet.translation,
                chord: truth.sheet.chord,
                profile: truth.sheet.profile,
                focal: 0.75,
                lines,
                points: own,
            };
            let at_truth = evaluate(&state, &prepared, &cam, &profile, false, false).unwrap();
            assert!(fitted.cost <= at_truth + 1e-6 * at_truth, "{mode:?} scene {seed}: fit {} truth {at_truth}", fitted.cost);
        }
    }
}
