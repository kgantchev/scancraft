//! The metrics of §10.3. Owner: WP4.

use crate::geom::{apply_h, homography_from_unit_square, invert3, polygon_area, polygon_intersection_area};
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::BitPlane;
use scancraft_core::map::PageMap;
use scancraft_core::scene::SceneTruth;

fn scene_error(what: impl Into<String>) -> CoreError {
    CoreError::Scene(what.into())
}

/// A point helper kept here so metrics don't depend on the pipeline's geometry crate.
pub fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Overlap area over union area, by SmartDoc's protocol (ICDAR 2015, §II.B): the true page and the
/// found page are put through the perspective correction that makes the true page a rectangle,
/// and the Jaccard index is taken on the corrected pair. The index isn't invariant under a
/// projective map, so taking it in photo pixels would give different numbers from the published
/// ones, most of all on the tilted pages.
pub fn jaccard(found: &Quad, truth: &Quad) -> Result<f64, CoreError> {
    if !found.is_finite() || !truth.is_finite() {
        return Err(scene_error("a quadrilateral has corners that are not finite"));
    }
    let long = truth.0.iter().map(|p| p.x.abs().max(p.y.abs())).fold(1.0, f64::max);
    let h = homography_from_unit_square(truth, long).ok_or_else(|| scene_error("the true page is degenerate"))?;
    let inverse = invert3(&h).ok_or_else(|| scene_error("the true page is degenerate"))?;
    let mut corrected = Vec::with_capacity(4);
    for p in found.0 {
        let q = apply_h(&inverse, p.x, p.y).ok_or_else(|| scene_error("a found corner has no place in the corrected page"))?;
        corrected.push(q);
    }
    let square = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
    let intersection = polygon_intersection_area(&corrected, &square);
    let union = polygon_area(&corrected) + 1.0 - intersection;
    Ok(if union > 0.0 { (intersection / union).clamp(0.0, 1.0) } else { 0.0 })
}

/// The largest corner distance as a fraction of the photo's diagonal. Both quads are in the
/// canonical order TL, TR, BR, BL, and corners are paired by index.
pub fn corner_error(found: &Quad, truth: &Quad, photo: (u32, u32)) -> Result<f64, CoreError> {
    if !found.is_finite() || !truth.is_finite() {
        return Err(scene_error("a quadrilateral has corners that are not finite"));
    }
    if photo.0 == 0 || photo.1 == 0 {
        return Err(scene_error("the photo has no pixels"));
    }
    let worst = found.0.iter().zip(&truth.0).map(|(a, b)| distance(*a, *b)).fold(0.0, f64::max);
    Ok(worst / f64::from(photo.0).hypot(f64::from(photo.1)))
}

/// The q-quantile (0..=1) by §7.7: a 1024-bin histogram over the values' range, giving the lower
/// edge of the first bin whose cumulative count reaches q of the values. `None` for no values.
pub fn percentile(values: &[f64], q: f64) -> Option<f64> {
    const BINS: usize = 1024;
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() || !q.is_finite() {
        return None;
    }
    let (min, max) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if max <= min {
        return Some(min);
    }
    let mut counts = [0u64; BINS];
    for &v in &finite {
        let bin = (((v - min) / (max - min)) * BINS as f64).floor().clamp(0.0, (BINS - 1) as f64) as usize;
        counts[bin] += 1;
    }
    let need = (q.clamp(0.0, 1.0) * finite.len() as f64).ceil().max(1.0) as u64;
    let mut cumulative = 0u64;
    for (bin, count) in counts.iter().enumerate() {
        cumulative += count;
        if cumulative >= need {
            return Some(min + (max - min) * bin as f64 / BINS as f64);
        }
    }
    Some(max)
}

/// The median of the finite values, or `None` for none (the mean of the two middle ones for an
/// even count).
pub fn median(values: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) { (v[mid - 1] + v[mid]) / 2.0 } else { v[mid] })
}

/// The mean of the finite values, or `None` for none.
pub fn mean(values: &[f64]) -> Option<f64> {
    let v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// The flattening error: the 95th percentile over the truth's page grid of the distance between
/// where `map` puts each page point and where it truly is, as a fraction of the page's diagonal.
pub fn flattening_error(map: &dyn PageMap, truth: &SceneTruth) -> Result<f64, CoreError> {
    let grid = truth.page_grid.as_ref().ok_or_else(|| scene_error("this scene has no page grid"))?;
    let n = grid.len();
    if n < 2 || grid.iter().any(|row| row.len() != n) {
        return Err(scene_error("the page grid is not square"));
    }
    let mut errors = Vec::with_capacity(n * n);
    for (j, row) in grid.iter().enumerate() {
        for (i, &truth_point) in row.iter().enumerate() {
            let (u, v) = (i as f64 / (n - 1) as f64, j as f64 / (n - 1) as f64);
            let found = map.map(u, v).ok_or_else(|| scene_error(format!("the mapping is undefined at page point ({u:.3}, {v:.3})")))?;
            errors.push(distance(found, truth_point));
        }
    }
    let diagonal = page_diagonal(&truth.corners);
    if !(diagonal > 0.0) {
        return Err(scene_error("the page has no diagonal"));
    }
    percentile(&errors, 0.95).map(|p| p / diagonal).ok_or_else(|| scene_error("no grid points"))
}

/// The page's diagonal in the photo: the longer of the two corner-to-corner distances.
pub fn page_diagonal(corners: &Quad) -> f64 {
    distance(corners.tl(), corners.br()).max(distance(corners.tr(), corners.bl()))
}

/// The share of photos whose corner error (a fraction of the diagonal) is at most 0.5%.
pub fn no_touch_rate(errors: &[f64]) -> f64 {
    if errors.is_empty() {
        return 0.0;
    }
    errors.iter().filter(|e| **e <= 0.005).count() as f64 / errors.len() as f64
}

/// The relative error of a page's ratio.
pub fn ratio_error(found: f64, truth: f64) -> Result<f64, CoreError> {
    if !truth.is_finite() || truth <= 0.0 || !found.is_finite() {
        return Err(scene_error("a ratio must be positive and finite"));
    }
    Ok(((found - truth) / truth).abs())
}

/// A29's straightness: the RMS distance of the lines' points from their own line, over all the
/// points of all the lines. Each line is measured against its own least-squares straight line, so
/// the figure doesn't depend on which way the text runs; for horizontal lines that is the distance
/// from the line's mean height, up to the line's slope. Lines of fewer than 3 points are skipped.
pub fn line_straightness(lines: &[Vec<Point>]) -> f64 {
    let (mut sum, mut count) = (0.0, 0usize);
    for line in lines.iter().filter(|l| l.len() >= 3 && l.iter().all(|p| p.is_finite())) {
        let n = line.len() as f64;
        let (cx, cy) = (line.iter().map(|p| p.x).sum::<f64>() / n, line.iter().map(|p| p.y).sum::<f64>() / n);
        let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
        for p in line {
            let (dx, dy) = (p.x - cx, p.y - cy);
            sxx += dx * dx;
            sxy += dx * dy;
            syy += dy * dy;
        }
        // The line's direction is the principal axis of the points; the distance from it is
        // the smaller eigenvalue's share of the scatter.
        let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let (ux, uy) = (angle.cos(), angle.sin());
        for p in line {
            let across = -(p.x - cx) * uy + (p.y - cy) * ux;
            sum += across * across;
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { (sum / count as f64).sqrt() }
}

/// The character error rate: the Levenshtein distance over the reference's length in characters.
/// An empty reference gives 0 when the hypothesis is empty too, and 1 otherwise.
pub fn character_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (reference.chars().collect(), hypothesis.chars().collect());
    if a.is_empty() {
        return if b.is_empty() { 0.0 } else { 1.0 };
    }
    // Two rolling rows over the shorter string.
    let (long, short) = if a.len() >= b.len() { (&a, &b) } else { (&b, &a) };
    let mut previous: Vec<usize> = (0..=short.len()).collect();
    let mut current = vec![0usize; short.len() + 1];
    for (i, &c) in long.iter().enumerate() {
        current[0] = i + 1;
        for (j, &d) in short.iter().enumerate() {
            let substitute = previous[j] + usize::from(c != d);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[short.len()] as f64 / a.len() as f64
}

/// The binarisation F-measure between a true ink mask and a found one of the same size. Two masks
/// with no ink agree completely (1); no true ink and some found ink is 0.
pub fn binarization_f(truth: &BitPlane, found: &BitPlane) -> Result<f64, CoreError> {
    if truth.width() != found.width() || truth.height() != found.height() {
        return Err(scene_error("the two ink masks differ in size"));
    }
    let (mut tp, mut fp, mut fneg) = (0u64, 0u64, 0u64);
    for y in 0..truth.height() {
        for x in 0..truth.width() {
            match (truth.get(x, y).unwrap_or(false), found.get(x, y).unwrap_or(false)) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (true, false) => fneg += 1,
                (false, false) => {}
            }
        }
    }
    if tp + fp + fneg == 0 {
        return Ok(1.0);
    }
    if tp == 0 {
        return Ok(0.0);
    }
    let (p, r) = (tp as f64 / (tp + fp) as f64, tp as f64 / (tp + fneg) as f64);
    Ok(2.0 * p * r / (p + r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::recipe::BendMode;
    use scancraft_core::scene::{SCENE_FORMAT, SceneShape};

    fn quad(p: [(f64, f64); 4]) -> Quad {
        Quad(p.map(|(x, y)| Point::new(x, y)))
    }

    fn square(x: f64, y: f64, s: f64) -> Quad {
        quad([(x, y), (x + s, y), (x + s, y + s), (x, y + s)])
    }

    #[test]
    fn the_jaccard_index_of_identical_quads_is_one() {
        let q = quad([(412.0, 380.0), (3610.2, 455.7), (3301.9, 2840.4), (611.3, 2702.8)]);
        assert!((jaccard(&q, &q).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_jaccard_index_matches_smartdocs_protocol() {
        // A found page covering the left half of an axis-aligned true page: 0.5 of the union.
        let truth = square(0.0, 0.0, 100.0);
        let found = quad([(0.0, 0.0), (50.0, 0.0), (50.0, 100.0), (0.0, 100.0)]);
        assert!((jaccard(&found, &truth).unwrap() - 0.5).abs() < 1e-9);
        // Disjoint, and a found page twice the true one in each direction: 1/4.
        assert_eq!(jaccard(&square(200.0, 200.0, 10.0), &truth).unwrap(), 0.0);
        let double = square(0.0, 0.0, 200.0);
        assert!((jaccard(&double, &truth).unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn the_jaccard_index_is_not_the_raw_overlap() {
        // A tilted page: a trapezoid, far side shorter. A found quad covering its near half.
        let truth = quad([(30.0, 0.0), (70.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        // The page's own coordinates v = 0.5 are not at the middle of the photo: a perspective
        // trapezoid's "half way up" line sits at y = 100·(…); take the line the corrected page
        // calls half way, and compare with the overlap measured in photo pixels.
        let h = homography_from_unit_square(&truth, 100.0).unwrap();
        let (left, right) = (apply_h(&h, 0.0, 0.5).unwrap(), apply_h(&h, 1.0, 0.5).unwrap());
        let found = quad([(truth.0[0].x, truth.0[0].y), (truth.0[1].x, truth.0[1].y), (right.x, right.y), (left.x, left.y)]);
        // Half of the page by its own coordinates: exactly 0.5 after the correction.
        let corrected = jaccard(&found, &truth).unwrap();
        assert!((corrected - 0.5).abs() < 1e-9, "{corrected}");
        // In photo pixels the same two quads overlap by a different share.
        let raw = polygon_intersection_area(&found.0, &truth.0) / (polygon_area(&truth.0));
        assert!((raw - corrected).abs() > 0.01, "raw {raw} corrected {corrected}");
        // A degenerate or non-finite quad is an error.
        assert!(jaccard(&found, &quad([(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)])).is_err());
        assert!(jaccard(&quad([(f64::NAN, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), &truth).is_err());
    }

    #[test]
    fn the_corner_error_is_the_largest_corner_distance_over_the_diagonal() {
        let truth = square(10.0, 10.0, 80.0);
        let mut found = truth;
        found.0[2] = Point::new(found.0[2].x + 6.0, found.0[2].y + 8.0);
        let e = corner_error(&found, &truth, (100, 100)).unwrap();
        assert!((e - 10.0 / 20000f64.sqrt()).abs() < 1e-12, "{e}");
        assert_eq!(corner_error(&truth, &truth, (100, 100)).unwrap(), 0.0);
        assert!(corner_error(&truth, &truth, (0, 100)).is_err());
        assert!(corner_error(&quad([(f64::INFINITY, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), &truth, (10, 10)).is_err());
    }

    #[test]
    fn the_no_touch_rate_counts_the_pages_within_half_a_percent() {
        assert_eq!(no_touch_rate(&[0.001, 0.0049, 0.0051, 0.02]), 0.5);
        assert_eq!(no_touch_rate(&[]), 0.0);
    }

    struct Offset {
        grid: Vec<Vec<Point>>,
        dx: f64,
    }

    impl PageMap for Offset {
        fn map(&self, u: f64, v: f64) -> Option<Point> {
            let n = self.grid.len();
            let (i, j) = ((u * (n - 1) as f64).round() as usize, (v * (n - 1) as f64).round() as usize);
            self.grid.get(j)?.get(i).map(|p| Point::new(p.x + self.dx, p.y))
        }
    }

    fn truth_with_grid(n: usize) -> SceneTruth {
        let grid: Vec<Vec<Point>> = (0..n).map(|j| (0..n).map(|i| Point::new(100.0 + i as f64 * 10.0, 100.0 + j as f64 * 10.0)).collect()).collect();
        let (a, b) = (grid[0][0], grid[n - 1][n - 1]);
        SceneTruth {
            format: SCENE_FORMAT,
            description: String::new(),
            size: (400, 400),
            focal: 0.8,
            corners: quad([(a.x, a.y), (b.x, a.y), (b.x, b.y), (a.x, b.y)]),
            ratio: 1.0,
            shape: SceneShape::Bent { mode: BendMode::Across, profile_turning: vec![] },
            spine: None,
            page_grid: Some(grid),
        }
    }

    #[test]
    fn the_flattening_error_of_the_exact_mapping_is_zero() {
        let truth = truth_with_grid(9);
        let exact = Offset { grid: truth.page_grid.clone().unwrap(), dx: 0.0 };
        assert_eq!(flattening_error(&exact, &truth).unwrap(), 0.0);
    }

    #[test]
    fn the_flattening_error_is_the_ninety_fifth_percentile() {
        let truth = truth_with_grid(9);
        // Every point 1.5 px off: the 95th percentile is 1.5, over the diagonal.
        let shifted = Offset { grid: truth.page_grid.clone().unwrap(), dx: 1.5 };
        let e = flattening_error(&shifted, &truth).unwrap();
        let diagonal = (80.0f64).hypot(80.0);
        assert!((e - 1.5 / diagonal).abs() < 1e-12, "{e}");
        // The §7.7 rule by hand: 100 values 0..99, the first bin to reach 95% starts at 94.something.
        let values: Vec<f64> = (0..100).map(f64::from).collect();
        let p = percentile(&values, 0.95).unwrap();
        let bin = (94.0 / 99.0 * 1024.0f64).floor();
        assert!((p - 99.0 * bin / 1024.0).abs() < 1e-9, "{p}");
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7.0, 7.0], 0.95), Some(7.0));
    }

    #[test]
    fn the_flattening_error_needs_a_page_grid() {
        let mut truth = truth_with_grid(5);
        let map = Offset { grid: truth.page_grid.clone().unwrap(), dx: 0.0 };
        truth.page_grid = None;
        assert!(matches!(flattening_error(&map, &truth), Err(CoreError::Scene(_))));
        truth.page_grid = Some(vec![vec![Point::new(0.0, 0.0); 3]; 2]);
        assert!(matches!(flattening_error(&map, &truth), Err(CoreError::Scene(_))));
    }

    #[test]
    fn the_line_straightness_is_the_rms_deviation_from_each_lines_own_line() {
        // Heights 0, 2, 4 along a straight slope: on the line, so zero; the spec's example, a
        // line whose heights wander 0, 2, 0 around a flat fit, gives the RMS of the residuals.
        let slope = vec![Point::new(0.0, 0.0), Point::new(1.0, 2.0), Point::new(2.0, 4.0)];
        assert!(line_straightness(&[slope]) < 1e-12);
        let bump = vec![Point::new(0.0, 0.0), Point::new(10.0, 3.0), Point::new(20.0, 0.0)];
        // Fit line is nearly flat; residuals are about (-1, 2, -1): RMS ≈ sqrt(2).
        let s = line_straightness(&[bump]);
        assert!((s - 2.0f64.sqrt()).abs() < 0.05, "{s}");
        // A line of any direction, and each line its own mean: two parallel flat lines at
        // different heights are perfectly straight.
        let a: Vec<Point> = (0..5).map(|i| Point::new(f64::from(i), 10.0)).collect();
        let b: Vec<Point> = (0..5).map(|i| Point::new(f64::from(i), 99.0)).collect();
        let vertical: Vec<Point> = (0..5).map(|i| Point::new(7.0, f64::from(i))).collect();
        assert!(line_straightness(&[a, b, vertical]) < 1e-9);
        assert_eq!(line_straightness(&[]), 0.0);
    }

    #[test]
    fn the_character_error_rate_counts_edits_over_the_reference_length() {
        assert!((character_error_rate("abc", "abd") - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(character_error_rate("abc", "abc"), 0.0);
        assert_eq!(character_error_rate("", ""), 0.0);
        assert_eq!(character_error_rate("", "x"), 1.0);
        assert!((character_error_rate("kitten", "sitting") - 3.0 / 6.0).abs() < 1e-12);
        assert_eq!(character_error_rate("abc", ""), 1.0);
        assert!((character_error_rate("héllo", "hello") - 0.2).abs() < 1e-12);
    }

    #[test]
    fn the_binarisation_f_measure_is_one_for_identical_masks() {
        let mut a = BitPlane::new(20, 10).unwrap();
        let mut b = BitPlane::new(20, 10).unwrap();
        for x in 2..8 {
            a.set(x, 3, true);
            b.set(x, 3, true);
        }
        assert_eq!(binarization_f(&a, &b).unwrap(), 1.0);
        let mut c = BitPlane::new(20, 10).unwrap();
        c.set(15, 8, true);
        assert_eq!(binarization_f(&a, &c).unwrap(), 0.0);
        // Half found: precision 1, recall 0.5, F = 2/3.
        let mut half = BitPlane::new(20, 10).unwrap();
        for x in 2..5 {
            half.set(x, 3, true);
        }
        assert!((binarization_f(&a, &half).unwrap() - 2.0 / 3.0).abs() < 1e-12);
        let empty = BitPlane::new(20, 10).unwrap();
        assert_eq!(binarization_f(&empty, &empty).unwrap(), 1.0);
        assert_eq!(binarization_f(&empty, &a).unwrap(), 0.0);
        assert!(binarization_f(&a, &BitPlane::new(5, 5).unwrap()).is_err());
    }

    #[test]
    fn the_ratio_error_is_relative() {
        assert!((ratio_error(0.75, 0.7071).unwrap() - 0.0606).abs() < 1e-3);
        assert!(ratio_error(1.0, 0.0).is_err());
        assert!(ratio_error(f64::NAN, 1.0).is_err());
    }

    #[test]
    fn median_and_mean_ignore_what_is_not_a_number() {
        assert_eq!(median(&[3.0, 1.0, f64::NAN, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        assert_eq!(mean(&[1.0, 3.0, f64::INFINITY]), Some(2.0));
    }
}
