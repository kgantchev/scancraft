//! Where the classical path stands on the synthetic set, by background and damage (build plan WP12:
//! A9's constants are tuned against these numbers; specification §10.4's synthetic gates).
//!
//! Run it with
//! `cargo test --release -p scancraft-detect --test gates -- --ignored --nocapture`.
//! `SCANCRAFT_GATE_COUNT` sets the number of scenes (default 60) and `SCANCRAFT_GATE_SEED` the seed.

use scancraft_core::geom::Quad;
use scancraft_core::recipe::Detector;
use scancraft_detect::contours::contour_candidates;
use scancraft_detect::detect::{detect_page, work_image};
use scancraft_detect::edges::colour_edges;
use scancraft_detect::lines::line_candidates;
use scancraft_raster::colour::to_f32;
use scancraft_synth::generate::draw_params;
use scancraft_synth::metrics::corner_error;
use scancraft_synth::{SceneSpec, generate};
use std::collections::BTreeMap;

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[derive(Default)]
struct Tally {
    scenes: u32,
    /// Some classical candidate is within 0.5% of the truth.
    candidate_exists: u32,
    /// The chosen outline is within 0.5%.
    no_touch: u32,
    /// Nothing was good enough.
    not_found: u32,
}

#[test]
#[ignore = "a measurement; run in a release build"]
fn the_classical_path_by_background_and_damage() {
    let spec = SceneSpec { count: env("SCANCRAFT_GATE_COUNT", 60) as u32, seed: env("SCANCRAFT_GATE_SEED", 7), curved: false };
    let mut by: BTreeMap<String, Tally> = BTreeMap::new();
    for index in 0..spec.count {
        let Ok(p) = draw_params(&spec, index) else { continue };
        let Ok(scene) = generate(&spec, index) else { continue };
        let size = (scene.photo.width(), scene.photo.height());
        let truth = Quad::canonical(scene.truth.corners.0);
        let work = work_image(&scene.photo).unwrap();
        let k = f64::from(size.0) / f64::from(work.width());
        let maps = colour_edges(&to_f32(&work).unwrap()).unwrap();
        let mut candidates: Vec<(Detector, Quad)> = contour_candidates(&maps).unwrap().into_iter().map(|q| (Detector::Contours, q)).collect();
        candidates.extend(line_candidates(&maps).unwrap().into_iter().map(|q| (Detector::Lines, q)));
        let exists = candidates.iter().any(|(_, q)| {
            let up = Quad(q.0.map(|c| scancraft_core::geom::Point::new(c.x * k, c.y * k)));
            corner_error(&up, &truth, size).is_ok_and(|e| e <= 0.005 * 4.0)
        });
        let d = detect_page(&scene.photo, None, 1000 + u64::from(index)).unwrap();
        let found = !matches!(d.origin, scancraft_core::recipe::CornerOrigin::NotFound);
        let touch = found && corner_error(&d.refined.corners, &truth, size).is_ok_and(|e| e <= 0.005);
        for key in [format!("{:?}", p.background), format!("damage {:?}", p.damage), "all".to_string()] {
            let t = by.entry(key).or_default();
            t.scenes += 1;
            t.candidate_exists += u32::from(exists);
            t.no_touch += u32::from(touch);
            t.not_found += u32::from(!found);
        }
        if !touch {
            eprintln!("miss {index:>3} {:?} {:?} candidate_exists={exists} found={found}", p.background, p.damage);
        }
    }
    eprintln!("{:<22}{:>7}{:>14}{:>11}{:>11}", "", "scenes", "cand<=2%", "no-touch", "not found");
    for (name, t) in &by {
        eprintln!("{name:<22}{:>7}{:>14}{:>11}{:>11}", t.scenes, t.candidate_exists, t.no_touch, t.not_found);
    }
}
