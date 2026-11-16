//! A30: book spreads (specification A30). Owner: WP10.
//!
//! A photo of an open book holds two pages joined at the spine. The fold between them, the
//! gutter, is a straight line in the photo, because it is a ruling of both pages. It is usually
//! the darkest column of the spread, and no line of text crosses it.

use crate::DetectError;
use crate::text_lines::{Copy, copy_for, lines_in_copy};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{Plane, PlaneF32};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{Half, SideShape};
use scancraft_raster::blur::gaussian;

/// The long side of the spread's copy (step 1).
const COPY_LONG_SIDE: u32 = 1000;
/// The share of the rows the column profile is taken over (step 2).
const PROFILE_ROWS: f64 = 0.8;
/// The share of the width the profile is smoothed by, as σ (step 2).
const PROFILE_SIGMA: f64 = 0.01;
/// The middle of the width the gutter is looked for in, as a share on each side (step 3).
const GUTTER_MIDDLE: f64 = 0.3;
/// How much darker than the paper the gutter must be (step 3).
const GUTTER_DEPTH: f64 = 0.06;
/// The share of the text lines that may cross the gutter (step 3).
const GUTTER_CROSSING: f64 = 0.10;
/// The bands the height is split into for the spine's fit (step 4).
const SPINE_BANDS: usize = 8;
/// How far from the gutter the spine's bands look for their minimum (step 4).
const SPINE_SEARCH: f64 = 0.03;
/// How far a point may be from the spine's line and still be used (step 4).
const SPINE_FIT: f64 = 0.01;
/// The ratio a spread must have for a cut at the centre (step 5).
const SPREAD_RATIO_MIN: f64 = 1.2;
const SPREAD_RATIO_MAX: f64 = 1.7;
/// The share of the spread's width each page gives up along the spine (step 6).
const SPINE_CESSION: f64 = 0.005;
/// The short side of a spread of standard paper, in millimetres, for the copy's density.
const NOMINAL_SHORT_MM: f64 = 297.0;

/// A spine found by steps 1–5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spine {
    /// Top and bottom ends in photo pixels.
    pub ends: [Point; 2],
    /// Step 3's confidence; 0 for a cut at the centre line.
    pub confidence: f32,
}

/// One page of a split spread (step 6).
#[derive(Debug, Clone, PartialEq)]
pub struct HalfOutline {
    pub half: Half,
    pub corners: Quad,
    pub sides: [SideShape; 4],
}

/// Steps 1–5: `ratio` is A11's ratio of the whole outline. `None` means one page.
pub fn find_spine(pyramid: &LumaPyramid, outline: &Quad, ratio: f64) -> Result<Option<Spine>, DetectError> {
    if !outline.is_finite() {
        return Err(scancraft_raster::RasterError::BadParameter("the outline is not finite").into());
    }
    if !(ratio.is_finite() && ratio > 0.0) {
        return Err(scancraft_raster::RasterError::BadParameter("the spread's ratio must be a positive number").into());
    }
    // Step 1: the copy, and the size the spread would have.
    let size = spread_size_mm(ratio);
    let copy = copy_for(pyramid, outline, COPY_LONG_SIDE, size)?;
    // Step 2: the column profile.
    let profile = column_profile(&copy.plane)?;
    // Step 3: the gutter, and the text lines that cross it.
    let Some(gutter) = find_gutter(&profile, &copy)? else {
        // Step 5: no gutter. A spread of two pages side by side is cut at the centre.
        if (SPREAD_RATIO_MIN..=SPREAD_RATIO_MAX).contains(&ratio) {
            return Ok(Some(centre_spine(&copy)?));
        }
        return Ok(None);
    };
    // Step 4: the spine, in the copy's columns.
    let (a, b) = fit_spine(&copy.plane, &gutter)?;
    let height = f64::from(copy.plane.height());
    let top = Point::new(a, 0.5);
    let bottom = Point::new(a + b * height, height - 0.5);
    let (Some(top_photo), Some(bottom_photo)) = (copy.photo_position(top.x, top.y), copy.photo_position(bottom.x, bottom.y)) else {
        return Ok(None);
    };
    Ok(Some(Spine { ends: [top_photo, bottom_photo], confidence: gutter.confidence }))
}

/// Step 6: cuts the outline and its sides along the spine; left page first.
pub fn split(outline: &Quad, sides: &[SideShape; 4], spine: &Spine) -> Result<[HalfOutline; 2], DetectError> {
    if !outline.is_finite() {
        return Err(scancraft_raster::RasterError::BadParameter("the outline is not finite").into());
    }
    let [top, bottom] = spine.ends;
    if !top.is_finite() || !bottom.is_finite() {
        return Err(scancraft_raster::RasterError::BadParameter("the spine's ends are not finite").into());
    }
    // The spine's line, in photo pixels.
    let direction = Point::new(bottom.x - top.x, bottom.y - top.y);
    let length = direction.distance(Point::new(0.0, 0.0));
    if length <= 0.0 {
        return Err(scancraft_raster::RasterError::BadParameter("the spine's ends are the same point").into());
    }
    // The cession: each page gives up 0.5 % of the spread's width along the spine, so that the
    // dark gutter is not baked into either page.
    let spread_width = outline.tl().distance(outline.tr()).max(1.0);
    let cession = SPINE_CESSION * spread_width;
    // A normal to the spine, pointing to the right of the page.
    let normal = Point::new(direction.y / length, -direction.x / length);
    let left_top = Point::new(top.x - normal.x * cession, top.y - normal.y * cession);
    let left_bottom = Point::new(bottom.x - normal.x * cession, bottom.y - normal.y * cession);
    let right_top = Point::new(top.x + normal.x * cession, top.y + normal.y * cession);
    let right_bottom = Point::new(bottom.x + normal.x * cession, bottom.y + normal.y * cession);

    // Each page's spine corners: where its own edge meets the spread's top and bottom sides,
    // along their A25 paths when those are curved.
    let left_tr = side_crossing(&[outline.tl(), outline.tr()], &sides[0], left_top, left_bottom).unwrap_or(left_top);
    let left_br = side_crossing(&[outline.bl(), outline.br()], &sides[2], left_top, left_bottom).unwrap_or(left_bottom);
    let right_tl = side_crossing(&[outline.tl(), outline.tr()], &sides[0], right_top, right_bottom).unwrap_or(right_top);
    let right_bl = side_crossing(&[outline.bl(), outline.br()], &sides[2], right_top, right_bottom).unwrap_or(right_bottom);

    // The left page: the spread's left side and its own edge of the gutter.
    let left_corners = Quad([outline.tl(), left_tr, left_br, outline.bl()]);
    // The right page: its own edge of the gutter and the spread's right side.
    let right_corners = Quad([right_tl, outline.tr(), outline.br(), right_bl]);

    Ok([
        HalfOutline {
            half: Half::Left,
            corners: left_corners,
            sides: [half_side(&sides[0], &top, &bottom, true), SideShape::Spine, half_side(&sides[2], &top, &bottom, true), sides[3].clone()],
        },
        HalfOutline {
            half: Half::Right,
            corners: right_corners,
            sides: [half_side(&sides[0], &top, &bottom, false), sides[1].clone(), half_side(&sides[2], &top, &bottom, false), SideShape::Spine],
        },
    ])
}

/// The spread's size in millimetres, from its ratio: two pages of standard paper side by side.
fn spread_size_mm(ratio: f64) -> (f64, f64) {
    if ratio >= 1.0 { (NOMINAL_SHORT_MM * ratio, NOMINAL_SHORT_MM) } else { (NOMINAL_SHORT_MM, NOMINAL_SHORT_MM / ratio) }
}

/// Step 2: the mean luma of each column over the middle 80 % of the rows, smoothed with a Gaussian
/// whose σ is 1 % of the width.
fn column_profile(plane: &PlaneF32) -> Result<PlaneF32, DetectError> {
    let (w, h) = (plane.width() as usize, plane.height() as usize);
    if w == 0 || h == 0 {
        return Err(scancraft_raster::RasterError::BadParameter("the copy is empty").into());
    }
    let margin = ((1.0 - PROFILE_ROWS) * h as f64 / 2.0).round() as usize;
    let y0 = margin.min(h - 1);
    let y1 = (h - margin).max(y0 + 1).min(h);
    let mut data = vec![0f32; w];
    for (x, slot) in data.iter_mut().enumerate() {
        let mut sum = 0f64;
        let mut count = 0u32;
        for y in y0..y1 {
            sum += f64::from(plane.get(x as u32, y as u32).unwrap_or(0.0));
            count += 1;
        }
        *slot = if count > 0 { (sum / f64::from(count)) as f32 } else { 0.0 };
    }
    let profile = Plane::from_vec(w as u32, 1, data)?;
    let smoothed = gaussian(&profile, (PROFILE_SIGMA * w as f64).max(0.3) as f32)?;
    Ok(smoothed)
}

/// The gutter of a spread.
struct Gutter {
    /// The gutter's column in the copy.
    column: f64,
    /// How much darker it is than the paper, as a fraction.
    depth: f64,
    /// Step 3's confidence.
    confidence: f32,
}

/// Step 3: the deepest local minimum in the middle of the width that no text line crosses.
fn find_gutter(profile: &PlaneF32, copy: &Copy) -> Result<Option<Gutter>, DetectError> {
    let w = profile.width() as usize;
    if w < 8 {
        return Ok(None);
    }
    let values: Vec<f64> = profile.data().iter().map(|&v| f64::from(v)).collect();
    // The paper's level: the mean of the two halves' medians.
    let half = w / 2;
    let left = median_of(&values[..half]);
    let right = median_of(&values[half..]);
    let paper = (left + right) / 2.0;
    if paper <= 1e-6 {
        return Ok(None);
    }
    // The text lines on this copy, and the ones that cross the gutter.
    let dpi = copy_density(copy);
    let lines = lines_in_copy(&copy.plane, dpi)?;
    let lowest = GUTTER_MIDDLE * w as f64;
    let mut best: Option<Gutter> = None;
    for x in (lowest as usize)..((w as f64 - lowest) as usize).min(w.saturating_sub(1)) {
        let value = values.get(x).copied().unwrap_or(0.0);
        let before = values.get(x.wrapping_sub(1)).copied().unwrap_or(value);
        let after = values.get(x + 1).copied().unwrap_or(value);
        if value > before || value > after {
            continue;
        }
        let depth = (paper - value) / paper;
        if depth < GUTTER_DEPTH {
            continue;
        }
        // At most a tenth of the lines may cross the gutter.
        let crossing = lines.lines.iter().filter(|line| crosses(line, x as f64)).count();
        if !lines.lines.is_empty() && crossing as f64 > GUTTER_CROSSING * lines.lines.len() as f64 {
            continue;
        }
        let confidence = ((depth - GUTTER_DEPTH) / GUTTER_DEPTH).min(1.0) as f32;
        let better = best.as_ref().is_none_or(|g| value < g.depth * paper);
        if better {
            best = Some(Gutter { column: x as f64, depth, confidence });
        }
    }
    Ok(best)
}

/// Does the line cross the column?
fn crosses(line: &[Point], column: f64) -> bool {
    let Some(first) = line.first() else { return false };
    let Some(last) = line.last() else { return false };
    let (low, high) = (first.x.min(last.x), first.x.max(last.x));
    column >= low && column <= high
}

/// The copy's density in pixels per inch, from the spread's nominal size.
fn copy_density(copy: &Copy) -> f64 {
    // The spread's long side is a nominal 297 mm; the copy's long side is 1000 px.
    f64::from(copy.plane.width().max(copy.plane.height())) / (NOMINAL_SHORT_MM * 1.5 / 25.4)
}

/// Step 4: the spine's line, fitted through the profile's minimum in each band.
///
/// The height is split into eight bands, and in each the column profile — the mean luma of each
/// column over that band — is searched for its minimum within 3 % of the width of the gutter. The
/// eight points are fitted with a least-squares line, the points more than 1 % of the width from it
/// are dropped, and the line is fitted again.
fn fit_spine(plane: &PlaneF32, gutter: &Gutter) -> Result<(f64, f64), DetectError> {
    let (w, h) = (plane.width() as usize, plane.height() as usize);
    let search = SPINE_SEARCH * w as f64;
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(SPINE_BANDS);
    for band in 0..SPINE_BANDS {
        let y0 = band * h / SPINE_BANDS;
        let y1 = ((band + 1) * h / SPINE_BANDS).max(y0 + 1).min(h);
        // The band's column profile.
        let mut band_profile = vec![0f32; w];
        for (x, slot) in band_profile.iter_mut().enumerate() {
            let mut sum = 0f64;
            let mut count = 0u32;
            for y in y0..y1 {
                sum += f64::from(plane.get(x as u32, y as u32).unwrap_or(0.0));
                count += 1;
            }
            *slot = if count > 0 { (sum / f64::from(count)) as f32 } else { 0.0 };
        }
        let smoothed = gaussian(&Plane::from_vec(w as u32, 1, band_profile)?, (PROFILE_SIGMA * w as f64).max(0.3) as f32)?;
        let from = (gutter.column - search).max(0.0) as usize;
        let to = ((gutter.column + search) as usize).min(w.saturating_sub(1));
        if from > to {
            continue;
        }
        // The band's darkest column within the window.
        let mut best = (f64::INFINITY, from);
        for x in from..=to {
            let value = f64::from(smoothed.data().get(x).copied().unwrap_or(0.0));
            if value < best.0 {
                best = (value, x);
            }
        }
        points.push((best.1 as f64, (y0 as f64 + y1 as f64) / 2.0));
    }
    let Some((a, b)) = fit_x_on_y(&points) else {
        return Ok((gutter.column, 0.0));
    };
    // The points more than 1 % of the width from the line are dropped, and the line is fitted again.
    let kept: Vec<(f64, f64)> = points.iter().copied().filter(|(x, y)| (x - (a + b * y)).abs() <= SPINE_FIT * w as f64).collect();
    match fit_x_on_y(&kept) {
        Some((a2, b2)) => Ok((a2, b2)),
        None => Ok((a, b)),
    }
}

/// The least-squares fit of x = a + b·y through the points.
fn fit_x_on_y(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let (mut sx, mut sy) = (0f64, 0f64);
    for (x, y) in points {
        sx += x;
        sy += y;
    }
    let (mx, my) = (sx / n, sy / n);
    let mut num = 0f64;
    let mut den = 0f64;
    for (x, y) in points {
        num += (y - my) * (x - mx);
        den += (y - my) * (y - my);
    }
    if den <= 1e-12 {
        return None;
    }
    let b = num / den;
    Some((mx - b * my, b))
}

/// Step 5: the cut at the centre line.
fn centre_spine(copy: &Copy) -> Result<Spine, DetectError> {
    let width = f64::from(copy.plane.width());
    let height = f64::from(copy.plane.height());
    let top = Point::new(width / 2.0, 0.5);
    let bottom = Point::new(width / 2.0, height - 0.5);
    let (Some(top_photo), Some(bottom_photo)) = (copy.photo_position(top.x, top.y), copy.photo_position(bottom.x, bottom.y)) else {
        return Err(scancraft_raster::RasterError::BadParameter("the spread's mapping is undefined at its centre").into());
    };
    Ok(Spine { ends: [top_photo, bottom_photo], confidence: 0.0 })
}

/// Where the spine meets a side of the spread: on its chord, or on its traced path.
fn side_crossing(chord: &[Point; 2], side: &SideShape, top: Point, bottom: Point) -> Option<Point> {
    match side {
        SideShape::Traced { samples, .. } => {
            // The first segment of the path that the spine crosses.
            for pair in samples.windows(2) {
                if let Some(point) = segment_crossing(&[pair[0].point, pair[1].point], top, bottom) {
                    return Some(point);
                }
            }
            None
        }
        _ => segment_crossing(chord, top, bottom),
    }
}

/// Where two lines cross.
fn segment_crossing(segment: &[Point; 2], top: Point, bottom: Point) -> Option<Point> {
    let (a, b) = (segment[0], segment[1]);
    let (p, q) = (top, bottom);
    let r = (b.x - a.x, b.y - a.y);
    let s = (q.x - p.x, q.y - p.y);
    let denominator = r.0 * s.1 - r.1 * s.0;
    if denominator.abs() < 1e-12 {
        return None;
    }
    let t = ((p.x - a.x) * s.1 - (p.y - a.y) * s.0) / denominator;
    let u = ((p.x - a.x) * r.1 - (p.y - a.y) * r.0) / denominator;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    Some(Point::new(a.x + t * r.0, a.y + t * r.1))
}

/// The part of a side that belongs to one half of the spread.
fn half_side(side: &SideShape, top: &Point, bottom: &Point, left: bool) -> SideShape {
    match side {
        SideShape::Traced { samples, bow, curved } => {
            // The samples on this side of the spine.
            let kept: Vec<scancraft_core::recipe::SideSample> =
                samples.iter().copied().filter(|s| on_the_left(&s.point, top, bottom) == left).collect();
            SideShape::Traced { samples: kept, bow: *bow, curved: *curved }
        }
        other => other.clone(),
    }
}

/// Is the point on the left of the spine, seen from the top towards the bottom?
fn on_the_left(point: &Point, top: &Point, bottom: &Point) -> bool {
    let (dx, dy) = (bottom.x - top.x, bottom.y - top.y);
    let (px, py) = (point.x - top.x, point.y - top.y);
    // The cross product's sign: positive when the point is to the left of the direction.
    dx * py - dy * px > 0.0
}

/// The median of the values.
fn median_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}
