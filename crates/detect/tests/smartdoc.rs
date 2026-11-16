//! The SmartDoc 2015 detection gate (specification §10.4): the mean Jaccard index of the found
//! page against the truth, over a public dataset of photographed documents.
//!
//! The dataset is not in the repository: it is 1 GB of frames that show people's documents, and
//! the licence (CC BY 4.0) allows it to be kept, but not in git. Point `SCANCRAFT_SMARTDOC_DIR` at
//! the folder that holds `frames/metadata.csv.gz` and the frames beside it:
//!
//! ```text
//! SCANCRAFT_SMARTDOC_DIR=/path/to/smartdoc15 cargo test -p scancraft-detect --test smartdoc
//! ```
//!
//! The full set is 24 889 frames and takes minutes. `SCANCRAFT_SMARTDOC_FRAMES` caps how many are
//! used, so a quick run can be made; the gate's number then belongs to that subset and says so.

use rayon::prelude::*;
use scancraft_core::determinism::stable_hash;
use scancraft_core::image::Rgb8;
use scancraft_detect::detect::detect_page;
use scancraft_synth::datasets::smartdoc;
use scancraft_synth::eval::{Found, score};
use std::path::PathBuf;

/// The gate's target for the classical path alone (specification §10.4).
const CLASSICAL_TARGET: f64 = 0.93;
/// The least the test accepts: a little under what the classical path measures on the whole set
/// (0.682 on 24 889 frames; 0.708 on a 608-frame subset), so that a change that makes detection
/// worse turns this red. The target above is not met yet; the test says so and does not fail on it.
const FLOOR: f64 = 0.60;

/// Where the dataset is, and how many frames to use.
fn setup() -> Option<(PathBuf, Option<usize>)> {
    let root = std::env::var_os("SCANCRAFT_SMARTDOC_DIR").map(PathBuf::from)?;
    if !root.join("frames").is_dir() {
        eprintln!("skipped: {root:?} has no frames folder");
        return None;
    }
    let limit = std::env::var("SCANCRAFT_SMARTDOC_FRAMES").ok().and_then(|v| v.parse::<usize>().ok());
    Some((root, limit))
}

/// The photo's pixels, decoded from its JPEG bytes.
fn decode(bytes: &[u8]) -> Result<Rgb8, String> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    let rgb = image.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let (pixels, _) = rgb.as_raw().as_chunks::<3>();
    Rgb8::from_vec(w, h, pixels.to_vec()).map_err(|e| e.to_string())
}

/// One photo of the set, found.
fn find(sample: &scancraft_synth::datasets::Sample) -> Result<Found, String> {
    let bytes = std::fs::read(&sample.image).map_err(|e| e.to_string())?;
    let photo = decode(&bytes)?;
    let size = (photo.width(), photo.height());
    // The seed is fixed by the photo's name, so the same frame always gives the same corners.
    let seed = stable_hash(sample.id.as_bytes());
    let found = detect_page(&photo, None, seed).map_err(|e| e.to_string())?;
    let corners = found.refined.corners.0.map(|p| [p.x, p.y]);
    Ok(Found { id: sample.id.clone(), corners: Some(corners), size: Some(size), ..Found::default() })
}

/// A line of the report, for the test's output.
fn line(report: &scancraft_synth::eval::EvalReport, label: &str) -> String {
    let m = &report.metrics;
    format!(
        "{label}: {} photos, {} scored | mean Jaccard {} | corner error mean {} p95 {} | not found {} | no-touch {}",
        report.photos,
        report.scored,
        fmt(m.jaccard_mean),
        fmt(m.corner_error_mean),
        fmt(m.corner_error_p95),
        fmt(m.not_found_rate),
        fmt(m.no_touch_rate),
    )
}

fn fmt(v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{v:.4}"),
        None => "—".to_string(),
    }
}

#[test]
fn the_smartdoc_detection_gate() {
    let Some((root, limit)) = setup() else {
        eprintln!("skipped: set SCANCRAFT_SMARTDOC_DIR to the SmartDoc 2015 folder (with frames/ inside)");
        return;
    };
    let set = smartdoc::load(&root).expect("the SmartDoc set loads");
    assert!(!set.samples.is_empty(), "the set holds no frames");
    // Every nth frame when a limit is given, so the subset still covers every background.
    let stride = match limit {
        Some(limit) if limit < set.samples.len() => (set.samples.len() / limit).max(1),
        _ => 1,
    };
    let samples: Vec<&scancraft_synth::datasets::Sample> = set.samples.iter().step_by(stride).collect();
    let used = samples.len();
    eprintln!("SmartDoc: {used} of {} frames (stride {stride})", set.samples.len());
    // The report is over the frames that were run, and not over the whole set: a frame that was
    // not run is not a frame the detector missed, and counting it as one says nothing about the
    // detector.
    let mut subset = scancraft_synth::datasets::Dataset::new(&set.name);
    subset.samples = samples.iter().map(|s| (*s).clone()).collect();
    subset.skipped = set.skipped.clone();
    subset.skipped_count = set.skipped_count;

    let started = std::time::Instant::now();
    let found: Vec<Result<Found, String>> = samples.par_iter().map(|s| find(s)).collect();
    let seconds = started.elapsed().as_secs_f64();
    let failures: Vec<String> = found.iter().filter_map(|f| f.as_ref().err().cloned()).collect();
    assert!(failures.is_empty(), "{} frames could not be read: {:?}", failures.len(), failures.first());
    let found: Vec<Found> = found.into_iter().filter_map(Result::ok).collect();

    let report = score(&subset, Some(&found));
    eprintln!("{}", line(&report, "SmartDoc classical"));
    eprintln!("detection took {seconds:.1} s for {used} frames");
    for (name, category) in &report.by_category {
        eprintln!("  {name}: {} photos, {} found, mean Jaccard {}", category.photos, category.found, fmt(category.jaccard_mean));
    }

    // The results are written beside the dataset, so the command-line scorer can produce the same
    // report from them. It scores them against the whole set, so its report is only the same as
    // this one when every frame was run.
    let out = root.join("found-classical.json");
    if let Ok(text) = serde_json::to_string_pretty(&found) {
        let _ = std::fs::write(&out, text);
        eprintln!("wrote {}", out.display());
    }
    let report_out = root.join("report-classical.json");
    if let Ok(text) = serde_json::to_string_pretty(&report) {
        let _ = std::fs::write(&report_out, text);
    }

    let jaccard = report.metrics.jaccard_mean.expect("the report has a mean Jaccard index");
    assert_eq!(report.scored, used as u32, "every frame that was run has a record");
    // The specification's target, said plainly, whether or not it is met.
    let verdict = if jaccard >= CLASSICAL_TARGET { "MET" } else { "NOT MET" };
    eprintln!("specification target {CLASSICAL_TARGET} (§10.4, classical detection only): {verdict}, measured {jaccard:.4} on {used} frames");
    assert!(jaccard >= FLOOR, "the mean Jaccard index fell to {jaccard:.4}, under the floor of {FLOOR} ({used} frames)");
}
