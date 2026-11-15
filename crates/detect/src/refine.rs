//! A10: sub-pixel edge refinement, and the luma pyramid it builds (specification A10). Owner: WP8.
//!
//! Each side of the page is a long, straight step in brightness. Refinement looks across that
//! step at many places along the side, finds the step's exact position on each look to a fraction
//! of a pixel, and fits a straight line through the positions that agree. The corners are where
//! neighbouring lines cross.

use crate::DetectError;
use crate::Polarity;
use crate::shapes::{add, bilinear, in_allowed_area, is_convex, is_positive, line_intersection, median, polygon_area, side_ends, side_frame};
use scancraft_core::CoreError;
use scancraft_core::Step;
use scancraft_core::determinism::{Xorshift64Star, seed_for};
use scancraft_core::geom::{Line, Point, Quad, Side};
use scancraft_core::image::{Plane, PlaneF32};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_raster::blur::gaussian;
use scancraft_raster::downscale::{area_downscale, fit_long_side};
use std::cmp::Ordering;

/// The long side of the pyramid's coarsest level (A10).
const LEVEL_ZERO: u32 = 1024;
/// The half-width of the search at the coarsest level, in pixels (A10 step 1).
const WIDE_RADIUS: f64 = 16.0;
/// The half-width of the search at the finer levels, in pixels (A10 step 1).
const NARROW_RADIUS: f64 = 6.0;
/// The distance between two samples of a profile, in pixels (A10 step 3).
const PROFILE_STEP: f64 = 0.5;
/// The smoothing of a profile, in pixels (A10 step 3).
const PROFILE_SIGMA: f64 = 1.0;
/// The share of the side skipped at each end (A10 step 2).
const END_SKIP: f64 = 0.08;
/// The most scan lines of a side (A10 step 2).
const MAX_SCAN_LINES: usize = 400;
/// The fewest scan lines of a side (A10 step 2).
const MIN_SCAN_LINES: usize = 32;
/// The share of the strongest extremes that counts as a candidate (A10 step 5).
const EXTREME_SHARE: f64 = 0.25;
/// The weakest step that counts as an edge at all: half a grey level of slope per pixel.
///
/// The specification's threshold is relative (a quarter of the median of the strongest extremes),
/// which says nothing about a profile with no step in it. There the median is rounding noise
/// (about 1e-8 for a flat region seen through the Gaussian), a quarter of it is just as small, and
/// noise passes. A side that runs along the photo's frame over a flat page was fitted to such
/// noise, a few pixels off the frame, until this floor was added. It sits far below any real edge
/// and any camera noise (noise of σ = 6 levels gives slopes of several times this).
const MIN_STEP: f64 = 0.5 / 255.0;
/// The most candidates kept for one scan line (A10 step 5).
const MAX_EXTREMES: usize = 4;
/// The hypotheses drawn for a side (A10 step 6).
const HYPOTHESES: usize = 200;
/// How close a candidate must be to a hypothesis to support it, in pixels (A10 step 6).
const SUPPORT_DISTANCE: f64 = 1.0;
/// The share of the best support a hypothesis must have (A10 step 6).
const SUPPORT_SHARE: f64 = 0.7;
/// The share of the scan lines a fit must come from (A10 step 7).
const INLIER_SHARE: f64 = 0.4;
/// How far a side may turn and still be accepted, in degrees (A10 step 7).
const MAX_TURN: f64 = 6.0;
/// How far a corner may move, as a multiple of the search width (A10's final checks).
const MAX_MOVE: f64 = 2.0;
/// The area the refined quadrilateral may have, as a share of the input's (A10's final checks).
const MIN_AREA_SHARE: f64 = 0.8;
/// The area the refined quadrilateral may have, as a share of the input's (A10's final checks).
const MAX_AREA_SHARE: f64 = 1.25;

/// Builds the pyramid from the full-resolution luma (level 0 at long side 1024 px).
///
/// Level 0 is the work image; each finer level doubles the size and is made from the
/// full-resolution luma, so that no error is carried from one level to the next. The last level
/// is the full-resolution luma itself.
pub fn build_pyramid(full_luma: &PlaneF32) -> Result<LumaPyramid, DetectError> {
    let (w, h) = (full_luma.width(), full_luma.height());
    let long = w.max(h);
    if long == 0 {
        return Err(CoreError::BadSize { width: w, height: h }.into());
    }
    let mut levels: Vec<PlaneF32> = Vec::new();
    let mut size = LEVEL_ZERO.min(long);
    while size < long {
        levels.push(level_of(full_luma, size)?);
        size = size.saturating_mul(2).min(long);
    }
    levels.push(full_luma.clone());
    Ok(LumaPyramid { levels })
}

/// One level of the pyramid: the full-resolution luma reduced to the given long side.
fn level_of(full_luma: &PlaneF32, size: u32) -> Result<PlaneF32, DetectError> {
    let (w, h) = fit_long_side(full_luma.width(), full_luma.height(), size);
    if (w, h) == (full_luma.width(), full_luma.height()) {
        return Ok(full_luma.clone());
    }
    Ok(area_downscale(full_luma, w, h)?)
}

/// A10's result, in full-resolution photo pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Refined {
    pub corners: Quad,
    /// Sides kept as they were (A10 step 7), top, right, bottom, left: those the finest level
    /// could not fit, so their accuracy is that of the coarser levels, or of the input.
    pub unrefined: [bool; 4],
    pub polarity: [Polarity; 4],
}

/// One place where the edge might be, on one scan line.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    /// The distance along the side, from its start.
    along: f64,
    /// The distance from the side's line, along the outward normal.
    offset: f64,
    /// How strong the step is.
    weight: f64,
    /// Which scan line the candidate came from.
    line: usize,
}

/// A line in the side's own frame: the distance along the side, and the distance from it.
#[derive(Debug, Clone, Copy)]
struct FrameLine {
    along: f64,
    offset: f64,
    /// The direction in the frame, as (along, offset).
    direction: (f64, f64),
}

impl FrameLine {
    /// The line through two candidates.
    fn through(a: &Candidate, b: &Candidate) -> Option<Self> {
        let da = b.along - a.along;
        let db = b.offset - a.offset;
        let len = da.hypot(db);
        if !is_positive(len - 1e-9) {
            return None;
        }
        Some(Self { along: a.along, offset: a.offset, direction: (da / len, db / len) })
    }

    /// How far a candidate lies from the line.
    fn distance(&self, c: &Candidate) -> f64 {
        let v = (c.along - self.along, c.offset - self.offset);
        (v.0 * self.direction.1 - v.1 * self.direction.0).abs()
    }
}

/// Refines `quad`, given in work-image pixels; `seed` comes from `determinism::seed_for`.
///
/// The corners come back in full-resolution photo pixels. If the refinement would move the page
/// too far, turn it too much, or change its shape too much, the input comes back unchanged.
pub fn refine(quad: &Quad, pyramid: &LumaPyramid, seed: u64) -> Result<Refined, DetectError> {
    let levels = &pyramid.levels;
    let Some(first) = levels.first() else {
        return Err(CoreError::BadSize { width: 0, height: 0 }.into());
    };
    let full = levels.last().unwrap_or(first);
    let to_photo_x = f64::from(full.width()) / f64::from(first.width());
    let to_photo_y = f64::from(full.height()) / f64::from(first.height());
    let unchanged = Refined {
        corners: Quad(quad.0.map(|p| Point::new(p.x * to_photo_x, p.y * to_photo_y))),
        unrefined: [true; 4],
        polarity: [Polarity::PageLighter; 4],
    };
    let mut corners = quad.0;
    let mut unrefined = [false; 4];
    let mut polarity = [Polarity::PageLighter; 4];
    for (index, level) in levels.iter().enumerate() {
        let radius = if index == 0 { WIDE_RADIUS } else { NARROW_RADIUS };
        let mut fitted: [Option<Line>; 4] = [None, None, None, None];
        for side in Side::ALL {
            let part = seed_for(seed, Step::A10, index as u64 * 4 + side.index() as u64);
            match fit_side(level, &corners, side, radius, part)? {
                Some((line, side_polarity)) => {
                    fitted[side.index()] = Some(line);
                    polarity[side.index()] = side_polarity;
                    unrefined[side.index()] = false;
                }
                // The flag describes the last level, which fixes the final accuracy: a side that
                // a coarse level could not fit, but a finer one could, is refined.
                None => unrefined[side.index()] = true,
            }
        }
        corners = corners_of(&corners, &fitted);
        // The next level is twice the size.
        if let Some(next) = levels.get(index + 1) {
            let sx = f64::from(next.width()) / f64::from(level.width());
            let sy = f64::from(next.height()) / f64::from(level.height());
            corners = corners.map(|p| Point::new(p.x * sx, p.y * sy));
        }
    }
    let refined = Refined { corners: Quad(corners), unrefined, polarity };
    if accepted(&refined.corners, quad, to_photo_x, to_photo_y, full.width(), full.height(), first)? { Ok(refined) } else { Ok(unchanged) }
}

/// The final checks at full resolution (A10).
///
/// The corner may not have moved more than twice the search width, the quadrilateral must still
/// be convex, and its area must not have changed by more than a quarter.
fn accepted(corners: &Quad, input: &Quad, to_photo_x: f64, to_photo_y: f64, width: u32, height: u32, first: &PlaneF32) -> Result<bool, DetectError> {
    let original = Quad(input.0.map(|p| Point::new(p.x * to_photo_x, p.y * to_photo_y)));
    let moved = corners.0.iter().zip(original.0.iter()).map(|(a, b)| a.distance(*b)).fold(0.0f64, f64::max);
    let scale = f64::from(width.max(height)) / f64::from(first.width().max(first.height()));
    if moved > MAX_MOVE * WIDE_RADIUS * scale {
        return Ok(false);
    }
    if !is_convex(&corners.0) {
        return Ok(false);
    }
    let before = polygon_area(&original.0);
    let after = polygon_area(&corners.0);
    if before <= 0.0 {
        return Ok(false);
    }
    let ratio = after / before;
    if ratio < MIN_AREA_SHARE || ratio > MAX_AREA_SHARE {
        return Ok(false);
    }
    Ok(corners.0.iter().all(|p| in_allowed_area(*p, width, height)))
}

/// The corners of the next level: where neighbouring sides cross.
fn corners_of(original: &[Point; 4], fitted: &[Option<Line>; 4]) -> [Point; 4] {
    let quad = Quad(*original);
    let mut out = *original;
    // Each corner is where two sides meet: left and top, top and right, right and bottom,
    // bottom and left.
    let pairs = [(3usize, 0usize), (0, 1), (1, 2), (2, 3)];
    for (corner, (a, b)) in pairs.iter().enumerate() {
        let first = fitted.get(*a).and_then(|l| *l).unwrap_or_else(|| side_line(&quad, *a));
        let second = fitted.get(*b).and_then(|l| *l).unwrap_or_else(|| side_line(&quad, *b));
        if let Some(p) = line_intersection(&first, &second) {
            out[corner] = p;
        }
    }
    out
}

/// The line through a side's two corners.
fn side_line(quad: &Quad, side: usize) -> Line {
    let side = Side::ALL.get(side).copied().unwrap_or(Side::Top);
    let (a, b) = side_ends(quad, side);
    Line { point: a, direction: crate::shapes::normalised(crate::shapes::sub(b, a)) }
}

/// Fits one side of the page, from the pyramid level's luma.
fn fit_side(level: &PlaneF32, corners: &[Point; 4], side: Side, radius: f64, seed: u64) -> Result<Option<(Line, Polarity)>, DetectError> {
    let quad = Quad(*corners);
    let (a, b) = side_ends(&quad, side);
    let (dir, normal) = side_frame(&quad, side);
    let side_length = a.distance(b);
    if !is_positive(side_length - 1.0) {
        return Ok(None);
    }
    let count = ((side_length / 4.0).round() as usize).clamp(MIN_SCAN_LINES, MAX_SCAN_LINES);
    let samples = ((2.0 * radius / PROFILE_STEP).round() as usize) + 1;
    // Step 2: the scan lines, and step 3: their profiles.
    let mut profiles: Vec<Vec<f32>> = Vec::with_capacity(count);
    let mut positions: Vec<f64> = Vec::with_capacity(count);
    for i in 0..count {
        let t = END_SKIP * side_length + (1.0 - 2.0 * END_SKIP) * side_length * i as f64 / (count - 1) as f64;
        positions.push(t);
        let base = add(a, dir.scaled(t));
        let mut profile = Vec::with_capacity(samples);
        for k in 0..samples {
            let u = -radius + PROFILE_STEP * k as f64;
            let p = add(base, normal.scaled(u));
            profile.push(bilinear(level, p.x, p.y));
        }
        profiles.push(profile);
    }
    // Step 3: smooth each profile with a Gaussian and differentiate it.
    let mut derivatives: Vec<Vec<f64>> = Vec::with_capacity(count);
    let mut inside_sum = 0.0f64;
    let mut inside_count = 0usize;
    let mut outside_sum = 0.0f64;
    let mut outside_count = 0usize;
    let edge = ((1.5 / PROFILE_STEP) as usize) + 1;
    for profile in &profiles {
        let plane = Plane::from_vec(samples as u32, 1, profile.clone())?;
        let smoothed = gaussian(&plane, (PROFILE_SIGMA / PROFILE_STEP) as f32)?;
        let values = smoothed.data();
        for k in 0..edge {
            inside_sum += f64::from(values.get(k).copied().unwrap_or(0.0));
            inside_count += 1;
        }
        for k in (samples.saturating_sub(edge))..samples {
            outside_sum += f64::from(values.get(k).copied().unwrap_or(0.0));
            outside_count += 1;
        }
        // The central difference; the two end samples have no neighbour on one side.
        let d: Vec<f64> = (0..samples)
            .map(|k| match (k.checked_sub(1).and_then(|p| values.get(p)), values.get(k + 1)) {
                (Some(before), Some(after)) => f64::from(*after) - f64::from(*before),
                _ => 0.0,
            })
            .collect();
        derivatives.push(d);
    }
    // Step 4: the polarity, from the ends of all the profiles.
    let inside = inside_sum / inside_count.max(1) as f64;
    let outside = outside_sum / outside_count.max(1) as f64;
    let polarity = if inside >= outside { Polarity::PageLighter } else { Polarity::PageDarker };
    // Step 5: the candidates of each scan line.
    let mut strongest: Vec<f64> = Vec::with_capacity(count);
    for d in &derivatives {
        let mut best = 0.0f64;
        for k in 1..d.len().saturating_sub(1) {
            if is_extreme(d, k, polarity) {
                best = best.max(d[k].abs());
            }
        }
        strongest.push(best);
    }
    let mut found: Vec<f64> = strongest.iter().copied().filter(|v| *v >= MIN_STEP).collect();
    if found.is_empty() {
        return Ok(None);
    }
    let threshold = (EXTREME_SHARE * median(&mut found)).max(MIN_STEP);
    let mut candidates: Vec<Candidate> = Vec::new();
    for (line, d) in derivatives.iter().enumerate() {
        let mut extremes: Vec<(usize, f64)> = Vec::new();
        for k in 1..d.len().saturating_sub(1) {
            if is_extreme(d, k, polarity) && d[k].abs() >= threshold {
                extremes.push((k, d[k].abs()));
            }
        }
        extremes.sort_by(|a, b| b.1.total_cmp(&a.1));
        extremes.truncate(MAX_EXTREMES);
        for (k, magnitude) in extremes {
            // The position of the extremum, refined with a parabola through its neighbours.
            let (before, middle, after) = (d[k - 1], d[k], d[k + 1]);
            let denominator = before - 2.0 * middle + after;
            let mut delta = if denominator.abs() > 1e-12 { 0.5 * (before - after) / denominator } else { 0.0 };
            delta = delta.clamp(-1.0, 1.0);
            let offset = -radius + PROFILE_STEP * k as f64 + PROFILE_STEP * delta;
            candidates.push(Candidate { along: positions.get(line).copied().unwrap_or(0.0), offset, weight: magnitude, line });
        }
    }
    // Step 6: the hypotheses.
    let mut per_line: Vec<Vec<Candidate>> = vec![Vec::new(); count];
    for candidate in candidates {
        if let Some(slot) = per_line.get_mut(candidate.line) {
            slot.push(candidate);
        }
    }
    let with_candidates: Vec<usize> = (0..count).filter(|i| per_line.get(*i).is_some_and(|c| !c.is_empty())).collect();
    if with_candidates.len() < 2 {
        return Ok(None);
    }
    let mut rng = Xorshift64Star::new(seed);
    let mut best_support = 0usize;
    let mut hypotheses: Vec<(Vec<Candidate>, f64)> = Vec::with_capacity(HYPOTHESES);
    for _ in 0..HYPOTHESES {
        let first = with_candidates.get(rng.below(with_candidates.len() as u64) as usize).copied();
        let mut second = with_candidates.get(rng.below(with_candidates.len() as u64) as usize).copied();
        let mut guard = 0;
        while second == first && guard < 8 {
            second = with_candidates.get(rng.below(with_candidates.len() as u64) as usize).copied();
            guard += 1;
        }
        let (Some(one), Some(two)) = (first, second) else { continue };
        if one == two {
            continue;
        }
        let (Some(c1), Some(c2)) = (pick(&per_line, one, &mut rng), pick(&per_line, two, &mut rng)) else { continue };
        let Some(line) = FrameLine::through(&c1, &c2) else { continue };
        // The support: the scan lines with a candidate within 1 px of the hypothesis. Each such
        // scan line gives its closest candidate to the fit; the others are not inliers.
        let mut inliers: Vec<Candidate> = Vec::new();
        for &li in &with_candidates {
            let closest = per_line
                .get(li)
                .into_iter()
                .flatten()
                .map(|c| (line.distance(c), *c))
                .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(Ordering::Equal));
            if let Some((d, c)) = closest
                && d <= SUPPORT_DISTANCE
            {
                inliers.push(c);
            }
        }
        best_support = best_support.max(inliers.len());
        // How far out the hypothesis lies: the mean offset of its inliers along the normal.
        let mean_offset = if inliers.is_empty() { 0.0 } else { inliers.iter().map(|c| c.offset).sum::<f64>() / inliers.len() as f64 };
        hypotheses.push((inliers, mean_offset));
    }
    if best_support == 0 {
        return Ok(None);
    }
    // Among the hypotheses with at least 70% of the best support, take the outermost one: the
    // paper's edge, not a ruled frame just inside it.
    let limit = (SUPPORT_SHARE * best_support as f64).ceil() as usize;
    let Some((inliers, _)) =
        hypotheses.iter().filter(|(inliers, _)| inliers.len() >= limit).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
    else {
        return Ok(None);
    };
    // Step 7: accept the fit if its inliers come from at least 40% of the scan lines and it turns
    // the side by less than 6°. Otherwise the side is kept as it was.
    if (inliers.len() as f64) < INLIER_SHARE * count as f64 {
        return Ok(None);
    }
    let Some((mean_along, mean_offset, axis)) = weighted_fit(inliers) else { return Ok(None) };
    // The axis can point either way along the side, so the turn is measured against the side's
    // own direction, whichever way the axis was drawn.
    let turn = axis.1.atan2(axis.0.abs()).to_degrees().abs();
    if turn > MAX_TURN {
        return Ok(None);
    }
    let direction = add(dir.scaled(axis.0), normal.scaled(axis.1));
    let point = add(add(a, dir.scaled(mean_along)), normal.scaled(mean_offset));
    Ok(Some((Line { point, direction }, polarity)))
}

/// Whether the derivative at `k` is an extremum of the side's polarity.
fn is_extreme(d: &[f64], k: usize, polarity: Polarity) -> bool {
    let (before, here, after) = (d[k - 1], d[k], d[k + 1]);
    match polarity {
        Polarity::PageLighter => here < before && here <= after && here < 0.0,
        Polarity::PageDarker => here > before && here >= after && here > 0.0,
    }
}

/// One of a scan line's candidates, chosen at random.
fn pick(per_line: &[Vec<Candidate>], line: usize, rng: &mut Xorshift64Star) -> Option<Candidate> {
    let candidates = per_line.get(line)?;
    if candidates.is_empty() {
        return None;
    }
    let index = rng.below(candidates.len() as u64) as usize;
    candidates.get(index).copied()
}

/// The weighted total least squares fit of the inlier candidates: their weighted middle and their
/// principal axis, in the side's own frame.
fn weighted_fit(inliers: &[Candidate]) -> Option<(f64, f64, (f64, f64))> {
    let weight: f64 = inliers.iter().map(|c| c.weight).sum();
    if !is_positive(weight) {
        return None;
    }
    let mean_along = inliers.iter().map(|c| c.weight * c.along).sum::<f64>() / weight;
    let mean_offset = inliers.iter().map(|c| c.weight * c.offset).sum::<f64>() / weight;
    let (mut m11, mut m12, mut m22) = (0.0f64, 0.0f64, 0.0f64);
    for c in inliers {
        let (da, db) = (c.along - mean_along, c.offset - mean_offset);
        m11 += c.weight * da * da;
        m12 += c.weight * da * db;
        m22 += c.weight * db * db;
    }
    Some((mean_along, mean_offset, principal_axis(m11, m12, m22)))
}

/// The direction of the principal axis of the weighted points: the one with the larger spread.
fn principal_axis(m11: f64, m12: f64, m22: f64) -> (f64, f64) {
    let (sin, cos) = (2.0 * m12, m11 - m22);
    let len = sin.hypot(cos);
    if len < 1e-12 {
        return if m11 >= m22 { (1.0, 0.0) } else { (0.0, 1.0) };
    }
    // The eigenvector of the larger eigenvalue.
    let (sin, cos) = (sin / len, cos / len);
    let angle = 0.5 * sin.atan2(cos);
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::{build_pyramid, refine};
    use crate::Polarity;
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::image::{Plane, PlaneF32};
    use scancraft_core::pyramid::LumaPyramid;
    use scancraft_raster::blur::gaussian;

    /// The photo the tests draw: 6 megapixels, so the pyramid has three levels (1024, 2048, 3000).
    const PHOTO: (u32, u32) = (3000, 2000);

    /// How far inside edge `a → b` a point lies, in pixels. The corners run clockwise on screen,
    /// so the page's inside is where the cross product is positive.
    fn depth(a: Point, b: Point, p: Point) -> f64 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        ((dx * (p.y - a.y) - dy * (p.x - a.x)) / dx.hypot(dy)).max(-1e9)
    }

    /// The share of the pixel `(x, y)` that lies inside the quadrilateral, found from a 4 × 4
    /// grid of samples; pixels far from every edge are decided at once.
    fn coverage(corners: &[Point; 4], x: u32, y: u32) -> f32 {
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
        inside as f32 / 16.0
    }

    /// A page of brightness `page` on a desk of brightness `desk`. Each pixel is the page's share
    /// of it, so an edge that falls between pixels is a real partial-coverage step.
    fn scene(corners: [Point; 4], page: f32, desk: f32) -> PlaneF32 {
        let (w, h) = PHOTO;
        let mut data = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = coverage(&corners, x, y);
                data.push(desk + (page - desk) * c);
            }
        }
        Plane::from_vec(w, h, data).unwrap()
    }

    /// A quadrilateral in photo pixels as the work image sees it (A10's input).
    fn in_work_pixels(pyramid: &LumaPyramid, photo: &Quad) -> Quad {
        let (Some(first), Some(full)) = (pyramid.levels.first(), pyramid.full()) else { return *photo };
        let (kx, ky) = (f64::from(first.width()) / f64::from(full.width()), f64::from(first.height()) / f64::from(full.height()));
        Quad(photo.0.map(|p| Point::new(p.x * kx, p.y * ky)))
    }

    /// The page the tests mostly use, with its edges between pixels and a slight tilt.
    fn truth() -> [Point; 4] {
        [Point::new(420.35, 310.62), Point::new(2610.48, 355.27), Point::new(2575.71, 1690.44), Point::new(455.58, 1650.83)]
    }

    /// Moves every corner of `quad` by a random amount of up to `reach` pixels each way.
    fn shaken(quad: [Point; 4], reach: f64, rng: &mut Xorshift64Star) -> [Point; 4] {
        quad.map(|p| Point::new(p.x + reach * (2.0 * rng.next_f64() - 1.0), p.y + reach * (2.0 * rng.next_f64() - 1.0)))
    }

    fn errors(found: &Quad, truth: &[Point; 4]) -> Vec<f64> {
        found.0.iter().zip(truth.iter()).map(|(a, b)| a.distance(*b)).collect()
    }

    fn mean_and_p95(mut values: Vec<f64>) -> (f64, f64) {
        values.sort_by(|a, b| a.total_cmp(b));
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let p95 = values[((values.len() as f64 * 0.95).ceil() as usize).clamp(1, values.len()) - 1];
        (mean, p95)
    }

    #[test]
    fn the_pyramid_doubles_until_the_photo_itself() {
        let luma = Plane::new(3000, 2000, 0.5f32).unwrap();
        let pyramid = build_pyramid(&luma).unwrap();
        let sizes: Vec<u32> = pyramid.levels.iter().map(|p| p.width().max(p.height())).collect();
        assert_eq!(sizes, vec![1024, 2048, 3000]);
        assert_eq!(pyramid.levels.last().unwrap().width(), 3000);
        // A small photo is one level.
        let small = Plane::new(400, 300, 0.5f32).unwrap();
        let pyramid = build_pyramid(&small).unwrap();
        assert_eq!(pyramid.levels.len(), 1);
        assert_eq!(pyramid.levels[0].width(), 400);
    }

    #[test]
    fn a_clean_page_is_found_to_a_fraction_of_a_pixel_from_a_rough_start() {
        // A10's first test: starting up to 25 px off (here up to 8 px in the work image, about 23
        // in the photo): mean error ≤ 0.3 px, P95 ≤ 0.8 px.
        let truth = truth();
        let pyramid = build_pyramid(&scene(truth, 0.9, 0.15)).unwrap();
        let mut rng = Xorshift64Star::new(11);
        let mut all = Vec::new();
        for run in 0..8u64 {
            let start = shaken(truth, 23.0, &mut rng);
            let refined = refine(&in_work_pixels(&pyramid, &Quad(start)), &pyramid, 100 + run).unwrap();
            assert_eq!(refined.unrefined, [false; 4], "run {run} gave up on a side");
            all.extend(errors(&refined.corners, &truth));
        }
        let (mean, p95) = mean_and_p95(all);
        assert!(mean <= 0.3, "mean error {mean:.3} px");
        assert!(p95 <= 0.8, "P95 error {p95:.3} px");
    }

    #[test]
    fn blur_and_noise_cost_a_little_accuracy() {
        // A10's second test: blur σ = 1.5 px and noise σ = 6 levels (6/255 of the range):
        // mean ≤ 0.6 px, P95 ≤ 1.5 px.
        let truth = truth();
        let mut luma = gaussian(&scene(truth, 0.9, 0.15), 1.5).unwrap();
        let mut rng = Xorshift64Star::new(5);
        for v in luma.data_mut() {
            let (a, b) = (rng.next_f64().max(1e-12), rng.next_f64());
            let g = (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos();
            *v += (6.0 / 255.0 * g) as f32;
        }
        let pyramid = build_pyramid(&luma).unwrap();
        let mut all = Vec::new();
        for run in 0..8u64 {
            let start = shaken(truth, 23.0, &mut rng);
            let refined = refine(&in_work_pixels(&pyramid, &Quad(start)), &pyramid, 200 + run).unwrap();
            all.extend(errors(&refined.corners, &truth));
        }
        let (mean, p95) = mean_and_p95(all);
        assert!(mean <= 0.6, "mean error {mean:.3} px");
        assert!(p95 <= 1.5, "P95 error {p95:.3} px");
    }

    #[test]
    fn an_exact_start_hardly_moves() {
        // "An exact starting quadrilateral moves by less than 0.2 px."
        let truth = truth();
        let pyramid = build_pyramid(&scene(truth, 0.9, 0.15)).unwrap();
        let refined = refine(&in_work_pixels(&pyramid, &Quad(truth)), &pyramid, 3).unwrap();
        let moved = errors(&refined.corners, &truth);
        assert!(moved.iter().all(|m| *m < 0.2), "the corners moved by {moved:?}");
    }

    #[test]
    fn refining_twice_changes_the_corners_by_less_than_a_third_of_a_pixel() {
        let truth = truth();
        let pyramid = build_pyramid(&scene(truth, 0.9, 0.15)).unwrap();
        let start = shaken(truth, 15.0, &mut Xorshift64Star::new(9));
        let once = refine(&in_work_pixels(&pyramid, &Quad(start)), &pyramid, 4).unwrap();
        let twice = refine(&in_work_pixels(&pyramid, &once.corners), &pyramid, 4).unwrap();
        let change = errors(&twice.corners, &once.corners.0);
        assert!(change.iter().all(|c| *c < 0.3), "the second pass moved the corners by {change:?}");
    }

    #[test]
    fn a_ruled_frame_inside_the_page_does_not_win() {
        // A frame 12 px inside the page is as strong as the paper's edge on any one scan line,
        // but the paper's edge is the outermost line that most scan lines agree on.
        let truth = truth();
        let (w, h) = PHOTO;
        let mut luma = scene(truth, 0.9, 0.15);
        let inner = [
            Point::new(truth[0].x + 12.0, truth[0].y + 12.0),
            Point::new(truth[1].x - 12.0, truth[1].y + 12.0),
            Point::new(truth[2].x - 12.0, truth[2].y - 12.0),
            Point::new(truth[3].x + 12.0, truth[3].y - 12.0),
        ];
        let inner_edge = [
            Point::new(inner[0].x + 3.0, inner[0].y + 3.0),
            Point::new(inner[1].x - 3.0, inner[1].y + 3.0),
            Point::new(inner[2].x - 3.0, inner[2].y - 3.0),
            Point::new(inner[3].x + 3.0, inner[3].y - 3.0),
        ];
        // A dark ruled line, 3 px wide: the page's share of the pixel inside `inner`, but not
        // inside `inner_edge`.
        for y in 0..h {
            for x in 0..w {
                let ring = coverage(&inner, x, y) - coverage(&inner_edge, x, y);
                if ring > 0.0
                    && let Some(v) = luma.get(x, y)
                {
                    luma.set(x, y, v + (0.2 - v) * ring);
                }
            }
        }
        let pyramid = build_pyramid(&luma).unwrap();
        let start = shaken(truth, 8.0, &mut Xorshift64Star::new(2));
        let refined = refine(&in_work_pixels(&pyramid, &Quad(start)), &pyramid, 6).unwrap();
        let found = errors(&refined.corners, &truth);
        assert!(found.iter().all(|e| *e < 2.0), "the frame won: errors {found:?}");
    }

    #[test]
    fn polarity_is_read_from_the_page_and_the_desk() {
        let truth = truth();
        for (page, desk, want) in [(0.9, 0.15, Polarity::PageLighter), (0.15, 0.9, Polarity::PageDarker)] {
            let pyramid = build_pyramid(&scene(truth, page, desk)).unwrap();
            let refined = refine(&in_work_pixels(&pyramid, &Quad(truth)), &pyramid, 7).unwrap();
            // The fallback would leave every side unrefined, so this also shows the work was done.
            assert_eq!(refined.unrefined, [false; 4], "page {page} on desk {desk}");
            assert_eq!(refined.polarity, [want; 4], "page {page} on desk {desk}");
        }
    }

    #[test]
    fn one_thread_and_eight_give_the_same_corners() {
        let truth = truth();
        let luma = scene(truth, 0.9, 0.15);
        let start = Quad(shaken(truth, 15.0, &mut Xorshift64Star::new(21)));
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                let pyramid = build_pyramid(&luma).unwrap();
                let refined = refine(&in_work_pixels(&pyramid, &start), &pyramid, 13).unwrap();
                (pyramid, refined)
            })
        };
        let (one_pyramid, one) = run(1);
        let (eight_pyramid, eight) = run(8);
        assert_eq!(one_pyramid, eight_pyramid);
        assert_eq!(one, eight);
    }

    #[test]
    fn sides_along_the_frame_of_a_flat_region_stay_on_the_frame() {
        // A page that spills past the top and the left of a 1200 × 900 photo: along those two
        // sides the luma is flat, and the profile has no step. A real photo's work image is made
        // by an area downscale, which leaves a ripple of one unit in the last place of an f32 at
        // the same columns of every row, a thousandth of one 8-bit level. A side must not be
        // fitted to it, as the tests here would if the ripple were a step: every scan line shows
        // it at the same place, so the candidates all agree.
        let (w, h) = (1200u32, 900u32);
        let mut data = vec![0.15f32; (w * h) as usize];
        for y in 0..800u32 {
            for x in 0..1080u32 {
                data[(y * w + x) as usize] = 0.9263 + 1e-7 * ((x * 7) % 5) as f32;
            }
        }
        let pyramid = build_pyramid(&Plane::from_vec(w, h, data).unwrap()).unwrap();
        let start = Quad([Point::new(0.0, 0.0), Point::new(921.6, 0.0), Point::new(921.6, 682.7), Point::new(0.0, 682.7)]);
        let refined = refine(&start, &pyramid, 5).unwrap();
        assert!(refined.unrefined[0] && refined.unrefined[3], "top and left have no step: {:?}", refined.unrefined);
        assert!(!refined.unrefined[1] && !refined.unrefined[2], "right and bottom do: {:?}", refined.unrefined);
        let (tl, bl) = (refined.corners.tl(), refined.corners.bl());
        assert!(tl.x.abs() < 0.01 && tl.y.abs() < 0.01, "the top left corner moved to {tl:?}");
        assert!(bl.x.abs() < 0.01, "the bottom left corner moved to {bl:?}");
        // The page's right and bottom edges are at x = 1080 and y = 800 in the photo.
        let br = refined.corners.br();
        assert!((br.x - 1080.0).abs() < 0.5 && (br.y - 800.0).abs() < 0.5, "the bottom right corner is at {br:?}");
    }

    #[test]
    fn a_start_that_is_far_from_any_page_comes_back_unchanged() {
        // On a plain desk there is no step to find: every side is kept as it was.
        let pyramid = build_pyramid(&Plane::new(3000, 2000, 0.4f32).unwrap()).unwrap();
        let start = Quad([Point::new(200.0, 150.0), Point::new(800.0, 150.0), Point::new(800.0, 600.0), Point::new(200.0, 600.0)]);
        let refined = refine(&start, &pyramid, 1).unwrap();
        assert_eq!(refined.unrefined, [true; 4]);
        // The work image is 1024 × 683, so the two axes scale a little differently.
        let (kx, ky) = (3000.0 / f64::from(pyramid.levels[0].width()), 2000.0 / f64::from(pyramid.levels[0].height()));
        for (got, want) in refined.corners.0.iter().zip(start.0.iter()) {
            assert!(got.distance(Point::new(want.x * kx, want.y * ky)) < 0.5, "{got:?} against {want:?}");
        }
    }
}
