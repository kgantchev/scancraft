//! A25: curved sides (specification A25). Owner: WP11.
//!
//! A curved side of a page is still a long step in brightness, but no straight line follows it.
//! A25 looks across the side every 2 px along it, over a band that is narrow at the corners and
//! wide in the middle. It then finds the path that stays on the strongest step while changing
//! course as little as possible. The search is dynamic programming over the band, the approach
//! Montanari (1971) introduced for finding curves in noisy pictures.

use crate::shapes::{add, bilinear, side_ends, side_frame};
use crate::{DetectError, Polarity};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::PlaneF32;
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{SideSample, SideShape};

/// The long side of the pyramid level A25 works on (the level closest to it is used).
const LEVEL_LONG_SIDE: u32 = 2048;
/// The distance between two columns, in the level's pixels.
const COLUMN_STEP: f64 = 2.0;
/// The share of the chord skipped at each end.
const END_SKIP: f64 = 0.02;
/// The band's half-width at the corners, in pixels.
const BAND_BASE: f64 = 6.0;
/// How much wider the band grows towards the middle of the side: 0.32 · len · t · (1 − t), which
/// is 8% of the side's length at the middle.
const BAND_GROWTH: f64 = 0.32;
/// The width of the smoothing of a column's samples, in pixels.
const SIGMA: f64 = 1.0;
/// The cost of a change of one pixel in offset between two columns.
const SMOOTHNESS: f64 = 0.05;
/// The most the offset may change between two neighbouring columns, in pixels.
const MAX_JUMP: i64 = 2;
/// How far outside (or, with Book Spreads, inside) the first path the second search starts.
const SECOND_GAP: i64 = 3;
/// The share of the first path's strength that the second path needs to replace it.
const SECOND_SHARE: f64 = 0.7;
/// The strength from which a column is supported.
const SUPPORTED: f64 = 0.25;
/// The share of the columns that the strongest steps are measured against: the step strengths are
/// divided by the 95th percentile of the columns' strongest steps.
///
/// The specification divides by the 95th percentile of all the band's values. On a clean edge the
/// step fills a few pixels of a band that is up to 170 pixels wide, under 5% of its values, so that
/// percentile is the faint tail of the smoothed step, and every pixel within three of the edge
/// came out at full strength. The path then took the cheapest straight line through that plateau
/// and missed a gentle bow by up to 3 px. The percentile of each column's strongest step is the
/// same idea (a strength that a few outliers cannot raise) without that dependence on the band's
/// width.
const NORMALISE_PERCENTILE: f64 = 0.95;
/// The width of the running median that smooths the distances from the straight line, in columns.
const MEDIAN_WIDTH: usize = 15;
/// The least bow of a curved side, as a share of its length.
const CURVED_BOW_SHARE: f64 = 0.003;
/// The least bow of a curved side, in pixels.
const CURVED_BOW_PIXELS: f64 = 2.0;
/// The share of the columns a curved side must have supported.
const CURVED_SUPPORT_SHARE: f64 = 0.6;
/// The most samples a side hands on.
const MAX_SAMPLES: usize = 64;
/// A side shorter than this, in the level's pixels, is not traced.
const MIN_LENGTH: f64 = 24.0;
/// A side longer than this many times the level's long side is not traced: no page that lies
/// within the photo's allowed area (A6) is that long, and the band's size grows with the length.
const MAX_LENGTH_SHARE: f64 = 4.0;
/// The Gaussian kernel's reach, in pixels (three sigma).
const KERNEL_REACH: i64 = 3;

/// Traces every side not marked in `skip` (border sides, a spine). `inward` is Book Spreads' rule
/// (A25 step 4). Returns top, right, bottom, left; skipped sides come back as `Straight`, and the
/// caller marks a spine as `Spine`.
///
/// `corners` are in full-resolution photo pixels, and so are the samples.
pub fn trace_sides(
    corners: &Quad,
    polarity: &[Polarity; 4],
    pyramid: &LumaPyramid,
    skip: [bool; 4],
    inward: bool,
) -> Result<[SideShape; 4], DetectError> {
    if !corners.is_finite() {
        return Err(scancraft_raster::RasterError::BadParameter("the corners are not finite").into());
    }
    let (Some(level), Some(full)) = (pyramid.closest(LEVEL_LONG_SIDE), pyramid.full()) else {
        return Err(scancraft_core::CoreError::BadSize { width: 0, height: 0 }.into());
    };
    // The level's pixels per photo pixel, along x and along y.
    let kx = f64::from(level.width()) / f64::from(full.width()).max(1.0);
    let ky = f64::from(level.height()) / f64::from(full.height()).max(1.0);
    let in_level = Quad(corners.0.map(|p| Point::new(p.x * kx, p.y * ky)));
    let mut shapes = [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight];
    for side in Side::ALL {
        let index = side.index();
        if skip.get(index).copied().unwrap_or(true) {
            continue;
        }
        let Some(polarity) = polarity.get(index).copied() else { continue };
        let Some(traced) = trace_side(level, &in_level, side, polarity, inward) else { continue };
        let samples =
            traced.samples.iter().map(|s| SideSample { point: Point::new(s.point.x / kx, s.point.y / ky), weight: s.weight }).collect::<Vec<_>>();
        if let Some(slot) = shapes.get_mut(index) {
            *slot = SideShape::Traced { samples, bow: traced.bow as f32, curved: traced.curved };
        }
    }
    Ok(shapes)
}

/// What the trace of one side found, in the level's pixels.
#[derive(Debug, Clone, PartialEq)]
struct Traced {
    samples: Vec<SideSample>,
    /// The bow as a share of the side's length.
    bow: f64,
    curved: bool,
}

/// One column of the band: where it lies and the strength of the step at each offset.
#[derive(Debug, Clone, PartialEq)]
struct Column {
    /// Where the column crosses the chord, in the level's pixels.
    origin: Point,
    /// The offset of the first strength, in pixels (the negative of the band's half-width).
    first: i64,
    /// The strength of the step at each offset from `first`, in 0..=1.
    strength: Vec<f64>,
}

impl Column {
    /// The offset of the last strength.
    fn last(&self) -> i64 {
        self.first + self.strength.len() as i64 - 1
    }

    /// The strength at an offset; 0 outside the band.
    fn at(&self, d: i64) -> f64 {
        usize::try_from(d - self.first).ok().and_then(|i| self.strength.get(i)).copied().unwrap_or(0.0)
    }
}

/// Traces one side (A25 steps 1–6). `None` when the side is too short, or when nothing in the band
/// is a step of the side's polarity.
fn trace_side(level: &PlaneF32, quad: &Quad, side: Side, polarity: Polarity, inward: bool) -> Option<Traced> {
    let (a, b) = side_ends(quad, side);
    let (dir, normal) = side_frame(quad, side);
    let len = a.distance(b);
    let longest = f64::from(level.width().max(level.height())) * MAX_LENGTH_SHARE;
    if !len.is_finite() || !(MIN_LENGTH..=longest).contains(&len) {
        return None;
    }
    let columns = band(level, a, dir, normal, len, polarity)?;
    let first = best_path(&columns, &vec![None; columns.len()])?;
    // Step 4: the outermost path. A printed frame just inside a sheet's edge can be as strong as
    // the edge. With Book Spreads on, the pages under a book's top page lie just outside it, so
    // the second search looks inward.
    let chosen = {
        let limits: Vec<Option<(i64, i64)>> =
            first.iter().map(|d| if inward { Some((i64::MIN, d - SECOND_GAP)) } else { Some((d + SECOND_GAP, i64::MAX)) }).collect();
        match best_path(&columns, &limits) {
            Some(second) if mean_strength(&columns, &second) >= SECOND_SHARE * mean_strength(&columns, &first) => second,
            _ => first,
        }
    };
    samples_of(&columns, &chosen, normal, len)
}

/// The columns of the band, with the strength of the step in each (A25 steps 1 and 2).
fn band(level: &PlaneF32, a: Point, dir: Point, normal: Point, len: f64, polarity: Polarity) -> Option<Vec<Column>> {
    let count = (((1.0 - 2.0 * END_SKIP) * len) / COLUMN_STEP).floor() as usize + 1;
    let kernel = gaussian_kernel();
    let mut columns: Vec<Column> = Vec::with_capacity(count);
    let mut strongest: Vec<f64> = Vec::with_capacity(count);
    for k in 0..count {
        let along = END_SKIP * len + COLUMN_STEP * k as f64;
        let t = along / len;
        let half = (BAND_BASE + BAND_GROWTH * len * t * (1.0 - t)).floor() as i64;
        let origin = add(a, dir.scaled(along));
        // The luma every pixel across the band, with room for the smoothing.
        let reach = half + KERNEL_REACH + 1;
        let luma: Vec<f64> = (-reach..=reach)
            .map(|d| {
                let p = add(origin, normal.scaled(d as f64));
                f64::from(bilinear(level, p.x, p.y))
            })
            .collect();
        let smooth = smoothed(&luma, &kernel);
        let strength: Vec<f64> = (-half..=half)
            .map(|d| {
                let i = (d + reach) as usize;
                let before = smooth.get(i.wrapping_sub(1)).copied().unwrap_or(0.0);
                let after = smooth.get(i + 1).copied().unwrap_or(0.0);
                let slope = (after - before) / 2.0;
                // The page is lighter than what lies outside it: the luma falls along the outward
                // normal there, and rises along it when the page is darker.
                let step = match polarity {
                    Polarity::PageLighter => -slope,
                    Polarity::PageDarker => slope,
                };
                step.max(0.0)
            })
            .collect();
        strongest.push(strength.iter().copied().fold(0.0, f64::max));
        columns.push(Column { origin, first: -half, strength });
    }
    // Divide by the 95th percentile of the columns' strongest steps, and clamp to [0, 1].
    let scale = percentile(&mut strongest, NORMALISE_PERCENTILE);
    if scale.is_nan() || scale <= 1e-9 {
        return None;
    }
    for column in &mut columns {
        for value in &mut column.strength {
            *value = (*value / scale).clamp(0.0, 1.0);
        }
    }
    Some(columns)
}

/// The Gaussian kernel of σ = 1 px, normalised, from −3 to +3.
fn gaussian_kernel() -> Vec<f64> {
    let weights: Vec<f64> = (-KERNEL_REACH..=KERNEL_REACH).map(|d| (-(d * d) as f64 / (2.0 * SIGMA * SIGMA)).exp()).collect();
    let total: f64 = weights.iter().sum();
    weights.into_iter().map(|w| w / total).collect()
}

/// The values smoothed with the kernel; the ends repeat their edge value.
fn smoothed(values: &[f64], kernel: &[f64]) -> Vec<f64> {
    let last = (values.len() as i64 - 1).max(0);
    (0..values.len() as i64)
        .map(|i| {
            kernel
                .iter()
                .enumerate()
                .map(|(j, w)| {
                    let at = (i + j as i64 - KERNEL_REACH).clamp(0, last) as usize;
                    w * values.get(at).copied().unwrap_or(0.0)
                })
                .sum()
        })
        .collect()
}

/// The value at a share of the sorted values.
fn percentile(values: &mut [f64], share: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let index = ((values.len() as f64 * share).ceil() as usize).saturating_sub(1).min(values.len() - 1);
    values.get(index).copied().unwrap_or(0.0)
}

/// The path of offsets, one per column, that maximises the strength along it less the cost of its
/// changes of course (A25 step 3). `limits` holds, per column, the range of offsets that may be
/// used (`None` for no limit). `None` when a column has no offset left to use.
fn best_path(columns: &[Column], limits: &[Option<(i64, i64)>]) -> Option<Vec<i64>> {
    // The usable offsets of each column.
    let ranges: Vec<(i64, i64)> = columns
        .iter()
        .zip(limits)
        .map(|(c, limit)| match limit {
            Some((low, high)) => (c.first.max(*low), c.last().min(*high)),
            None => (c.first, c.last()),
        })
        .collect();
    if ranges.iter().any(|(low, high)| low > high) {
        return None;
    }
    // Forward pass: for every offset, the best score of any path that ends there, and where that
    // path came from.
    let mut scores: Vec<Vec<f64>> = Vec::with_capacity(columns.len());
    let mut came_from: Vec<Vec<i64>> = Vec::with_capacity(columns.len());
    for (k, column) in columns.iter().enumerate() {
        let (low, high) = *ranges.get(k)?;
        let mut here = Vec::with_capacity((high - low + 1) as usize);
        let mut from = Vec::with_capacity(here.capacity());
        for d in low..=high {
            let own = column.at(d);
            if k == 0 {
                here.push(own);
                from.push(d);
                continue;
            }
            let (prev_low, prev_high) = *ranges.get(k - 1)?;
            let prev = scores.get(k - 1)?;
            // The neighbours with the smaller |d'| first, so that a tie goes to the smaller one.
            let mut best: Option<(f64, i64)> = None;
            for jump in neighbours() {
                let earlier = d + jump;
                if earlier < prev_low || earlier > prev_high {
                    continue;
                }
                let Some(score) = prev.get((earlier - prev_low) as usize) else { continue };
                let value = score - SMOOTHNESS * jump.abs() as f64;
                let better = match best {
                    None => true,
                    Some((top, _)) => value > top,
                };
                if better {
                    best = Some((value, earlier));
                }
            }
            let (value, earlier) = best?;
            here.push(own + value);
            from.push(earlier);
        }
        scores.push(here);
        came_from.push(from);
    }
    // Backward pass: the end of the best path, then the way back. A tie goes to the smaller |d|.
    let (low, high) = *ranges.last()?;
    let last_scores = scores.last()?;
    let mut end: Option<(f64, i64)> = None;
    let mut order: Vec<i64> = (low..=high).collect();
    order.sort_by_key(|d| d.abs());
    for d in order {
        let value = last_scores.get((d - low) as usize).copied()?;
        if end.is_none_or(|(top, _)| value > top) {
            end = Some((value, d));
        }
    }
    let mut d = end?.1;
    let mut path = vec![0i64; columns.len()];
    for k in (0..columns.len()).rev() {
        *path.get_mut(k)? = d;
        if k > 0 {
            let (low, _) = *ranges.get(k)?;
            d = came_from.get(k)?.get((d - low) as usize).copied()?;
        }
    }
    Some(path)
}

/// The changes of offset between neighbouring columns, the smaller ones first. The offset the
/// path comes from is `d + jump`.
fn neighbours() -> impl Iterator<Item = i64> {
    [0, -1, 1, -MAX_JUMP, MAX_JUMP].into_iter()
}

/// The mean strength along a path.
fn mean_strength(columns: &[Column], path: &[i64]) -> f64 {
    if columns.is_empty() {
        return 0.0;
    }
    columns.iter().zip(path).map(|(c, d)| c.at(*d)).sum::<f64>() / columns.len() as f64
}

/// The samples of a path, its bow and its verdict (A25 steps 5 and 6), in the level's pixels.
fn samples_of(columns: &[Column], path: &[i64], normal: Point, len: f64) -> Option<Traced> {
    // Step 5: a column is supported when its step is strong enough; its offset is refined with a
    // parabola through the strengths on either side.
    let mut points: Vec<SideSample> = Vec::new();
    for (column, d) in columns.iter().zip(path) {
        let strength = column.at(*d);
        if strength < SUPPORTED {
            continue;
        }
        let (before, after) = (column.at(d - 1), column.at(d + 1));
        let curvature = before - 2.0 * strength + after;
        let inside_band = *d > column.first && *d < column.last();
        let shift = if curvature < -1e-9 && inside_band { (0.5 * (before - after) / curvature).clamp(-0.5, 0.5) } else { 0.0 };
        let point = add(column.origin, normal.scaled(*d as f64 + shift));
        points.push(SideSample { point, weight: strength as f32 });
    }
    if points.len() < 2 {
        return None;
    }
    let bow_pixels = bow(&points);
    let share = bow_pixels / len;
    let supported = points.len() as f64 / columns.len().max(1) as f64;
    let curved = share >= CURVED_BOW_SHARE && bow_pixels >= CURVED_BOW_PIXELS && supported >= CURVED_SUPPORT_SHARE;
    Some(Traced { samples: spread(&points, MAX_SAMPLES), bow: share, curved })
}

/// At most `count` of the points, evenly spread over them, the first and the last included.
fn spread(points: &[SideSample], count: usize) -> Vec<SideSample> {
    if points.len() <= count || count < 2 {
        return points.iter().take(count.max(1)).copied().collect();
    }
    (0..count)
        .filter_map(|i| {
            let at = (i as f64 * (points.len() - 1) as f64 / (count - 1) as f64).round() as usize;
            points.get(at).copied()
        })
        .collect()
}

/// The side's bow in pixels: how far the samples spread away from the straight line through them
/// (a weighted total least squares fit), after a running median over 15 columns.
///
/// The spread is the distance between the farthest sample on one side of the line and the farthest
/// on the other. The best-fit line of an arc runs through the arc's middle, so the largest distance
/// from it is only about 0.6 of the arc's height, and a side that bows by 0.5% of its length would
/// measure 0.3%, on the edge of the threshold. The spread is the arc's height itself.
fn bow(points: &[SideSample]) -> f64 {
    let total: f64 = points.iter().map(|p| f64::from(p.weight)).sum();
    if total.is_nan() || total <= 0.0 {
        return 0.0;
    }
    let (mut mx, mut my) = (0.0, 0.0);
    for p in points {
        mx += f64::from(p.weight) * p.point.x;
        my += f64::from(p.weight) * p.point.y;
    }
    let (mx, my) = (mx / total, my / total);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in points {
        let (dx, dy) = (p.point.x - mx, p.point.y - my);
        let w = f64::from(p.weight);
        sxx += w * dx * dx;
        sxy += w * dx * dy;
        syy += w * dy * dy;
    }
    // The principal axis of the weighted points.
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (ax, ay) = (angle.cos(), angle.sin());
    let distances: Vec<f64> = points.iter().map(|p| (p.point.x - mx) * -ay + (p.point.y - my) * ax).collect();
    let smooth = running_median(&distances, MEDIAN_WIDTH);
    let (low, high) = smooth.iter().fold((f64::MAX, f64::MIN), |(low, high), d| (low.min(*d), high.max(*d)));
    if high >= low { high - low } else { 0.0 }
}

/// The median of each value and its neighbours within `width` values (fewer at the ends).
fn running_median(values: &[f64], width: usize) -> Vec<f64> {
    let half = width / 2;
    (0..values.len())
        .map(|i| {
            let from = i.saturating_sub(half);
            let to = (i + half + 1).min(values.len());
            let mut window: Vec<f64> = values.get(from..to).map(<[f64]>::to_vec).unwrap_or_default();
            crate::shapes::median(&mut window)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refine::build_pyramid;
    use scancraft_core::image::Plane;

    const PAGE: f64 = 0.9;
    const DESK: f64 = 0.2;

    /// A small deterministic generator of noise, so that the tests repeat.
    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
            bits as f64 / (1u64 << 53) as f64
        }

        /// Roughly normal, of standard deviation 1: the sum of twelve uniform values less six.
        fn normal(&mut self) -> f64 {
            (0..12).map(|_| self.next()).sum::<f64>() - 6.0
        }
    }

    /// The page's outline: x from 200 to 1200, y from 200 to 800.
    fn corners() -> Quad {
        Quad([Point::new(200.0, 200.0), Point::new(1200.0, 200.0), Point::new(1200.0, 800.0), Point::new(200.0, 800.0)])
    }

    /// How far a pixel is inside the page, in pixels, for a page whose top and bottom edges bow
    /// outwards by `bow` pixels at their middle.
    fn inside(x: f64, y: f64, bow: f64) -> f64 {
        let t = ((x - 200.0) / 1000.0).clamp(0.0, 1.0);
        let lift = bow * 4.0 * t * (1.0 - t);
        let top = y - (200.0 - lift);
        let bottom = (800.0 + lift) - y;
        let left = x - 200.0;
        let right = 1200.0 - x;
        top.min(bottom).min(left).min(right)
    }

    /// The scene as a luma plane: a lighter page on a darker desk, the edge one pixel wide, and
    /// noise of `sigma` levels (of 255).
    fn scene(bow: f64, sigma: f64, seed: u64, extra: impl Fn(f64, f64) -> Option<f64>) -> LumaPyramid {
        let mut noise = Noise(seed | 1);
        let mut data = Vec::with_capacity(1400 * 1000);
        for j in 0..1000u32 {
            for i in 0..1400u32 {
                let (x, y) = (f64::from(i) + 0.5, f64::from(j) + 0.5);
                let edge = (inside(x, y, bow) + 0.5).clamp(0.0, 1.0);
                let mut v = DESK + (PAGE - DESK) * edge;
                if let Some(over) = extra(x, y) {
                    v = over;
                }
                if sigma > 0.0 {
                    v += noise.normal() * sigma / 255.0;
                }
                data.push(v as f32);
            }
        }
        build_pyramid(&Plane::from_vec(1400, 1000, data).unwrap()).unwrap()
    }

    /// The distance of a sample from the page's true edge, measured across it.
    fn error(sample: &SideSample, side: Side, bow: f64) -> f64 {
        let (x, y) = (sample.point.x, sample.point.y);
        let t = ((x - 200.0) / 1000.0).clamp(0.0, 1.0);
        let lift = bow * 4.0 * t * (1.0 - t);
        // The slope of the edge, to turn a vertical error into the distance across it.
        let slope = bow * 4.0 * (1.0 - 2.0 * t) / 1000.0;
        let across = (1.0 + slope * slope).sqrt();
        match side {
            Side::Top => ((y - (200.0 - lift)) / across).abs(),
            Side::Bottom => ((y - (800.0 + lift)) / across).abs(),
            Side::Left => (x - 200.0).abs(),
            Side::Right => (x - 1200.0).abs(),
        }
    }

    const LIGHT: [Polarity; 4] = [Polarity::PageLighter; 4];

    fn trace(pyramid: &LumaPyramid, skip: [bool; 4], inward: bool) -> [SideShape; 4] {
        trace_sides(&corners(), &LIGHT, pyramid, skip, inward).unwrap()
    }

    fn traced(shape: &SideShape) -> (&[SideSample], f32, bool) {
        match shape {
            SideShape::Traced { samples, bow, curved } => (samples, *bow, *curved),
            other => panic!("expected a traced side, got {other:?}"),
        }
    }

    #[test]
    fn bowed_top_and_bottom_edges_are_followed_and_marked_curved() {
        // 0.5% to 6% of the side's length (1000 px).
        for share in [0.005, 0.01, 0.02, 0.04, 0.06] {
            let bow = share * 1000.0;
            let pyramid = scene(bow, 0.0, 11, |_, _| None);
            let shapes = trace(&pyramid, [false; 4], false);
            for side in [Side::Top, Side::Bottom] {
                let (samples, measured, curved) = traced(&shapes[side.index()]);
                assert!(curved, "{side:?} bowed by {share} was not marked curved (bow {measured})");
                assert!(!samples.is_empty() && samples.len() <= MAX_SAMPLES);
                let within = samples.iter().filter(|s| error(s, side, bow) <= 1.5).count();
                assert!(
                    within as f64 >= 0.95 * samples.len() as f64,
                    "{side:?} bowed by {share}: {within} of {} samples within 1.5 px",
                    samples.len()
                );
            }
            // The sides that don't bow are straight.
            for side in [Side::Left, Side::Right] {
                assert!(!traced(&shapes[side.index()]).2, "{side:?} of a bowed page was marked curved");
            }
        }
    }

    #[test]
    fn straight_sides_with_blur_and_noise_are_marked_straight() {
        let mut curved = 0;
        let mut total = 0;
        for trial in 0..40u64 {
            // The noise is σ = 6 levels; the edge is one pixel wide.
            let pyramid = scene(0.0, 6.0, 1000 + trial, |_, _| None);
            for shape in &trace(&pyramid, [false; 4], false) {
                total += 1;
                if let SideShape::Traced { curved: true, .. } = shape {
                    curved += 1;
                }
            }
        }
        assert!(f64::from(total - curved) >= 0.98 * f64::from(total), "{curved} of {total} straight sides were marked curved");
    }

    #[test]
    fn a_ruled_frame_inside_a_curved_edge_does_not_take_the_edge_s_place() {
        // A dark printed frame 12 px inside the edge, 2 px wide, darker than the desk.
        let bow = 30.0;
        let pyramid = scene(bow, 0.0, 5, |x, y| {
            let d = inside(x, y, bow);
            (d > 12.0 && d < 14.0).then_some(0.05)
        });
        let shapes = trace(&pyramid, [false; 4], false);
        for side in [Side::Top, Side::Bottom] {
            let (samples, _, curved) = traced(&shapes[side.index()]);
            assert!(curved);
            let within = samples.iter().filter(|s| error(s, side, bow) <= 1.5).count();
            assert!(within as f64 >= 0.95 * samples.len() as f64, "{side:?}: the frame was followed, not the edge ({within} of {})", samples.len());
        }
    }

    #[test]
    fn with_book_spreads_the_top_page_s_edge_is_chosen_over_the_pages_beneath() {
        // The pages beneath the top page show 8 px outside its edge, a little darker than it, and
        // their own edge is the stronger step against the desk (0.38 against 0.32).
        let bow = 20.0;
        let pyramid = scene(bow, 0.0, 9, |x, y| {
            let d = inside(x, y, bow);
            (d <= 0.0 && d > -8.0).then_some(0.58)
        });
        let inward = trace(&pyramid, [false; 4], true);
        let outward = trace(&pyramid, [false; 4], false);
        for side in [Side::Top, Side::Bottom] {
            let (samples, _, _) = traced(&inward[side.index()]);
            let near_top = samples.iter().filter(|s| error(s, side, bow) <= 1.5).count();
            assert!(
                near_top as f64 >= 0.9 * samples.len() as f64,
                "{side:?}: Book Spreads did not choose the top page's edge ({near_top} of {})",
                samples.len()
            );
            // Without Book Spreads the outermost edge, the stack's, is the one that is followed.
            let (samples, _, _) = traced(&outward[side.index()]);
            let near_stack = samples.iter().filter(|s| (error(s, side, bow) - 8.0).abs() <= 1.5).count();
            assert!(near_stack as f64 >= 0.9 * samples.len() as f64, "{side:?}: without Book Spreads the outermost edge was not chosen");
        }
    }

    #[test]
    fn skipped_sides_come_back_straight() {
        let pyramid = scene(20.0, 0.0, 3, |_, _| None);
        let shapes = trace(&pyramid, [true, false, true, false], false);
        assert_eq!(shapes[0], SideShape::Straight);
        assert_eq!(shapes[2], SideShape::Straight);
        assert!(matches!(shapes[1], SideShape::Traced { .. }));
    }

    #[test]
    fn a_page_of_one_brightness_gives_no_samples() {
        let pyramid = build_pyramid(&Plane::from_vec(400, 300, vec![0.5f32; 400 * 300]).unwrap()).unwrap();
        let quad = Quad([Point::new(50.0, 50.0), Point::new(350.0, 50.0), Point::new(350.0, 250.0), Point::new(50.0, 250.0)]);
        let shapes = trace_sides(&quad, &LIGHT, &pyramid, [false; 4], false).unwrap();
        assert!(shapes.iter().all(|s| *s == SideShape::Straight), "{shapes:?}");
    }

    #[test]
    fn hostile_corners_are_an_error_or_straight_never_a_panic() {
        let pyramid = scene(0.0, 0.0, 2, |_, _| None);
        let nan = Quad([Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)]);
        assert!(trace_sides(&nan, &LIGHT, &pyramid, [false; 4], false).is_err());
        // A page that is a dot, and one far outside the photo.
        let dot = Quad([Point::new(5.0, 5.0); 4]);
        assert!(trace_sides(&dot, &LIGHT, &pyramid, [false; 4], false).unwrap().iter().all(|s| *s == SideShape::Straight));
        let far = Quad([Point::new(-9e6, -9e6), Point::new(9e6, -9e6), Point::new(9e6, 9e6), Point::new(-9e6, 9e6)]);
        let _ = trace_sides(&far, &LIGHT, &pyramid, [false; 4], false);
    }

    #[test]
    fn the_running_median_and_the_spread_behave() {
        assert_eq!(running_median(&[1.0, 9.0, 1.0, 1.0, 1.0], 3), vec![5.0, 1.0, 1.0, 1.0, 1.0]);
        let points: Vec<SideSample> = (0..200).map(|i| SideSample { point: Point::new(f64::from(i), 0.0), weight: 1.0 }).collect();
        let kept = spread(&points, 64);
        assert_eq!(kept.len(), 64);
        assert_eq!(kept.first().map(|s| s.point.x), Some(0.0));
        assert_eq!(kept.last().map(|s| s.point.x), Some(199.0));
    }

    #[test]
    fn the_trace_is_the_same_whatever_the_thread_count() {
        let pyramid = scene(20.0, 6.0, 77, |_, _| None);
        let once = trace(&pyramid, [false; 4], false);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap();
        let eight = pool.install(|| trace(&pyramid, [false; 4], false));
        assert_eq!(once, eight);
    }
}
