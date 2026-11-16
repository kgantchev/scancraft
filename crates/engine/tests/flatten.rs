//! A29 through the whole engine: synthetic photos with exact truth go in as files, the page is
//! detected, cropped and applied, and the engine decides whether to flatten it.
//!
//! The scenes come from `scancraft-synth`, which draws them with its own camera and sheet code, so
//! the decision is checked against geometry the engine never saw.

mod common;

use common::*;
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen};
use scancraft_core::geom::Point;
use scancraft_core::recipe::{FlattenMode, FlattenReason, Page};
use scancraft_engine::{Engine, Services};
use scancraft_geom::homography::Homography;
use scancraft_synth::{Scene, SceneSpec, generate};

/// A page finder that knows where the page is, as Vision does when it is right: it answers with
/// the outline in the work image's pixels, and A10 refines it from there.
struct Finder {
    corners: scancraft_core::geom::Quad,
    size: (u32, u32),
}

impl scancraft_core::traits::PageDetector for Finder {
    fn detect(&self, work: &scancraft_core::image::Rgb8) -> Result<Option<scancraft_core::geom::Quad>, scancraft_core::traits::ServiceError> {
        let k = f64::from(work.width()) / f64::from(self.size.0);
        Ok(Some(scancraft_core::geom::Quad(self.corners.0.map(|p| Point::new(p.x * k, p.y * k)))))
    }
}

/// How far the corners alone leave the page from the truth, as a share of its diagonal: the
/// largest distance, over the truth's grid of page points, between where the grid says a point lies
/// and where the corners' homography puts it.
fn corner_error(scene: &Scene) -> f64 {
    let truth = &scene.truth;
    let Some(grid) = &truth.page_grid else { return 0.0 };
    let long = f64::from(truth.size.0.max(truth.size.1));
    let Ok(h) = Homography::from_unit_square(&truth.corners, long) else { return f64::NAN };
    let n = grid.len().max(2) - 1;
    let mut worst = 0.0f64;
    for (j, row) in grid.iter().enumerate() {
        for (i, point) in row.iter().enumerate() {
            let Some(p) = h.apply(i as f64 / n as f64, j as f64 / n as f64) else { return f64::NAN };
            worst = worst.max(p.distance(*point));
        }
    }
    worst / truth.corners.tl().distance(truth.corners.br())
}

/// Imports the scene as a file, applies the crop the engine detected, and waits for A29. Returns
/// the page the engine ended up with.
fn run(scene: &Scene, tag: &str) -> Page {
    run_with(scene, tag, FlattenMode::Auto)
}

/// The same, with the page's Flatten setting changed before it is applied.
fn run_with(scene: &Scene, tag: &str, mode: FlattenMode) -> Page {
    let dir = TempDir::new(tag);
    let finder = Finder { corners: scene.truth.corners, size: scene.truth.size };
    let mut engine: Engine = open(&dir, Services { detector: Some(std::sync::Arc::new(finder)), recognizer: None, upright: None });
    let photo = &scene.photo;
    let bytes = png(photo.width(), photo.height(), |x, y| photo.get(x, y).unwrap_or([0, 0, 0]));
    let path = write(&dir, "scene.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    let (photo_id, page_id) = current(&engine);
    if mode != FlattenMode::Auto {
        engine.dispatch(Command::CropSetFlatten { page: page_id, mode }).unwrap();
    }
    engine.dispatch(Command::CropApply { photo: photo_id }).unwrap();
    assert!(matches!(engine.view().screen, Screen::Enhance(_)));
    settle(&mut engine);
    engine.session().pending().first().cloned().expect("the applied page is waiting for Add Page")
}

fn decision(page: &Page) -> (bool, Option<FlattenReason>) {
    match &page.sheet {
        Some(fit) => (fit.decision.flattened, Some(fit.decision.reason)),
        None => (false, None),
    }
}

// ---- the decisions, on a few scenes ----

/// Scene `index` of the bent set with seed 4242: 3 is a sheet curled through 25°, 8 one curled
/// through 77°, and 2 a book page rising 28° from its spine.
fn bent(index: u32) -> Scene {
    generate(&SceneSpec { count: 12, seed: 4242, curved: true }, index).unwrap()
}

#[test]
fn a_flat_page_is_left_flat() {
    for index in [0, 1, 3] {
        let scene = generate(&SceneSpec { count: 12, seed: 4242, curved: false }, index).unwrap();
        let page = run(&scene, "flat");
        assert!(!decision(&page).0, "flat page {index} was flattened: {:?}", page.sheet);
    }
}

#[test]
fn a_curled_page_is_flattened_and_says_why() {
    for index in [3, 8] {
        let scene = bent(index);
        assert!(corner_error(&scene) > 0.01, "the scene should be one the corners alone get wrong");
        let page = run(&scene, "curl");
        let fit = page.sheet.as_ref().unwrap_or_else(|| panic!("curl {index} has no sheet"));
        assert!(fit.decision.flattened && fit.decision.reason == FlattenReason::Straighter, "curl {index}: {:?}", fit.decision);
        // The record: the size of the bend, the two costs and the two straightness figures.
        let d = &fit.decision;
        assert!(d.d95 >= 0.002 && d.curved_cost <= 0.97 * d.plane_cost);
        assert!(d.straightness_flat.unwrap() < d.straightness_corners.unwrap());
        assert!(d.lines_corners >= 4 && d.lines_flat >= 4);
    }
}

#[test]
fn off_gives_no_sheet_and_on_flattens_the_page() {
    let scene = bent(3);
    let off = run_with(&scene, "off", FlattenMode::Off);
    assert!(off.sheet.is_none(), "Off fitted a sheet");
    let on = run_with(&scene, "on", FlattenMode::On);
    let fit = on.sheet.as_ref().expect("On fits a sheet");
    assert!(fit.decision.flattened && fit.decision.reason == FlattenReason::On, "{:?}", fit.decision);
}

#[test]
fn a_page_with_no_text_and_straight_sides_gets_no_sheet() {
    // A blank desk and sheet: no lines and no curved sides, so there is no evidence (check 1).
    let dir = TempDir::new("blank");
    let mut engine = open(&dir, services(desk_quad()));
    let path = write(&dir, "desk.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    let (photo, _) = current(&engine);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    settle(&mut engine);
    let page = engine.session().pending().first().cloned().unwrap();
    assert!(page.sheet.is_none(), "{:?}", page.sheet);
}

#[test]
fn a25_traces_the_curved_edges_of_a_detected_page() {
    // The curl of 25° bows two opposite edges of the page, and detection hands them on.
    let scene = bent(3);
    let dir = TempDir::new("traced");
    let finder = Finder { corners: scene.truth.corners, size: scene.truth.size };
    let mut engine: Engine = open(&dir, Services { detector: Some(std::sync::Arc::new(finder)), recognizer: None, upright: None });
    let photo = &scene.photo;
    let bytes = png(photo.width(), photo.height(), |x, y| photo.get(x, y).unwrap_or([0, 0, 0]));
    let path = write(&dir, "scene.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    let crop = engine.view().crop.expect("Screen 2");
    let traced = crop.outlines[0].sides.iter().filter(|s| matches!(s, scancraft_core::recipe::SideShape::Traced { .. })).count();
    assert!(traced >= 3, "A25 traced {traced} of the four sides");
    assert!(crop.outlines[0].sides.iter().any(|s| matches!(s, scancraft_core::recipe::SideShape::Traced { curved: true, .. })));
}

#[test]
fn one_thread_and_eight_threads_decide_the_same() {
    use scancraft_core::recipe::FocalInput;
    use scancraft_engine::flatten::flatten_page;
    use scancraft_engine::jobs::CancellationToken;
    use scancraft_engine::pipeline::page_geometry;
    use scancraft_engine::settings::Settings;
    let scene = bent(3);
    let mut page = run(&scene, "threads");
    page.sheet = None;
    let geometry = page_geometry(&page, (scene.photo.width(), scene.photo.height()), FocalInput::default(), &Settings::default()).unwrap();
    let decide = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| flatten_page(&scene.photo, &page, &geometry, &CancellationToken::new()).unwrap())
    };
    let (one, eight) = (decide(1), decide(8));
    assert!(one.fit.is_some());
    assert_eq!(one, eight, "the decision depends on the number of threads");
}

/// The share of scenes of each kind that A29 flattened: the figures the report quotes. Run with
/// `cargo test -p scancraft-engine --test flatten rates -- --ignored --nocapture`.
#[test]
#[ignore = "renders about a hundred scenes and takes minutes"]
fn rates() {
    let mut flat = (0, 0);
    let spec = SceneSpec { count: 30, seed: 9001, curved: false };
    for i in 0..spec.count {
        let page = run(&generate(&spec, i).unwrap(), "rate-flat");
        flat.1 += 1;
        flat.0 += usize::from(!decision(&page).0);
        if decision(&page).0 {
            eprintln!("WRONG flat #{i} flattened: {:?}", page.sheet.as_ref().map(|f| f.decision.clone()));
        }
    }
    eprintln!("RATE flat pages left flat: {} of {}", flat.0, flat.1);
    let mut kinds: std::collections::BTreeMap<&str, (usize, usize, usize)> = std::collections::BTreeMap::new();
    let spec = SceneSpec { count: 200, seed: 9002, curved: true };
    for i in 0..spec.count {
        let scene = generate(&spec, i).unwrap();
        let d = &scene.truth.description;
        let kind = if d.starts_with("sheet curled") {
            "curl"
        } else if d.starts_with("sheet sagging") {
            "sag"
        } else if d.starts_with("book page") {
            "book page"
        } else {
            continue;
        };
        let entry = kinds.entry(kind).or_default();
        if entry.0 >= 16 {
            continue;
        }
        let error = corner_error(&scene);
        let page = run(&scene, "rate-bent");
        entry.0 += 1;
        if error > 0.01 {
            entry.1 += 1;
            entry.2 += usize::from(decision(&page).0);
            if !decision(&page).0 {
                let d = page.sheet.as_ref().map(|f| &f.decision);
                eprintln!(
                    "MISS {kind} #{i} error {:.2}%: {:?} d95 {:.4} cost {:.0}/{:.0} lines {}/{} straight {:.2?}/{:.2?}",
                    error * 100.0,
                    decision(&page).1,
                    d.map_or(0.0, |d| d.d95),
                    d.map_or(0.0, |d| d.plane_cost),
                    d.map_or(0.0, |d| d.curved_cost),
                    d.map_or(0, |d| d.lines_corners),
                    d.map_or(0, |d| d.lines_flat),
                    d.and_then(|d| d.straightness_corners),
                    d.and_then(|d| d.straightness_flat)
                );
            }
        }
    }
    for (kind, (total, over, flattened)) in kinds {
        eprintln!("RATE {kind}: {total} scenes, {over} with corner error over 1% of the diagonal, {flattened} of those flattened");
    }
}
