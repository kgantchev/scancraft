//! Scoring a data set against what the pipeline found, and the report of §10.3 (`scancraft-cli
//! eval` writes it as JSON, attached to each release).
//!
//! The pipeline's answers come as a list of [`Found`] records, one per photo, so the same scoring
//! serves the command-line tool, the engine and the tests. A photo with no record, or a record with
//! no corners, counts as "not found" and a Jaccard index of 0 (SmartDoc's protocol).

use crate::datasets::{Dataset, Sample, reference::BENT_CATEGORIES};
use crate::metrics::{
    binarization_f, character_error_rate, corner_error, distance, jaccard, mean, median, no_touch_rate, page_diagonal, percentile, ratio_error,
};
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// What the pipeline found in one photo. Everything but the id is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Found {
    /// The sample's id: the frame's path, the file's stem or the manifest's file name.
    pub id: String,
    /// The page's corners, TL, TR, BR, BL; `null` when detection failed.
    pub corners: Option<[[f64; 2]; 4]>,
    pub ratio: Option<f64>,
    /// Where the pipeline puts each point of the truth's page grid, same rows and columns.
    pub grid: Option<Vec<Vec<[f64; 2]>>>,
    pub text: Option<String>,
    /// A PNG of the found ink mask (255 = ink) in the same pixels as the truth's.
    pub ink: Option<String>,
    /// Whether Auto flattened the page.
    pub flattened: Option<bool>,
    /// A spread's spine, top and bottom.
    pub spine: Option<[[f64; 2]; 2]>,
    /// A29's line straightness at 300 DPI, on the flattened page and on the page flattened by its corners alone.
    pub straightness: Option<Straightness>,
    /// The photo's size, when its file can't be read here.
    pub size: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Straightness {
    pub flat: f64,
    pub corners: f64,
}

/// The §10.3 figures for one photo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhotoMetrics {
    pub id: String,
    pub category: Option<String>,
    pub found: bool,
    pub jaccard: Option<f64>,
    /// A fraction of the photo's diagonal.
    pub corner_error: Option<f64>,
    pub ratio_error: Option<f64>,
    /// A fraction of the page's diagonal.
    pub flattening_error: Option<f64>,
    pub character_error_rate: Option<f64>,
    pub binarization_f: Option<f64>,
    pub spine_within_one_percent: Option<bool>,
    pub flattened: Option<bool>,
    /// Why a figure could not be worked out, if so.
    pub problems: Vec<String>,
}

/// Every metric of §10.3 over a set; `None` where nothing could be measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub jaccard_mean: Option<f64>,
    pub corner_error_mean: Option<f64>,
    pub corner_error_p95: Option<f64>,
    pub no_touch_rate: Option<f64>,
    pub not_found_rate: Option<f64>,
    pub flattening_error_median: Option<f64>,
    pub flattening_error_p95: Option<f64>,
    pub straightness_flat_median: Option<f64>,
    pub straightness_corners_median: Option<f64>,
    /// The median over photos of 1 − (flattened / corners alone): how much straighter flattening makes the lines.
    pub straightness_reduction_median: Option<f64>,
    pub ratio_error_median: Option<f64>,
    pub character_error_rate_median: Option<f64>,
    pub binarization_f_median: Option<f64>,
    /// The share of flat pages that Auto flattened.
    pub auto_flatten_rate: Option<f64>,
    pub spine_within_one_percent_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CategoryMetrics {
    pub photos: u32,
    pub found: u32,
    pub jaccard_mean: Option<f64>,
    pub no_touch_rate: Option<f64>,
    pub flattening_error_median: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub set: String,
    pub photos: u32,
    /// How many photos had a record from the pipeline.
    pub scored: u32,
    pub note: Option<String>,
    pub metrics: Metrics,
    pub by_category: BTreeMap<String, CategoryMetrics>,
    pub per_photo: Vec<PhotoMetrics>,
    /// Rows or files the loader skipped, with reasons (the first 100).
    pub skipped: Vec<String>,
    pub skipped_count: u32,
}

fn quad_of(c: [[f64; 2]; 4]) -> Quad {
    Quad(c.map(|[x, y]| Point::new(x, y)))
}

/// Collapses runs of whitespace, so a line break in the reference never counts as an edit.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The flattening error of a found grid against the truth's: the 95th percentile of the distances
/// as a fraction of the page's diagonal.
pub fn grid_error(found: &[Vec<[f64; 2]>], truth: &[Vec<Point>], diagonal: f64) -> Result<f64, CoreError> {
    let same = found.len() == truth.len() && found.iter().zip(truth).all(|(a, b)| a.len() == b.len());
    if truth.len() < 2 || !same {
        return Err(CoreError::Scene("the found grid does not match the truth's rows and columns".into()));
    }
    if !(diagonal > 0.0) {
        return Err(CoreError::Scene("the page has no diagonal".into()));
    }
    let mut errors = Vec::new();
    for (row_f, row_t) in found.iter().zip(truth) {
        for (f, t) in row_f.iter().zip(row_t) {
            let p = Point::new(f[0], f[1]);
            if !p.is_finite() {
                return Err(CoreError::Scene("a found grid point is not finite".into()));
            }
            errors.push(distance(p, *t));
        }
    }
    percentile(&errors, 0.95).map(|p| p / diagonal).ok_or_else(|| CoreError::Scene("no grid points".into()))
}

fn is_bent(sample: &Sample) -> bool {
    sample.grid.is_some() || sample.category.as_deref().is_some_and(|c| BENT_CATEGORIES.contains(&c))
}

/// The spine ends of the truth and the found, within 1% of the spread's width.
fn spine_ok(truth: &Sample, found: [[f64; 2]; 2]) -> Option<bool> {
    let spine = truth.spine?;
    let corners = truth.corners?;
    let width = (distance(corners.tl(), corners.tr()) + distance(corners.bl(), corners.br())) / 2.0;
    let worst = spine.iter().zip(found).map(|(a, f)| distance(*a, Point::new(f[0], f[1]))).fold(0.0, f64::max);
    (width > 0.0).then_some(worst <= 0.01 * width)
}

fn score_photo(sample: &Sample, record: Option<&Found>) -> PhotoMetrics {
    let mut m = PhotoMetrics { id: sample.id.clone(), category: sample.category.clone(), ..PhotoMetrics::default() };
    let Some(f) = record else { return m };
    m.flattened = f.flattened;
    let problem = |m: &mut PhotoMetrics, what: &str, e: CoreError| m.problems.push(format!("{what}: {e}"));
    if let (Some(found), Some(truth)) = (f.corners, sample.corners) {
        m.found = true;
        let (found, truth) = (quad_of(found), truth);
        match jaccard(&found, &truth) {
            Ok(j) => m.jaccard = Some(j),
            Err(e) => problem(&mut m, "Jaccard index", e),
        }
        match f.size.map_or_else(|| sample.size(), Ok).and_then(|size| corner_error(&found, &truth, size)) {
            Ok(e) => m.corner_error = Some(e),
            Err(e) => problem(&mut m, "corner error", e),
        }
    }
    if let (Some(a), Some(b)) = (f.ratio, sample.ratio) {
        match ratio_error(a, b) {
            Ok(e) => m.ratio_error = Some(e),
            Err(e) => problem(&mut m, "ratio error", e),
        }
    }
    if let (Some(found), Some(truth)) = (&f.grid, &sample.grid) {
        let diagonal = sample.corners.map_or_else(
            || {
                page_diagonal(&Quad([
                    truth[0][0],
                    truth[0][truth[0].len() - 1],
                    truth[truth.len() - 1][truth[0].len() - 1],
                    truth[truth.len() - 1][0],
                ]))
            },
            |c| page_diagonal(&c),
        );
        match grid_error(found, truth, diagonal) {
            Ok(e) => m.flattening_error = Some(e),
            Err(e) => problem(&mut m, "flattening error", e),
        }
    }
    if let (Some(found), Some(truth)) = (&f.text, &sample.text) {
        m.character_error_rate = Some(character_error_rate(&tidy(truth), &tidy(found)));
    }
    if let (Some(found), Some(truth)) = (&f.ink, &sample.ink) {
        let read = |p: &Path| crate::scene_io::read_ink(p);
        match (read(truth), read(Path::new(found))) {
            (Ok(t), Ok(g)) => match binarization_f(&t, &g) {
                Ok(v) => m.binarization_f = Some(v),
                Err(e) => problem(&mut m, "binarisation F-measure", e),
            },
            (Err(e), _) | (_, Err(e)) => problem(&mut m, "ink mask", e),
        }
    }
    if let Some(s) = f.spine {
        m.spine_within_one_percent = spine_ok(sample, s);
    }
    m
}

fn values(photos: &[PhotoMetrics], pick: impl Fn(&PhotoMetrics) -> Option<f64>) -> Vec<f64> {
    photos.iter().filter_map(pick).collect()
}

fn rate(hits: usize, of: usize) -> Option<f64> {
    (of > 0).then(|| hits as f64 / of as f64)
}

/// Scores `set` against `found`; `None` for no records at all gives a report that says so.
pub fn score(set: &Dataset, found: Option<&[Found]>) -> EvalReport {
    let mut report = EvalReport {
        set: set.name.clone(),
        photos: set.samples.len() as u32,
        skipped: set.skipped.clone(),
        skipped_count: set.skipped_count as u32,
        ..EvalReport::default()
    };
    let Some(found) = found else {
        report.note = Some("no results from the pipeline were given (--found), so nothing was scored".to_string());
        report.per_photo =
            set.samples.iter().map(|s| PhotoMetrics { id: s.id.clone(), category: s.category.clone(), ..PhotoMetrics::default() }).collect();
        return report;
    };
    let by_id: BTreeMap<&str, &Found> = found.iter().map(|f| (f.id.as_str(), f)).collect();
    report.scored = set.samples.iter().filter(|s| by_id.contains_key(s.id.as_str())).count() as u32;
    let per_photo: Vec<PhotoMetrics> = set.samples.iter().map(|s| score_photo(s, by_id.get(s.id.as_str()).copied())).collect();

    // Photos with true corners are the ones detection is judged on; a miss counts against it.
    let judged: Vec<usize> = set.samples.iter().enumerate().filter(|(_, s)| s.corners.is_some()).map(|(i, _)| i).collect();
    let judged_metrics: Vec<&PhotoMetrics> = judged.iter().map(|&i| &per_photo[i]).collect();
    let jaccards: Vec<f64> = judged_metrics.iter().map(|m| m.jaccard.unwrap_or(0.0)).collect();
    let corner_errors = values(&per_photo, |m| m.corner_error);
    let m = &mut report.metrics;
    m.jaccard_mean = mean(&jaccards);
    m.corner_error_mean = mean(&corner_errors);
    m.corner_error_p95 = percentile(&corner_errors, 0.95);
    // A page nothing was found for is a miss: its error counts as infinite.
    m.no_touch_rate = (!judged_metrics.is_empty())
        .then(|| no_touch_rate(&judged_metrics.iter().map(|p| p.corner_error.unwrap_or(f64::INFINITY)).collect::<Vec<_>>()));
    m.not_found_rate = rate(judged_metrics.iter().filter(|p| !p.found).count(), judged_metrics.len());
    let flat_errors = values(&per_photo, |p| p.flattening_error);
    m.flattening_error_median = median(&flat_errors);
    m.flattening_error_p95 = percentile(&flat_errors, 0.95);
    m.ratio_error_median = median(&values(&per_photo, |p| p.ratio_error));
    m.character_error_rate_median = median(&values(&per_photo, |p| p.character_error_rate));
    m.binarization_f_median = median(&values(&per_photo, |p| p.binarization_f));
    let spines: Vec<bool> = per_photo.iter().filter_map(|p| p.spine_within_one_percent).collect();
    m.spine_within_one_percent_rate = rate(spines.iter().filter(|&&ok| ok).count(), spines.len());
    // Flat pages Auto flattened, among the flat pages the pipeline said anything about.
    let flat_decisions: Vec<bool> = set.samples.iter().zip(&per_photo).filter(|(s, _)| !is_bent(s)).filter_map(|(_, p)| p.flattened).collect();
    m.auto_flatten_rate = rate(flat_decisions.iter().filter(|&&f| f).count(), flat_decisions.len());
    // Straightness comes straight from the records.
    let (mut flat_px, mut corner_px, mut reductions) = (Vec::new(), Vec::new(), Vec::new());
    for s in set.samples.iter().filter_map(|s| by_id.get(s.id.as_str()).and_then(|f| f.straightness)) {
        flat_px.push(s.flat);
        corner_px.push(s.corners);
        if s.corners > 0.0 {
            reductions.push(1.0 - s.flat / s.corners);
        }
    }
    m.straightness_flat_median = median(&flat_px);
    m.straightness_corners_median = median(&corner_px);
    m.straightness_reduction_median = median(&reductions);

    // By category.
    let mut groups: BTreeMap<String, Vec<&PhotoMetrics>> = BTreeMap::new();
    for (s, p) in set.samples.iter().zip(&per_photo) {
        groups.entry(s.category.clone().unwrap_or_else(|| "(none)".to_string())).or_default().push(p);
    }
    for (name, photos) in groups {
        let judged: Vec<&&PhotoMetrics> = photos.iter().collect();
        report.by_category.insert(
            name,
            CategoryMetrics {
                photos: photos.len() as u32,
                found: photos.iter().filter(|p| p.found).count() as u32,
                jaccard_mean: mean(&judged.iter().map(|p| p.jaccard.unwrap_or(0.0)).collect::<Vec<_>>()),
                no_touch_rate: rate(photos.iter().filter(|p| p.corner_error.is_some_and(|e| e <= 0.005)).count(), photos.len()),
                flattening_error_median: median(&photos.iter().filter_map(|p| p.flattening_error).collect::<Vec<_>>()),
            },
        );
    }
    report.per_photo = per_photo;
    report
}

/// Reads the pipeline's records: a JSON array of [`Found`], or `{"photos": [...]}`.
pub fn read_found(path: &Path) -> Result<Vec<Found>, CoreError> {
    let bytes = crate::datasets::read_capped(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| CoreError::Scene(format!("{}: {e}", path.display())))?;
    let list = value.get("photos").unwrap_or(&value).clone();
    serde_json::from_value(list).map_err(|e| CoreError::Scene(format!("{}: {e}", path.display())))
}

/// Writes the report as pretty JSON.
pub fn write_report(report: &EvalReport, path: &Path) -> Result<(), CoreError> {
    let json = serde_json::to_string_pretty(report).map_err(|e| CoreError::Scene(e.to_string()))?;
    std::fs::write(path, json + "\n").map_err(|e| CoreError::Scene(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneSpec;
    use crate::datasets::synthetic;
    use crate::scene_io::generate_set;

    fn square_sample(id: &str, x: f64, category: &str) -> Sample {
        let mut s = Sample::new(id, std::path::PathBuf::from("missing.png"));
        s.corners = Some(Quad([Point::new(x, 10.0), Point::new(x + 80.0, 10.0), Point::new(x + 80.0, 90.0), Point::new(x, 90.0)]));
        s.category = Some(category.to_string());
        s
    }

    fn exact(s: &Sample) -> Found {
        let c = s.corners.unwrap().0;
        Found { id: s.id.clone(), corners: Some(c.map(|p| [p.x, p.y])), size: Some((100, 100)), ..Found::default() }
    }

    fn set_of(samples: Vec<Sample>) -> Dataset {
        let mut set = Dataset::new("test");
        set.samples = samples;
        set
    }

    #[test]
    fn a_perfect_pipeline_scores_perfectly() {
        let set = set_of(vec![square_sample("a", 5.0, "dark desk"), square_sample("b", 8.0, "wood")]);
        let found: Vec<Found> = set.samples.iter().map(exact).collect();
        let r = score(&set, Some(&found));
        assert_eq!((r.photos, r.scored), (2, 2));
        assert_eq!(r.metrics.jaccard_mean, Some(1.0));
        assert_eq!(r.metrics.corner_error_mean, Some(0.0));
        assert_eq!(r.metrics.no_touch_rate, Some(1.0));
        assert_eq!(r.metrics.not_found_rate, Some(0.0));
        assert_eq!(r.by_category["wood"].photos, 1);
    }

    #[test]
    fn a_missing_or_failed_detection_counts_against_the_pipeline() {
        let set = set_of(vec![square_sample("a", 5.0, "dark desk"), square_sample("b", 8.0, "wood"), square_sample("c", 9.0, "wood")]);
        let mut failed = exact(&set.samples[1]);
        failed.corners = None;
        let found = vec![exact(&set.samples[0]), failed];
        let r = score(&set, Some(&found));
        // One found exactly, one failed, one never answered: Jaccard 1, 0, 0.
        assert!((r.metrics.jaccard_mean.unwrap() - 1.0 / 3.0).abs() < 1e-12);
        assert!((r.metrics.not_found_rate.unwrap() - 2.0 / 3.0).abs() < 1e-12);
        assert!((r.metrics.no_touch_rate.unwrap() - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(r.scored, 2);
        assert!(!r.per_photo[1].found && !r.per_photo[2].found);
    }

    #[test]
    fn corner_errors_and_the_no_touch_rate_follow_the_definitions() {
        let set = set_of(vec![square_sample("a", 5.0, "x"), square_sample("b", 5.0, "x")]);
        let mut near = exact(&set.samples[0]);
        near.corners.as_mut().unwrap()[2][0] += 0.5;
        let mut far = exact(&set.samples[1]);
        far.corners.as_mut().unwrap()[0][1] += 3.0;
        let r = score(&set, Some(&[near, far]));
        let diagonal = 100f64.hypot(100.0);
        assert!((r.per_photo[0].corner_error.unwrap() - 0.5 / diagonal).abs() < 1e-12);
        assert!((r.per_photo[1].corner_error.unwrap() - 3.0 / diagonal).abs() < 1e-12);
        // 0.5 / 141 = 0.35% is within half a percent; 3 / 141 = 2.1% is not.
        assert_eq!(r.metrics.no_touch_rate, Some(0.5));
        assert!(r.metrics.corner_error_p95.unwrap() > 0.0);
    }

    #[test]
    fn flattening_error_ratio_text_ink_spine_and_straightness_are_scored() {
        let mut s = square_sample("p", 10.0, "open book page");
        let grid: Vec<Vec<Point>> =
            (0..5).map(|j| (0..4).map(|i| Point::new(10.0 + 80.0 * i as f64 / 3.0, 10.0 + 80.0 * j as f64 / 4.0)).collect()).collect();
        s.grid = Some(grid.clone());
        s.ratio = Some(0.7);
        s.text = Some("the quick\nbrown fox".to_string());
        s.spine = Some([Point::new(50.0, 10.0), Point::new(50.0, 90.0)]);
        let mut f = exact(&s);
        f.grid = Some(grid.iter().map(|r| r.iter().map(|p| [p.x + 1.6, p.y]).collect()).collect());
        f.ratio = Some(0.707);
        f.text = Some("the quick brown fax".to_string());
        f.spine = Some([[50.5, 10.0], [50.0, 90.0]]);
        f.straightness = Some(Straightness { flat: 0.5, corners: 2.0 });
        let r = score(&set_of(vec![s]), Some(&[f]));
        let p = &r.per_photo[0];
        // 1.6 px of the page's diagonal of 80√2 = 113.1 px.
        assert!((p.flattening_error.unwrap() - 1.6 / (80.0f64 * 2f64.sqrt())).abs() < 1e-9, "{:?}", p.flattening_error);
        assert!((p.ratio_error.unwrap() - 0.01).abs() < 1e-9);
        assert!((p.character_error_rate.unwrap() - 1.0 / 19.0).abs() < 1e-9);
        assert_eq!(p.spine_within_one_percent, Some(true));
        assert_eq!(r.metrics.straightness_reduction_median, Some(0.75));
        assert_eq!(r.metrics.spine_within_one_percent_rate, Some(1.0));
        // A grid of the wrong shape is a problem, not a panic.
        let mut bad = exact(&set_of(vec![]).samples.first().cloned().unwrap_or_else(|| square_sample("p", 10.0, "x")));
        bad.id = "p".into();
        bad.grid = Some(vec![vec![[0.0, 0.0]; 2]; 2]);
        let mut with_grid = square_sample("p", 10.0, "x");
        with_grid.grid = Some(grid);
        let r = score(&set_of(vec![with_grid]), Some(&[bad]));
        assert!(r.per_photo[0].flattening_error.is_none() && !r.per_photo[0].problems.is_empty());
    }

    #[test]
    fn auto_flatten_counts_only_flat_pages() {
        let flat = square_sample("flat", 1.0, "dark desk");
        let bent = square_sample("bent", 1.0, "open book page");
        let (mut a, mut b) = (exact(&flat), exact(&bent));
        a.flattened = Some(false);
        b.flattened = Some(true);
        let r = score(&set_of(vec![flat, bent]), Some(&[a, b]));
        assert_eq!(r.metrics.auto_flatten_rate, Some(0.0));
    }

    #[test]
    fn the_report_of_an_empty_set_is_not_a_division_by_zero() {
        for r in [score(&set_of(vec![]), Some(&[])), score(&set_of(vec![]), None), score(&set_of(vec![square_sample("a", 1.0, "x")]), Some(&[]))] {
            let json = serde_json::to_string(&r).unwrap();
            assert!(!json.contains("NaN") && !json.contains("inf"), "{json}");
        }
        let r = score(&set_of(vec![]), Some(&[]));
        assert_eq!(r.metrics, Metrics::default());
        // Without records nothing is scored and the report says why.
        let r = score(&set_of(vec![square_sample("a", 1.0, "x")]), None);
        assert!(r.note.is_some() && r.scored == 0 && r.metrics == Metrics::default());
    }

    #[test]
    fn the_report_holds_every_metric_of_section_10_3_and_round_trips() {
        let set = set_of(vec![square_sample("a", 5.0, "dark desk")]);
        let r = score(&set, Some(&[exact(&set.samples[0])]));
        let json = serde_json::to_value(&r).unwrap();
        for key in [
            "jaccard_mean",
            "corner_error_mean",
            "corner_error_p95",
            "no_touch_rate",
            "not_found_rate",
            "flattening_error_median",
            "straightness_flat_median",
            "straightness_corners_median",
            "straightness_reduction_median",
            "ratio_error_median",
            "character_error_rate_median",
            "binarization_f_median",
            "auto_flatten_rate",
            "spine_within_one_percent_rate",
        ] {
            assert!(json["metrics"].get(key).is_some(), "{key}");
        }
        let back: EvalReport = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn synthetic_scenes_score_perfectly_against_their_own_truth() {
        let dir = std::env::temp_dir().join(format!("scancraft-eval-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        generate_set(&SceneSpec { count: 3, seed: 5, curved: true }, &dir).unwrap();
        let set = synthetic::load(&dir).unwrap();
        // Pretend the pipeline found the truth exactly, read the page's ink and text, and flattened.
        let found: Vec<Found> = set
            .samples
            .iter()
            .map(|s| Found {
                id: s.id.clone(),
                corners: s.corners.map(|q| q.0.map(|p| [p.x, p.y])),
                ratio: s.ratio,
                grid: s.grid.as_ref().map(|g| g.iter().map(|r| r.iter().map(|p| [p.x, p.y]).collect()).collect()),
                text: s.text.clone(),
                ink: s.ink.as_ref().map(|p| p.display().to_string()),
                flattened: Some(true),
                spine: s.spine.map(|sp| sp.map(|p| [p.x, p.y])),
                ..Found::default()
            })
            .collect();
        let r = score(&set, Some(&found));
        assert!((r.metrics.jaccard_mean.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(r.metrics.flattening_error_median, Some(0.0));
        assert_eq!(r.metrics.character_error_rate_median, Some(0.0));
        assert_eq!(r.metrics.binarization_f_median, Some(1.0));
        assert_eq!(r.metrics.no_touch_rate, Some(1.0));
        // Round trip through the files the CLI uses.
        let path = dir.join("report.json");
        write_report(&r, &path).unwrap();
        let back: EvalReport = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, r);
        std::fs::write(dir.join("found.json"), serde_json::to_string(&found).unwrap()).unwrap();
        assert_eq!(read_found(&dir.join("found.json")).unwrap(), found);
        std::fs::write(dir.join("wrapped.json"), serde_json::json!({"photos": found}).to_string()).unwrap();
        assert_eq!(read_found(&dir.join("wrapped.json")).unwrap().len(), 3);
        std::fs::write(dir.join("junk.json"), "[1, 2]").unwrap();
        assert!(read_found(&dir.join("junk.json")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
