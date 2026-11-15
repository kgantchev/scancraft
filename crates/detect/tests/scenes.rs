//! Detection of a whole photo (specification §8 A9 and A10 tests, §10): WP0's fixtures, WP4's
//! synthetic scenes, and a few pages drawn here for the cases the generator can't make.

use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{Plane, Rgb8};
use scancraft_core::recipe::{CornerOrigin, Detector};
use scancraft_core::traits::{PageDetector, ServiceError};
use scancraft_detect::detect::detect_page;
use scancraft_synth::generate::draw_params;
use scancraft_synth::metrics::{corner_error, jaccard};
use scancraft_synth::scene_io::read_scene;
use scancraft_synth::{Scene, SceneSpec, generate};
use std::path::Path;

// ---- helpers ----

fn fixture(name: &str) -> Scene {
    read_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"), name).unwrap()
}

/// How far the found corners are from the true ones, as a share of the photo's diagonal.
///
/// The generator gives the page's own corner order, and the camera is rolled by any angle, while
/// `corner_error` wants both quads in the canonical order of §7.2.
fn error_of(found: &Quad, scene: &Scene) -> f64 {
    let size = (scene.photo.width(), scene.photo.height());
    corner_error(found, &Quad::canonical(scene.truth.corners.0), size).unwrap()
}

/// How far inside edge `a → b` of a clockwise quadrilateral a point lies, in pixels.
fn depth(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    (dx * (p.y - a.y) - dy * (p.x - a.x)) / dx.hypot(dy)
}

/// The share of pixel `(x, y)` inside the quadrilateral, from a 4 × 4 grid of samples.
fn coverage(corners: &[Point; 4], x: u32, y: u32) -> f64 {
    let centre = Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
    let nearest = (0..4).map(|i| depth(corners[i], corners[(i + 1) % 4], centre)).fold(f64::INFINITY, f64::min);
    if nearest > 1.0 {
        return 1.0;
    }
    if nearest < -1.0 {
        return 0.0;
    }
    let mut inside = 0u32;
    for j in 0..4 {
        for i in 0..4 {
            let p = Point::new(f64::from(x) + (f64::from(i) + 0.5) / 4.0, f64::from(y) + (f64::from(j) + 0.5) / 4.0);
            if (0..4).all(|k| depth(corners[k], corners[(k + 1) % 4], p) > 0.0) {
                inside += 1;
            }
        }
    }
    f64::from(inside) / 16.0
}

/// A photo of a page (a colour) on a desk (another colour), drawn with partial coverage at the
/// edges. `ruling` adds a frame of dark lines that far inside the page.
fn drawn(size: (u32, u32), corners: [Point; 4], page: [f32; 3], desk: [f32; 3], ruling: Option<f64>) -> Rgb8 {
    let inset = |d: f64| -> [Point; 4] {
        [
            Point::new(corners[0].x + d, corners[0].y + d),
            Point::new(corners[1].x - d, corners[1].y + d),
            Point::new(corners[2].x - d, corners[2].y - d),
            Point::new(corners[3].x + d, corners[3].y - d),
        ]
    };
    let mut data = Vec::with_capacity(size.0 as usize * size.1 as usize);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let c = coverage(&corners, x, y);
            let mut px = [0.0f32; 3];
            for k in 0..3 {
                px[k] = desk[k] + (page[k] - desk[k]) * c as f32;
            }
            if let Some(d) = ruling {
                let ring = coverage(&inset(d), x, y) - coverage(&inset(d + 3.0), x, y);
                for v in &mut px {
                    *v += (0.15 - *v) * ring as f32;
                }
            }
            data.push(px.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8));
        }
    }
    Plane::from_vec(size.0, size.1, data).unwrap()
}

/// A page the tests draw: tilted a little, well inside a 1600 × 1200 frame.
fn page() -> [Point; 4] {
    [Point::new(310.4, 220.7), Point::new(1310.6, 262.3), Point::new(1285.2, 1010.8), Point::new(335.9, 982.4)]
}

const LIGHT_PAGE: [f32; 3] = [0.93, 0.93, 0.90];
const DARK_DESK: [f32; 3] = [0.16, 0.15, 0.14];

// ---- the fixtures ----

#[test]
fn the_flat_fixture_is_found_to_the_pixel() {
    let scene = fixture("flat");
    let d = detect_page(&scene.photo, None, 1).unwrap();
    let error = error_of(&d.refined.corners, &scene);
    assert!(error <= 0.005, "corner error {error:.4} of the diagonal");
    assert!(matches!(d.origin, CornerOrigin::Detected { .. }), "{:?}", d.origin);
    assert_eq!(d.refined.unrefined, [false; 4]);
}

#[test]
fn the_curved_and_spread_fixtures_are_found_without_error_or_panic() {
    // Their curved sides are A25's work (WP11); here the detection must still give a page.
    for name in ["curved", "spread"] {
        let scene = fixture(name);
        let d = detect_page(&scene.photo, None, 1).unwrap();
        assert!(d.refined.corners.is_finite(), "{name}");
        assert!(jaccard(&d.refined.corners, &Quad::canonical(scene.truth.corners.0)).unwrap() > 0.8, "{name}");
    }
}

// ---- synthetic scenes ----

/// Scenes with a dark desk or wood, and no finger over the page: the edge between page and desk is
/// plain there, so the classical path must find the page every time but a few.
#[test]
fn clean_scenes_on_a_dark_desk_or_wood_are_found() {
    use scancraft_synth::background::Background;
    let spec = SceneSpec { count: 100, seed: 1, curved: false };
    let (mut tried, mut found, mut exact) = (0u32, 0u32, 0u32);
    for index in 0..spec.count {
        let p = draw_params(&spec, index).unwrap();
        if p.damage.is_finger() || !matches!(p.background, Background::DarkDesk | Background::Wood) {
            continue;
        }
        let scene = generate(&spec, index).unwrap();
        let d = detect_page(&scene.photo, None, 1000 + u64::from(index)).unwrap();
        let error = error_of(&d.refined.corners, &scene);
        tried += 1;
        found += u32::from(error <= 0.02);
        exact += u32::from(error <= 0.005);
        if tried >= 14 {
            break;
        }
    }
    assert!(tried >= 12, "only {tried} scenes qualified");
    // Measured on this set: 14 of 14 on a dark desk and 12 of 14 on wood.
    assert!(found * 100 >= tried * 85, "{found} of {tried} scenes found");
    assert!(exact * 100 >= tried * 85, "{exact} of {tried} scenes found to within 0.5%");
}

// ---- drawn scenes: the cases of A9's tests ----

#[test]
fn a_ruled_frame_inside_the_page_does_not_win() {
    // The frame lies 12 px inside the paper's edge, as strong as the edge on any one line.
    let corners = page();
    let photo = drawn((1600, 1200), corners, LIGHT_PAGE, DARK_DESK, Some(12.0));
    let d = detect_page(&photo, None, 3).unwrap();
    let truth = Quad(corners);
    let worst = d.refined.corners.0.iter().zip(truth.0.iter()).map(|(a, b)| a.distance(*b)).fold(0.0f64, f64::max);
    assert!(worst < 2.0, "the corners are {worst:.2} px from the paper's edge: {:?}", d.refined.corners);
    assert!(matches!(d.origin, CornerOrigin::Detected { .. }));
}

#[test]
fn a_desk_with_no_page_gives_not_found() {
    let mut rng = Xorshift64Star::new(4);
    // A plain desk, a desk with a lighting gradient, and a desk with a little noise.
    for kind in 0..3 {
        let (w, h) = (1200u32, 900u32);
        let mut data = Vec::with_capacity((w * h) as usize);
        for _ in 0..h {
            for x in 0..w {
                let g = match kind {
                    0 => 0.35,
                    1 => 0.25 + 0.3 * f64::from(x) / f64::from(w),
                    _ => 0.35 + 0.02 * (rng.next_f64() - 0.5),
                };
                data.push([(g * 255.0) as u8, (g * 240.0) as u8, (g * 220.0) as u8]);
            }
        }
        let photo = Plane::from_vec(w, h, data).unwrap();
        let d = detect_page(&photo, None, 5).unwrap();
        assert_eq!(d.origin, CornerOrigin::NotFound, "desk {kind}");
        // The corners start 3% inside the photo's edges, and every side is marked unrefined.
        let c = d.refined.corners;
        assert!((c.tl().x - 0.03 * f64::from(w)).abs() < 1.0 && (c.tl().y - 0.03 * f64::from(h)).abs() < 1.0, "{c:?}");
        assert_eq!(d.refined.unrefined, [true; 4]);
    }
}

#[test]
fn a_page_larger_than_the_frame_is_found_using_the_border_sides() {
    // The page spills past the top and the left of the photo, so those two sides can't be seen;
    // its right and bottom edges are in the frame. The two unseen sides are border sides: they get
    // a fixed support of 0.35 instead of dragging the page's score to nothing.
    let (w, h) = (1200.0, 900.0);
    let corners = [Point::new(-80.0, -60.0), Point::new(w - 120.0, -50.0), Point::new(w - 130.0, h - 100.0), Point::new(-90.0, h - 110.0)];
    let photo = drawn((1200, 900), corners, LIGHT_PAGE, DARK_DESK, None);
    let d = detect_page(&photo, None, 6).unwrap();
    assert!(matches!(d.origin, CornerOrigin::Detected { .. }), "{:?}", d.origin);
    let c = d.refined.corners;
    // The visible corner is found to a few pixels, and the corners nobody can see sit at the frame
    // or past it, never inside the page.
    assert!(c.br().distance(corners[2]) < 3.0, "the bottom right corner is at {:?}", c.br());
    assert!(c.tl().x < 5.0 && c.tl().y < 5.0, "the top left corner is at {:?}", c.tl());
    assert!(c.tr().y < 5.0 && c.bl().x < 5.0, "{c:?}");
}

// ---- A8 through the trait ----

/// A fake detector for the page of `page()` seen in the 1024 px work image of a 1600 × 1200 photo.
struct Fake(Result<Option<Quad>, ServiceError>);

impl PageDetector for Fake {
    fn detect(&self, _: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        self.0.clone()
    }
}

#[test]
fn the_platform_detector_is_used_when_the_classical_path_finds_nothing() {
    // A page whose edge is far too faint for edges to find: the desk is almost the page's colour.
    let corners = page();
    let photo = drawn((1600, 1200), corners, [0.50, 0.50, 0.50], [0.495, 0.495, 0.495], None);
    let alone = detect_page(&photo, None, 7).unwrap();
    assert_eq!(alone.origin, CornerOrigin::NotFound);
    // Vision's answer is in the 1024 px work image: the photo scaled by 1024 / 1600.
    let k = 1024.0 / 1600.0;
    let apple = Quad(corners.map(|p| Point::new(p.x * k, p.y * k)));
    let d = detect_page(&photo, Some(&Fake(Ok(Some(apple)))), 7).unwrap();
    assert!(matches!(d.origin, CornerOrigin::Detected { by: Detector::Apple, .. }), "{:?}", d.origin);
    let worst = d.refined.corners.0.iter().zip(corners.iter()).map(|(a, b)| a.distance(*b)).fold(0.0f64, f64::max);
    assert!(worst < 3.0, "{worst:.2} px from the page");
}

#[test]
fn a_platform_detector_that_fails_or_answers_with_junk_changes_nothing() {
    let corners = page();
    let photo = drawn((1600, 1200), corners, LIGHT_PAGE, DARK_DESK, None);
    let alone = detect_page(&photo, None, 8).unwrap();
    let nan = Quad([Point::new(f64::NAN, 0.0); 4]);
    let sliver = Quad([Point::new(0.0, 0.0), Point::new(900.0, 1.0), Point::new(900.0, 2.0), Point::new(0.0, 1.0)]);
    for answer in [
        Err(ServiceError::Unavailable),
        Err(ServiceError::TimedOut(2000)),
        Err(ServiceError::Failed("no".into())),
        Ok(None),
        Ok(Some(nan)),
        Ok(Some(sliver)),
    ] {
        let d = detect_page(&photo, Some(&Fake(answer.clone())), 8).unwrap();
        assert_eq!(d.refined, alone.refined, "{answer:?}");
        assert!(matches!(d.origin, CornerOrigin::Detected { by: Detector::Contours | Detector::Lines, .. }));
    }
}

// ---- determinism and safety ----

#[test]
fn one_thread_and_eight_give_the_same_detection() {
    let scene = fixture("flat");
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| detect_page(&scene.photo, None, 9).unwrap())
    };
    assert_eq!(run(1), run(8));
}

#[test]
fn photos_of_any_size_and_content_never_panic() {
    let mut rng = Xorshift64Star::new(10);
    for (w, h) in [(1, 1), (2, 2), (3, 3), (5, 1), (1, 5), (17, 3), (3, 400), (64, 64), (130, 7), (1025, 3)] {
        for kind in 0..3 {
            let data: Vec<[u8; 3]> = (0..w * h)
                .map(|i| match kind {
                    0 => [0, 0, 0],
                    1 => [255, 255, 255],
                    _ => [(rng.next_u64() & 255) as u8, (i % 251) as u8, (rng.next_u64() >> 8 & 255) as u8],
                })
                .collect();
            let photo = Plane::from_vec(w, h, data).unwrap();
            // An error is fine, a panic is not; a result must hold finite corners.
            if let Ok(d) = detect_page(&photo, None, 11) {
                assert!(d.refined.corners.is_finite(), "{w}×{h} kind {kind}");
            }
        }
    }
}

/// The time of a 12-megapixel photo, for the report against §9's budgets. Run it with
/// `cargo test --release -p scancraft-detect --test scenes -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run in a release build"]
fn timing_of_a_twelve_megapixel_photo() {
    use scancraft_detect::contours::contour_candidates;
    use scancraft_detect::detect::{photo_luma, work_image};
    use scancraft_detect::edges::colour_edges;
    use scancraft_detect::lines::line_candidates;
    use scancraft_detect::refine::{build_pyramid, refine};
    use scancraft_detect::select::{Prepared, select};
    use scancraft_raster::colour::to_f32;
    use std::time::Instant;
    let corners = [Point::new(560.0, 420.0), Point::new(3500.0, 500.0), Point::new(3440.0, 2600.0), Point::new(610.0, 2550.0)];
    let photo = drawn((4032, 3024), corners, LIGHT_PAGE, DARK_DESK, Some(40.0));
    let time = |name: &str, t: Instant| eprintln!("{name:<28}{:>8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    for round in 0..3 {
        eprintln!("round {round}");
        let t = Instant::now();
        let work8 = work_image(&photo).unwrap();
        let work = to_f32(&work8).unwrap();
        time("A3 work image", t);
        let t = Instant::now();
        let maps = colour_edges(&work).unwrap();
        time("A5 colour edges", t);
        let t = Instant::now();
        let mut classical: Vec<(Detector, Quad)> = contour_candidates(&maps).unwrap().into_iter().map(|q| (Detector::Contours, q)).collect();
        time("A6 contour candidates", t);
        let t = Instant::now();
        classical.extend(line_candidates(&maps).unwrap().into_iter().map(|q| (Detector::Lines, q)));
        time("A7 line candidates", t);
        let t = Instant::now();
        let pyramid = build_pyramid(&photo_luma(&photo).unwrap()).unwrap();
        time("luma and pyramid", t);
        let t = Instant::now();
        let prepared = Prepared::new(maps).unwrap();
        let selection = select(&prepared, &classical, None, &pyramid, 1).unwrap();
        time("A9 score + A10 (up to 3)", t);
        let t = Instant::now();
        let _ = refine(&classical[0].1, &pyramid, 1).unwrap();
        time("A10 for one candidate", t);
        let t = Instant::now();
        let _ = detect_page(&photo, None, 1).unwrap();
        time("detect_page, all of it", t);
        eprintln!("candidates {} / chosen {:?}", classical.len(), selection.by);
    }
}
