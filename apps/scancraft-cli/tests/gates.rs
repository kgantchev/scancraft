//! The synthetic release gates of specification §10.4 that need the whole engine: how often Auto
//! flattens a flat page, and how far a flattened bent page lies from the truth. Both run on scenes
//! drawn by `scancraft-synth`, whose camera and sheet code the pipeline never saw.
//!
//! The page finder is given the true outline, as Vision is when it is right, so these figures are
//! the flattening's own: what the classical detector adds is measured by `scancraft-detect`'s
//! `gates` test. Run them with
//! `cargo test --release -p scancraft-cli --test gates -- --ignored --nocapture`.
//! `SCANCRAFT_GATE_COUNT` sets the number of scenes of each set (default 40).

use craft_settings::Location;
use scancraft_core::camera::Camera;
use scancraft_core::command::Command;
use scancraft_core::engine_api::EngineApi;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{FlattenMode, Page};
use scancraft_core::traits::{PageDetector, ServiceError};
use scancraft_engine::pipeline::page_geometry;
use scancraft_engine::{Engine, EngineOptions, Services};
use scancraft_geom::homography::Homography;
use scancraft_geom::sheet::unroll;
use scancraft_synth::{Scene, SceneSpec, generate};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Finder {
    corners: Quad,
    size: (u32, u32),
}

impl PageDetector for Finder {
    fn detect(&self, work: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        let k = f64::from(work.width()) / f64::from(self.size.0);
        Ok(Some(Quad(self.corners.0.map(|p| Point::new(p.x * k, p.y * k)))))
    }
}

fn count() -> u32 {
    std::env::var("SCANCRAFT_GATE_COUNT").ok().and_then(|v| v.parse().ok()).unwrap_or(40)
}

/// The photo goes through the engine with the true outline, and the page comes out applied.
fn run(scene: &Scene, mode: FlattenMode) -> (Page, Engine, std::path::PathBuf) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("scancraft-gates-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let photo = &scene.photo;
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.png");
    let bytes: Vec<u8> = photo.data().iter().flat_map(|p| *p).collect();
    image::save_buffer(&path, &bytes, photo.width(), photo.height(), image::ExtendedColorType::Rgb8).unwrap();
    let services =
        Services { detector: Some(Arc::new(Finder { corners: scene.truth.corners, size: scene.truth.size })), recognizer: None, upright: None };
    let mut options = EngineOptions::new(dir.join("session"), services);
    options.settings = Location::Memory;
    options.sessions_root = Some(dir.clone());
    let mut engine = Engine::open(options).unwrap();
    engine.dispatch(Command::ImportAdd { paths: vec![path.to_string_lossy().into_owned()] }).unwrap();
    settle(&mut engine);
    let crop = engine.view().crop.unwrap();
    if mode != FlattenMode::Auto {
        engine.dispatch(Command::CropSetFlatten { page: crop.outlines[0].page, mode }).unwrap();
    }
    engine.dispatch(Command::CropApply { photo: crop.photo }).unwrap();
    settle(&mut engine);
    let page = engine.session().pending().first().cloned().unwrap();
    (page, engine, dir)
}

fn settle(engine: &mut Engine) {
    let end = Instant::now() + Duration::from_secs(300);
    while Instant::now() < end {
        engine.poll();
        if !engine.view().busy {
            engine.poll();
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the engine did not settle");
}

fn flattened(page: &Page) -> bool {
    page.sheet.as_ref().is_some_and(|f| f.decision.flattened)
}

/// The 17 × 17 grid of where the engine puts each point of the flat page, in the canonical frame.
fn found_grid(page: &Page, engine: &Engine, size: (u32, u32), n: usize) -> Vec<Vec<Point>> {
    let item = engine.session().photo(page.photo).unwrap().clone();
    let geometry = page_geometry(page, size, item.focal, engine.settings()).unwrap();
    let long = f64::from(size.0.max(size.1));
    let h = Homography::from_unit_square(&page.corners, long).unwrap();
    let unrolled = page.sheet.as_ref().filter(|f| f.decision.flattened).map(|fit| {
        let mut camera: Camera = geometry.camera;
        camera.f = fit.sheet.focal;
        camera.width = f64::from(size.0);
        camera.height = f64::from(size.1);
        unroll(&fit.sheet, &camera).unwrap()
    });
    (0..=n)
        .map(|j| {
            (0..=n)
                .map(|i| {
                    let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                    match &unrolled {
                        Some(m) => scancraft_core::map::PageMap::map(m, u, v).unwrap_or(Point::new(f64::NAN, f64::NAN)),
                        None => h.apply(u, v).unwrap_or(Point::new(f64::NAN, f64::NAN)),
                    }
                })
                .collect()
        })
        .collect()
}

fn turn(g: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let n = g.len();
    (0..n).map(|j| (0..n).map(|i| g[n - 1 - i][j]).collect()).collect()
}

/// The 95th percentile of the distances between the found grid and the truth's, as a share of the
/// page's diagonal in the photo. The truth's page may be turned against the canonical frame, so
/// the best of the four turns counts.
fn flattening_error(found: &[Vec<Point>], scene: &Scene) -> f64 {
    let truth = scene.truth.page_grid.as_ref().unwrap();
    let diagonal = scene.truth.corners.tl().distance(scene.truth.corners.br());
    let mut best = f64::INFINITY;
    let mut t: Vec<Vec<Point>> = truth.clone();
    for _ in 0..4 {
        let mut d: Vec<f64> = found.iter().flatten().zip(t.iter().flatten()).map(|(a, b)| a.distance(*b)).collect();
        d.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95 = d.get(((d.len() as f64) * 0.95) as usize).copied().unwrap_or(f64::NAN);
        best = best.min(p95 / diagonal);
        t = turn(&t);
    }
    best
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}

#[test]
#[ignore = "renders many scenes and takes minutes; run in a release build"]
fn the_synthetic_flattening_gates() {
    let n = count();
    // Flat pages that Auto flattens (target: at most 2%).
    let spec = SceneSpec { count: n, seed: 9001, curved: false };
    let mut flat_flattened = 0;
    for i in 0..n {
        let (page, _, dir) = run(&generate(&spec, i).unwrap(), FlattenMode::Auto);
        flat_flattened += u32::from(flattened(&page));
        let _ = std::fs::remove_dir_all(dir);
    }
    eprintln!(
        "GATE flat pages that Auto flattens: {flat_flattened} of {n} ({:.1}%; target at most 2%)",
        100.0 * f64::from(flat_flattened) / f64::from(n)
    );
    // Bent pages: the flattening error of the page as Auto leaves it, and as On flattens it, against
    // the corners alone (target: median at most 0.25%).
    let spec = SceneSpec { count: n, seed: 9002, curved: true };
    let (mut auto, mut on, mut corners_only) = (Vec::new(), Vec::new(), Vec::new());
    let mut auto_flattened = 0;
    for i in 0..n {
        let scene = generate(&spec, i).unwrap();
        if scene.truth.page_grid.is_none() {
            continue;
        }
        let size = (scene.photo.width(), scene.photo.height());
        let (page, engine, dir) = run(&scene, FlattenMode::Auto);
        auto_flattened += u32::from(flattened(&page));
        auto.push(flattening_error(&found_grid(&page, &engine, size, 16), &scene));
        let _ = std::fs::remove_dir_all(dir);
        let (page, engine, dir) = run(&scene, FlattenMode::On);
        on.push(flattening_error(&found_grid(&page, &engine, size, 16), &scene));
        let kind: String = scene.truth.description.split(';').next().unwrap_or("").chars().take(24).collect();
        eprintln!(
            "  #{i:<3} {kind:<24} auto {:.2}% on {:.2}% corners {:.2}%",
            100.0 * auto[auto.len() - 1],
            100.0 * on[on.len() - 1],
            100.0 * flattening_error(&found_grid(&Page { sheet: None, ..page.clone() }, &engine, size, 16), &scene)
        );
        let _ = std::fs::remove_dir_all(dir);
        let (page, engine, dir) = run(&scene, FlattenMode::Off);
        corners_only.push(flattening_error(&found_grid(&page, &engine, size, 16), &scene));
        let _ = std::fs::remove_dir_all(dir);
    }
    eprintln!("GATE bent pages, {} scenes (the photos carry no EXIF focal length):", auto.len());
    eprintln!("  Auto flattened {auto_flattened} of them");
    eprintln!("  median flattening error, Auto:         {:.3}% (target at most 0.35% without a focal length)", 100.0 * median(auto));
    eprintln!("  median flattening error, always on:    {:.3}%", 100.0 * median(on));
    eprintln!("  median flattening error, corners only: {:.3}%", 100.0 * median(corners_only));
}
