//! A26: text lines, a port of pdfcraft-android's strip tracker (specification A26). Owner: WP10.
//!
//! On a page of text, each line is a horizontal band of ink. A26 cuts the straightened page into
//! narrow vertical strips, finds the bands of ink in each strip, and follows each band from strip
//! to strip. The result is a few dozen points along the middle of every line of text.
//!
//! The ink is found by A18's local rule rather than one threshold for the whole page, so that the
//! lines survive the shade of a hand or of a book's gutter.

use crate::DetectError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{BitPlane, PlaneF32};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_geom::homography::Homography;
use scancraft_geom::sheet::{LineEvidence, TextDirection};
use scancraft_raster::warp::warp_luma;

/// The long side of the analysis copy (step 1).
const COPY_LONG_SIDE: u32 = 1600;
/// The number of vertical strips the copy is cut into (step 3).
const STRIPS: usize = 32;
/// The share of a strip's highest count that a row must hold to be part of a piece (step 3).
const PIECE_SHARE: f32 = 0.15;
/// The least height of a piece, in rows (step 3).
const PIECE_MIN_ROWS: usize = 3;
/// The share of a strip's median gap that two heights may differ by and still join (step 4).
const JOIN_SHARE: f64 = 0.6;
/// The least strips a line must be seen in (step 5).
const MIN_STRIPS: usize = 5;
/// The least distance between a line's first and last points, as a share of the copy's width (step 5).
const MIN_SPAN: f64 = 0.3;
/// The most lines kept (step 5).
const MAX_LINES: usize = 120;
/// How much longer the turned direction's lines must be to be used (step 6).
const DIRECTION_MARGIN: f64 = 1.5;
/// The strength of A18's ink rule (step 2).
const INK_STRENGTH: u8 = 55;
/// The least uncertainty of a point, in the copy's pixels (step 7).
const MIN_SIGMA: f64 = 1.5;
/// The share of the median piece height that a point's uncertainty is (step 7).
const SIGMA_SHARE: f64 = 0.12;

/// Lines found on an analysis copy, in the copy's pixels (A26 steps 2–6).
#[derive(Debug, Clone, PartialEq)]
pub struct CopyLines {
    pub lines: Vec<Vec<Point>>,
    pub direction: TextDirection,
    /// The median piece height, for step 7's σ.
    pub median_piece_height: f64,
}

/// Steps 2–6 on a copy at `dpi` pixels per inch. A29 and A30 call this on their own copies.
pub fn lines_in_copy(copy: &PlaneF32, dpi: f64) -> Result<CopyLines, DetectError> {
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(scancraft_raster::RasterError::BadParameter("the copy's density must be a positive number").into());
    }
    // Step 2: the ink, by A18's local rule.
    let ink = scancraft_raster::ink::midpoint_ink(copy, dpi, INK_STRENGTH)?;
    let across = track(&ink)?;
    // Step 6: the same tracker on the copy turned a quarter turn.
    let turned_ink = turn(&ink)?;
    let turned = track(&turned_ink)?;
    // The direction whose lines are longer: the turned copy's lines, when they are at least 1.5
    // times as long as the first's. The length of a direction is the mean length of its lines: the
    // total alone would favour the direction with the most lines, and the words of a line make
    // many short columns beside the few long lines of the text.
    let (across_length, turned_length) = (mean_length(&across.lines), mean_length(&turned.lines));
    let use_turned = turned_length > 0.0 && turned_length >= DIRECTION_MARGIN * across_length;
    let (lines, direction, piece_height) = if use_turned {
        (unturn(&turned.lines, turned_ink.height()), TextDirection::Down, turned.median_piece_height)
    } else {
        (across.lines, TextDirection::Across, across.median_piece_height)
    };
    Ok(CopyLines { lines, direction, median_piece_height: piece_height })
}

/// Steps 1–7 for a page outline: the evidence A27 fits to, in photo pixels. `None` when fewer
/// than 4 lines are found.
pub fn text_evidence(
    pyramid: &LumaPyramid,
    corners: &Quad,
    page_size_mm: (f64, f64),
) -> Result<Option<(Vec<LineEvidence>, TextDirection)>, DetectError> {
    if !corners.is_finite() {
        return Err(scancraft_raster::RasterError::BadParameter("the corners are not finite").into());
    }
    let (width_mm, height_mm) = page_size_mm;
    if !(width_mm.is_finite() && height_mm.is_finite() && width_mm > 0.0 && height_mm > 0.0) {
        return Err(scancraft_raster::RasterError::BadParameter("the page's size is not a positive number").into());
    }
    // Step 1: the analysis copy.
    let copy = copy_for(pyramid, corners, COPY_LONG_SIDE, (width_mm, height_mm))?;
    let dpi = f64::from(copy.plane.width().max(copy.plane.height())) / (width_mm.max(height_mm) / 25.4);
    // Steps 2 to 6.
    let found = lines_in_copy(&copy.plane, dpi)?;
    // Step 7: the photo positions.
    let sigma = MIN_SIGMA.max(SIGMA_SHARE * found.median_piece_height);
    let mut lines = Vec::with_capacity(found.lines.len());
    for line in &found.lines {
        let mut points = Vec::with_capacity(line.len());
        let mut sigmas = Vec::with_capacity(line.len());
        for point in line {
            let Some(photo) = copy.photo_position(point.x, point.y) else { continue };
            points.push(photo);
            // σ in the copy's pixels, converted to the photo's by the mapping's local scale.
            sigmas.push(sigma * copy.scale_at(point.x, point.y));
        }
        if points.len() >= 2 {
            lines.push(LineEvidence { points, sigma: sigmas });
        }
    }
    // Fewer than four lines means the page offers no text evidence.
    if lines.len() < 4 {
        return Ok(None);
    }
    Ok(Some((lines, found.direction)))
}

/// The analysis copy of a page or spread, and the mapping that made it.
pub(crate) struct Copy {
    /// The straightened page, in the pyramid level's pixels.
    pub plane: PlaneF32,
    /// The homography from the unit square to the outline, in the level's pixels.
    pub homography: Homography,
    /// The level's size, and the photo's, so that a position can be mapped back.
    pub level: (u32, u32),
    pub photo: (u32, u32),
}

impl Copy {
    /// The photo position of the copy's pixel (x, y).
    pub fn photo_position(&self, x: f64, y: f64) -> Option<Point> {
        let u = (x + 0.5) / f64::from(self.plane.width()).max(1.0);
        let v = (y + 0.5) / f64::from(self.plane.height()).max(1.0);
        let p = self.homography.apply(u, v)?;
        let (sx, sy) = (f64::from(self.level.0) / f64::from(self.photo.0).max(1.0), f64::from(self.level.1) / f64::from(self.photo.1).max(1.0));
        Some(Point::new(p.x / sx.max(1e-9), p.y / sy.max(1e-9)))
    }

    /// The scale of the mapping at a copy pixel: how many photo pixels one copy pixel is.
    pub fn scale_at(&self, x: f64, y: f64) -> f64 {
        let u = (x + 0.5) / f64::from(self.plane.width()).max(1.0);
        let v = (y + 0.5) / f64::from(self.plane.height()).max(1.0);
        self.scale(u, v)
    }

    /// The scale of the mapping at a page point: how many photo pixels one copy pixel is.
    ///
    /// The homography's Jacobian gives the level's pixels per unit of the page's coordinates; the
    /// copy's pixel is a step of 1/W and 1/H in those coordinates, and the level is smaller than
    /// the photo by its own scale. The geometric mean of the two axes' scale is the number of
    /// photo pixels one copy pixel is.
    pub fn scale(&self, u: f64, v: f64) -> f64 {
        let (sx, sy) = (f64::from(self.level.0) / f64::from(self.photo.0).max(1.0), f64::from(self.level.1) / f64::from(self.photo.1).max(1.0));
        let [a, b, c, d, e, f, g, h] = self.homography.m;
        let w = g * u + h * v + 1.0;
        if w == 0.0 || !w.is_finite() {
            return 1.0;
        }
        let (nx, ny) = (a * u + b * v + c, d * u + e * v + f);
        // The partial derivatives of the homography.
        let dxdu = (a * w - nx * g) / (w * w);
        let dxdv = (b * w - nx * h) / (w * w);
        let dydu = (d * w - ny * g) / (w * w);
        let dydv = (e * w - ny * h) / (w * w);
        // The copy's pixels: a step of 1/W and 1/H in the page's coordinates, and the level's
        // pixels are the photo's by the level's scale.
        let (du, dv) = (1.0 / f64::from(self.plane.width()).max(1.0), 1.0 / f64::from(self.plane.height()).max(1.0));
        let j11 = dxdu * du / sx.max(1e-9);
        let j12 = dxdv * dv / sx.max(1e-9);
        let j21 = dydu * du / sy.max(1e-9);
        let j22 = dydv * dv / sy.max(1e-9);
        (j11 * j22 - j12 * j21).abs().sqrt().max(1e-9)
    }
}

/// The copy of a page or spread with the given outline, and the mapping that made it.
///
/// The outline is in photo pixels; the level of the pyramid may be smaller, so the outline is
/// scaled into the level's pixels before the homography is built.
pub(crate) fn copy_for(pyramid: &LumaPyramid, outline: &Quad, long_side: u32, size_mm: (f64, f64)) -> Result<Copy, DetectError> {
    let level = pyramid.at_least(long_side).ok_or(scancraft_core::CoreError::BadSize { width: 0, height: 0 })?;
    let photo = pyramid.full().map(|p| (p.width(), p.height())).unwrap_or((level.width(), level.height()));
    let (lw, lh) = (level.width().max(1), level.height().max(1));
    let (pw, ph) = (photo.0.max(1), photo.1.max(1));
    let (sx, sy) = (f64::from(lw) / f64::from(pw), f64::from(lh) / f64::from(ph));
    let scaled = Quad(outline.0.map(|p| Point::new(p.x * sx, p.y * sy)));
    let homography = Homography::from_unit_square(&scaled, f64::from(lw.max(lh)))?;
    // The copy's size: the long side asked for, and the page's true aspect.
    let (width_mm, height_mm) = size_mm;
    let (out_w, out_h) = if width_mm >= height_mm {
        (long_side.max(1), ((f64::from(long_side) * height_mm / width_mm).round() as u32).max(1))
    } else {
        (((f64::from(long_side) * width_mm / height_mm).round() as u32).max(1), long_side.max(1))
    };
    let plane = warp_luma(level, &homography, out_w, out_h)?;
    Ok(Copy { plane, homography, level: (lw, lh), photo: (pw, ph) })
}

/// One piece of a text line, in a strip.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    /// The height of the piece's ink, in the copy's rows.
    height: f64,
}

/// The lines the tracker found in one direction.
#[derive(Debug, Clone, PartialEq)]
struct Tracked {
    lines: Vec<Vec<Point>>,
    /// The median height of the pieces, in the copy's rows.
    median_piece_height: f64,
}

/// Steps 3–5: the pieces of the lines, the following, and the keeping.
fn track(ink: &BitPlane) -> Result<Tracked, DetectError> {
    let (w, h) = (ink.width(), ink.height());
    if w == 0 || h == 0 {
        return Ok(Tracked { lines: Vec::new(), median_piece_height: 0.0 });
    }
    // The page's median gap between the neighbouring pieces: the widest tolerance any strip may
    // use, so that a line cannot jump to another band where the text thins out.
    let strips = pieces_by_strip(ink);
    let page_gap = {
        let gaps: Vec<f64> = strips.iter().filter_map(|s| median_gap(s)).collect();
        let mut sorted = gaps.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
    };
    let lines = follow(&strips, w, page_gap);
    let mut heights: Vec<f64> = strips.iter().flat_map(|s| s.iter().map(|p| p.height)).collect();
    heights.sort_by(f64::total_cmp);
    let median_piece_height = heights.get(heights.len() / 2).copied().unwrap_or(0.0);
    // Step 5: the lines seen in at least five strips, spanning 30 % of the width.
    let span = MIN_SPAN * f64::from(w);
    let kept: Vec<Vec<Point>> = lines
        .into_iter()
        .filter(|points| points.len() >= MIN_STRIPS)
        .filter(|points| {
            let first = points.first().map(|p| p.x).unwrap_or(0.0);
            let last = points.last().map(|p| p.x).unwrap_or(0.0);
            (last - first).abs() >= span
        })
        .take(MAX_LINES)
        .collect();
    Ok(Tracked { lines: kept, median_piece_height })
}

/// Step 3: the pieces of the lines in each strip.
fn pieces_by_strip(ink: &BitPlane) -> Vec<Vec<Piece>> {
    let (w, h) = (ink.width() as usize, ink.height() as usize);
    let mut out = Vec::with_capacity(STRIPS);
    for strip in 0..STRIPS {
        let x0 = strip * w / STRIPS;
        let x1 = (((strip + 1) * w) / STRIPS).max(x0 + 1).min(w);
        // The raw counts of the ink in each row of the strip.
        let mut counts = vec![0f32; h];
        for (y, slot) in counts.iter_mut().enumerate() {
            let mut count = 0u32;
            for x in x0..x1 {
                if ink.get(x as u32, y as u32) == Some(true) {
                    count += 1;
                }
            }
            *slot = count as f32;
        }
        // The counts smoothed over three rows, the edge row repeated at the border.
        let mut smoothed = vec![0f32; h];
        for (y, slot) in smoothed.iter_mut().enumerate() {
            let a = counts[y.saturating_sub(1)];
            let b = counts[y];
            let c = counts[(y + 1).min(h - 1)];
            *slot = (a + b + c) / 3.0;
        }
        let highest = smoothed.iter().copied().fold(0.0f32, f32::max);
        if highest <= 0.0 {
            out.push(Vec::new());
            continue;
        }
        let threshold = PIECE_SHARE * highest;
        // The runs of rows over the threshold, at least three rows tall.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut start: Option<usize> = None;
        // A sentinel row at the end, so that the last run is closed.
        let mut rows = smoothed.clone();
        rows.push(0.0);
        for (y, &count) in rows.iter().enumerate() {
            let over = count > threshold;
            match (over, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => {
                    let end = y - 1;
                    if end + 1 - s >= PIECE_MIN_ROWS {
                        runs.push((s, end));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        // The height of a run: the centre of mass of the raw counts over it.
        let height_of = |run: (usize, usize)| -> f64 {
            let (s, end) = run;
            let mut sum = 0f64;
            let mut moment = 0f64;
            for (row, &raw) in counts.iter().enumerate().take(end + 1).skip(s) {
                let c = f64::from(raw);
                sum += c;
                moment += c * (row as f64 + 0.5);
            }
            if sum > 0.0 { moment / sum } else { (s as f64 + end as f64) / 2.0 + 0.5 }
        };
        let pieces: Vec<Piece> = runs.iter().map(|&run| Piece { height: height_of(run) }).collect();
        out.push(pieces);
    }
    out
}

/// The median gap between the neighbouring pieces of a strip.
fn median_gap(pieces: &[Piece]) -> Option<f64> {
    if pieces.len() < 2 {
        return None;
    }
    let mut heights: Vec<f64> = pieces.iter().map(|p| p.height).collect();
    heights.sort_by(f64::total_cmp);
    let mut gaps: Vec<f64> = heights.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f64::total_cmp);
    gaps.get(gaps.len() / 2).copied()
}

/// The median height of all the pieces.
fn median_height(strips: &[Vec<Piece>]) -> f64 {
    let mut heights: Vec<f64> = strips.iter().flat_map(|s| s.iter().map(|p| p.height)).collect();
    heights.sort_by(f64::total_cmp);
    heights.get(heights.len() / 2).copied().unwrap_or(0.0)
}

/// Step 4: following the pieces from strip to strip.
fn follow(strips: &[Vec<Piece>], width: u32, page_gap: f64) -> Vec<Vec<Point>> {
    /// A line being followed.
    struct Open {
        points: Vec<Point>,
        /// The strips it has missed in a row.
        misses: u32,
    }
    let mut open: Vec<Open> = Vec::new();
    let mut done: Vec<Vec<Point>> = Vec::new();
    // The last strip's median gap, for a strip with too few pieces to have one of its own.
    let mut carried: Option<f64> = None;
    for (index, pieces) in strips.iter().enumerate() {
        // The tolerance: 60 % of the strip's median gap between the neighbouring pieces. A strip
        // with too few pieces to measure a gap uses the last strip's, and no strip's gap is wider
        // than the page's, so that a line cannot jump to another part of its own band where the
        // text thins out.
        let gap = if pieces.len() >= 3 { median_gap(pieces) } else { None };
        let gap = gap.map(|g| g.min(page_gap));
        let tolerance = JOIN_SHARE * gap.or(carried).unwrap_or_else(|| median_height(strips));
        carried = gap.or(carried);
        // The strip's centre column, in the copy's pixels.
        let x = (index as f64 + 0.5) / STRIPS as f64 * f64::from(width);
        // The pairs within the tolerance, nearest heights first.
        let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
        for (line, o) in open.iter().enumerate() {
            let height = o.points.last().map(|p| p.y).unwrap_or(0.0);
            for (piece, p) in pieces.iter().enumerate() {
                let distance = (p.height - height).abs();
                if distance <= tolerance {
                    pairs.push((distance, line, piece));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Each line and each piece is used once.
        let mut used_lines = vec![false; open.len()];
        let mut used_pieces = vec![false; pieces.len()];
        for (_, line, piece) in pairs {
            if used_lines[line] || used_pieces[piece] {
                continue;
            }
            used_lines[line] = true;
            used_pieces[piece] = true;
            open[line].points.push(Point::new(x, pieces[piece].height));
            open[line].misses = 0;
        }
        // The pieces that no line took start new lines — unless they lie within the tolerance of a
        // line, which means they are the other part of the same line's band of ink.
        for (piece, p) in pieces.iter().enumerate() {
            if used_pieces[piece] {
                continue;
            }
            let near = open.iter().any(|o| {
                let height = o.points.last().map(|q| q.y).unwrap_or(0.0);
                (p.height - height).abs() <= tolerance
            });
            if !near {
                open.push(Open { points: vec![Point::new(x, p.height)], misses: 0 });
            }
        }
        // The lines that found nothing stay open for two strips.
        let mut kept = Vec::new();
        for (index, mut line) in open.drain(..).enumerate() {
            if index < used_lines.len() && !used_lines[index] {
                line.misses += 1;
                if line.misses > 2 {
                    done.push(line.points);
                    continue;
                }
            }
            kept.push(line);
        }
        open = kept;
    }
    done.extend(open.into_iter().map(|l| l.points));
    done
}

/// The mean length of the lines: the distance from each line's first point to its last.
///
/// The *span* of a line, not the length of its path: a line that wanders a little from strip to
/// strip has a long path and a short span, and the span is what says how far the text runs.
fn mean_length(lines: &[Vec<Point>]) -> f64 {
    let spans: Vec<f64> = lines
        .iter()
        .filter_map(|points| match (points.first(), points.last()) {
            (Some(first), Some(last)) => Some(first.distance(*last)),
            _ => None,
        })
        .collect();
    if spans.is_empty() {
        return 0.0;
    }
    spans.iter().sum::<f64>() / spans.len() as f64
}

/// The ink mask turned a quarter turn clockwise.
fn turn(ink: &BitPlane) -> Result<BitPlane, DetectError> {
    let (w, h) = (ink.width(), ink.height());
    let mut out = BitPlane::new(h, w)?;
    for y in 0..h {
        for x in 0..w {
            if ink.get(x, y) == Some(true) {
                // A quarter turn clockwise sends (x, y) to (h − 1 − y, x).
                out.set(h - 1 - y, x, true);
            }
        }
    }
    Ok(out)
}

/// The lines of the turned copy, back in the copy's own columns and rows.
fn unturn(lines: &[Vec<Point>], turned_height: u32) -> Vec<Vec<Point>> {
    lines
        .iter()
        .map(|points| {
            points
                .iter()
                .map(|p| {
                    // The turned copy's (x, y) came from the copy's (y, height − 1 − x).
                    Point::new(p.y, f64::from(turned_height) - 1.0 - p.x)
                })
                .collect()
        })
        .collect()
}
