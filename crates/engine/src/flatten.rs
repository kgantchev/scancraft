//! A29: when to flatten (specification A29). Owner: WP11.
//!
//! A page is flattened only when three things hold: the bent sheet explains its evidence clearly
//! better than a flat one, it moves the page by enough to matter, and the flattened page's text
//! lines are straighter than with the corners alone. [`decide`] weighs them; [`flatten_page`] gathers
//! the evidence (A25, A26), fits the sheet (A27), straightens the page twice (A12, A28) and asks
//! [`decide`].

use crate::error::EngineError;
use crate::jobs::CancellationToken;
use crate::pipeline::PageGeometry;
use scancraft_core::CoreError;
use scancraft_core::camera::{Camera, FocalSource};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::{PlaneF32, Rgb8};
use scancraft_core::map::{PageMap, ScanOrder};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{CornerOrigin, FlattenDecision, FlattenMode, FlattenReason, Page, Sheet, SheetFit, SideShape};
use scancraft_detect::detect::photo_luma;
use scancraft_detect::refine::build_pyramid;
use scancraft_detect::text_lines::{CopyLines, lines_in_copy, text_evidence};
use scancraft_geom::homography::Homography;
use scancraft_geom::sheet::{CornerEvidence, SheetEvidence, SideEvidence, TextDirection, fit_sheets, unroll};
use scancraft_raster::warp::warp_luma;

// The constants below are the specification's starting values, not yet tuned: milestone M2 tunes
// them on the reference set (§10.2). What they do on the synthetic scenes of `scancraft-synth`
// when the page's outline is known (`tests/flatten.rs`, `rates`, seeds 9001 and 9002):
//
// - flat pages left flat: 29 of 30. The one that was flattened has a D95 of 5% and lines that came
//   out 0.74 as crooked.
// - pages whose corners alone leave an error over 1% of the diagonal that were flattened: book
//   pages 11 of 15, curls 12 of 16, sags 7 of 16.
//
// Nearly every page that stays flat is stopped by check 4. On those pages checks 1 to 3 pass
// (D95 from 0.3% to 7%, a lower cost), but the lines of the corner-straightened copy are already
// straight to within about 2 px: when the camera looks square-on, a bend compresses the page
// sideways and leaves its text lines straight, so straightness has nothing to measure. A check that
// asked only that the lines be no more crooked (at most 1.15 times) would pass most of them.
/// Check 1: the fewest text lines that count as evidence.
const EVIDENCE_LINES: u32 = 4;
/// Check 2: D95 as a share of the page's diagonal, from which the bend matters.
const MIN_D95: f64 = 0.002;
/// Check 3: the curved fit's cost, as a share of the plane fit's, at most.
const MAX_COST_SHARE: f64 = 0.97;
/// Check 4: the flattened page's straightness figure, as a share of the corner-straightened
/// page's, with at least [`MANY_LINES`] lines on both.
const STRAIGHTER_SHARE: f64 = 0.9;
/// Check 4: the same, with at least [`EVIDENCE_LINES`] lines on both.
const MUCH_STRAIGHTER_SHARE: f64 = 0.5;
/// Check 4: the lines that the first, milder share asks for.
const MANY_LINES: u32 = 6;
/// The points per side of the grid D95 is measured on.
const GRID: usize = 33;
/// The long side of the analysis copies, in pixels (A26 step 1).
const COPY_LONG_SIDE: u32 = 1600;

/// The sigma of a corner the detector found, in pixels (A27's evidence table).
const SIGMA_DETECTED: f64 = 2.0;
/// The sigma of a corner the user placed.
const SIGMA_PLACED: f64 = 0.5;
/// The sigma of a corner at a spine.
const SIGMA_SPINE: f64 = 4.0;
/// The sigma of a corner outside the photo.
const SIGMA_OUTSIDE: f64 = 8.0;
/// The sigma of a sample A25 traced on a side.
const SIGMA_TRACED: f64 = 1.5;
/// The sigma of the points on a side the user bent.
const SIGMA_BENT: f64 = 1.0;
/// The points taken along a side the user bent.
const BENT_POINTS: usize = 64;

/// What A29 weighs.
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenInput<'a> {
    pub mode: FlattenMode,
    pub plane: Option<&'a Sheet>,
    pub curved: Option<&'a Sheet>,
    pub sides: &'a [SideShape; 4],
    /// A26's lines on the corner-straightened copy, and on the copy flattened by the curved fit.
    pub lines_corners: Option<&'a CopyLines>,
    pub lines_flat: Option<&'a CopyLines>,
    /// The page's diagonal in the photo, in pixels, for D95.
    pub diagonal_px: f64,
    /// The camera the fits were made with, for D95.
    pub camera: Option<&'a Camera>,
    /// Use Full Image or a PDF page: flattening unavailable.
    pub unavailable: bool,
}

/// What A29 decided, and the sheet to flatten with when it decided to.
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenResult {
    /// The curved candidate and the decision behind it; `None` when no sheet could be fitted.
    pub fit: Option<SheetFit>,
    /// What to tell the user (§4.8), when the page didn't get the flattening it asked for.
    pub note: Option<String>,
}

/// The note of a page that asked to be flattened and couldn't be (§4.8).
pub const COULD_NOT_FLATTEN: &str = "Couldn't flatten the curve. The page was straightened by its corners instead.";

/// A29's decision, with the record of how it was reached.
///
/// `Err` is never returned for the inputs A29 can weigh; the type is the contract's.
pub fn decide(input: &FlattenInput<'_>) -> Result<FlattenDecision, CoreError> {
    if let Some(done) = early_checks(input) {
        return Ok(done);
    }
    // Check 4: straighter lines.
    let (corners, flat) = (input.lines_corners, input.lines_flat);
    let (lines_corners, lines_flat) = (line_count(corners), line_count(flat));
    let (figure_corners, figure_flat) = (corners.and_then(straightness), flat.and_then(straightness));
    let opposite = opposite_curved(input.sides);
    let passes = if lines_corners < EVIDENCE_LINES || lines_flat < EVIDENCE_LINES {
        // With fewer than 4 lines on either page, the lines can't tell: two curved opposite sides
        // have to.
        opposite
    } else {
        match (figure_corners, figure_flat) {
            (Some(before), Some(after)) => {
                (lines_corners >= MANY_LINES && lines_flat >= MANY_LINES && after <= STRAIGHTER_SHARE * before)
                    || after <= MUCH_STRAIGHTER_SHARE * before
            }
            _ => false,
        }
    };
    let (d95, plane_cost, curved_cost) = measures(input);
    Ok(FlattenDecision {
        flattened: passes,
        reason: if passes { FlattenReason::Straighter } else { FlattenReason::NotStraighter },
        d95,
        plane_cost,
        curved_cost,
        straightness_corners: figure_corners,
        straightness_flat: figure_flat,
        lines_corners,
        lines_flat,
    })
}

/// Everything A29 can settle before it needs the second set of lines: the mode, the availability
/// and checks 1 to 3. `None` when only check 4 is left, which costs the page's straightening a
/// second time.
pub fn early_checks(input: &FlattenInput<'_>) -> Option<FlattenDecision> {
    let (d95, plane_cost, curved_cost) = measures(input);
    let lines_corners = line_count(input.lines_corners);
    let done = |flattened: bool, reason: FlattenReason| {
        Some(FlattenDecision {
            flattened,
            reason,
            d95,
            plane_cost,
            curved_cost,
            straightness_corners: input.lines_corners.and_then(straightness),
            straightness_flat: input.lines_flat.and_then(straightness),
            lines_corners,
            lines_flat: line_count(input.lines_flat),
        })
    };
    if input.unavailable {
        return done(false, FlattenReason::Unavailable);
    }
    match input.mode {
        FlattenMode::Off => return done(false, FlattenReason::Off),
        FlattenMode::On => {
            let valid = input.curved.is_some_and(|sheet| sheet.cost.is_finite());
            return if valid { done(true, FlattenReason::On) } else { done(false, FlattenReason::OnButNoFit) };
        }
        FlattenMode::Auto => {}
    }
    // Check 1: evidence.
    if lines_corners < EVIDENCE_LINES && !opposite_curved(input.sides) {
        return done(false, FlattenReason::NoEvidence);
    }
    // Check 2: size.
    if d95 < MIN_D95 {
        return done(false, FlattenReason::NoBend);
    }
    // Check 3: gain.
    match (input.plane, input.curved) {
        (Some(plane), Some(curved)) if curved.cost <= MAX_COST_SHARE * plane.cost => None,
        _ => done(false, FlattenReason::NoGain),
    }
}

/// D95 and the two fits' costs, or 0 for what can't be measured.
fn measures(input: &FlattenInput<'_>) -> (f64, f64, f64) {
    let d95 = match (input.plane, input.curved, input.camera) {
        (Some(plane), Some(curved), Some(camera)) => d95_fraction(plane, curved, camera, input.diagonal_px).unwrap_or(0.0),
        _ => 0.0,
    };
    (d95, input.plane.map_or(0.0, |s| s.cost), input.curved.map_or(0.0, |s| s.cost))
}

fn line_count(lines: Option<&CopyLines>) -> u32 {
    lines.map_or(0, |l| u32::try_from(l.lines.len()).unwrap_or(u32::MAX))
}

/// Whether two opposite sides are curved: traced and marked curved by A25, or bent by the user.
pub fn opposite_curved(sides: &[SideShape; 4]) -> bool {
    let curved = |side: Side| matches!(sides.get(side.index()), Some(SideShape::Traced { curved: true, .. } | SideShape::Bent { .. }));
    (curved(Side::Top) && curved(Side::Bottom)) || (curved(Side::Left) && curved(Side::Right))
}

/// A29 check 4's straightness: RMS distance of line points from their own line's mean height.
/// `None` with no lines.
///
/// A line's height is its distance across the text's direction: the row of a line that runs across
/// the copy, the column of one that runs down it.
pub fn straightness(lines: &CopyLines) -> Option<f64> {
    let (mut sum, mut count) = (0.0, 0usize);
    for line in &lines.lines {
        if line.is_empty() {
            continue;
        }
        let height = |p: &Point| match lines.direction {
            TextDirection::Across => p.y,
            TextDirection::Down => p.x,
        };
        let mean = line.iter().map(height).sum::<f64>() / line.len() as f64;
        for p in line {
            let d = height(p) - mean;
            sum += d * d;
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count as f64).sqrt()).filter(|v| v.is_finite())
}

/// The camera a sheet was fitted with: the caller's, with the sheet's own focal length.
fn camera_of(camera: &Camera, sheet: &Sheet) -> Camera {
    Camera { f: sheet.focal, ..*camera }
}

/// D95 (A29 check 2): the 95th percentile of the distances between where the curved and the plane
/// fits place the points of a 33 × 33 grid of the page, as a share of the page's diagonal in the
/// photo. `None` when either sheet can't be unrolled.
pub fn d95_fraction(plane: &Sheet, curved: &Sheet, camera: &Camera, diagonal_px: f64) -> Option<f64> {
    if !(diagonal_px.is_finite() && diagonal_px > 0.0) {
        return None;
    }
    let a = unroll(plane, &camera_of(camera, plane)).ok()?;
    let b = unroll(curved, &camera_of(camera, curved)).ok()?;
    let mut distances = Vec::with_capacity(GRID * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let (u, v) = (i as f64 / (GRID - 1) as f64, j as f64 / (GRID - 1) as f64);
            distances.push(a.map(u, v)?.distance(b.map(u, v)?));
        }
    }
    distances.sort_by(f64::total_cmp);
    let index = ((distances.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(distances.len().saturating_sub(1));
    distances.get(index).map(|d| d / diagonal_px)
}

// ---- the evidence ----

/// The sigma of each corner (A27's table): 2 px detected, 0.5 px placed by the user, 4 px at a
/// spine, 8 px outside the photo.
fn corner_evidence(page: &Page, photo: (u32, u32)) -> [CornerEvidence; 4] {
    let base = if matches!(page.corners_from, CornerOrigin::Manual) { SIGMA_PLACED } else { SIGMA_DETECTED };
    let (w, h) = (f64::from(photo.0), f64::from(photo.1));
    std::array::from_fn(|k| {
        let point = page.corners.0.get(k).copied().unwrap_or(Point::new(0.0, 0.0));
        let outside = point.x < 0.0 || point.y < 0.0 || point.x > w || point.y > h;
        let at_spine = Side::ALL.iter().any(|side| {
            let (a, b) = side.corners();
            (a == k || b == k) && matches!(page.sides.get(side.index()), Some(SideShape::Spine))
        });
        let sigma = if outside {
            SIGMA_OUTSIDE
        } else if at_spine {
            SIGMA_SPINE
        } else {
            base
        };
        CornerEvidence { point, sigma }
    })
}

/// The samples of every side that has some: A25's, and 64 points along a side the user bent in
/// place of A25's. The sides A25 found straight count too (A27).
fn side_evidence(page: &Page) -> Vec<SideEvidence> {
    Side::ALL
        .iter()
        .filter_map(|&side| match page.sides.get(side.index())? {
            SideShape::Traced { samples, .. } if samples.len() >= 2 => {
                Some(SideEvidence { side, points: samples.iter().map(|s| s.point).collect(), sigma: SIGMA_TRACED })
            }
            SideShape::Bent { handles } => {
                let points = crate::curve::bent_side_samples(&page.corners, side, handles, BENT_POINTS);
                (points.len() >= 2).then_some(SideEvidence { side, points, sigma: SIGMA_BENT })
            }
            _ => None,
        })
        .collect()
}

// ---- the analysis copies ----

/// A mapping from the unit square to the photo, scaled into a pyramid level's pixels.
struct LevelMap<M: PageMap> {
    inner: M,
    kx: f64,
    ky: f64,
}

impl<M: PageMap> PageMap for LevelMap<M> {
    fn map(&self, u: f64, v: f64) -> Option<Point> {
        self.inner.map(u, v).map(|p| Point::new(p.x * self.kx, p.y * self.ky))
    }

    fn scan_order(&self) -> ScanOrder {
        self.inner.scan_order()
    }

    fn map_span(&self, order: ScanOrder, fixed: f64, out: &mut [Option<Point>]) {
        self.inner.map_span(order, fixed, out);
        for p in out.iter_mut().flatten() {
            *p = Point::new(p.x * self.kx, p.y * self.ky);
        }
    }
}

/// The level of the pyramid the analysis copies are made from: the coarsest one with at least 1600
/// pixels across the page, or the finest when none has (A26 step 1).
fn analysis_level<'a>(pyramid: &'a LumaPyramid, corners: &Quad) -> Option<&'a PlaneF32> {
    let photo_long = f64::from(pyramid.full().map(|p| p.width().max(p.height()))?);
    let page_long = (0..4).map(|k| corners.0[k].distance(corners.0[(k + 1) % 4])).fold(0.0, f64::max);
    pyramid
        .levels
        .iter()
        .find(|level| page_long * f64::from(level.width().max(level.height())) / photo_long.max(1.0) >= f64::from(COPY_LONG_SIDE))
        .or(pyramid.levels.last())
}

/// The size of an analysis copy of a page of `size_mm`: the long side, and the page's own aspect.
fn copy_size(size_mm: (f64, f64)) -> (u32, u32) {
    let (w, h) = size_mm;
    if w >= h {
        (COPY_LONG_SIDE, ((f64::from(COPY_LONG_SIDE) * h / w).round() as u32).max(1))
    } else {
        (((f64::from(COPY_LONG_SIDE) * w / h).round() as u32).max(1), COPY_LONG_SIDE)
    }
}

/// A26 on the page straightened at the analysis size, by `map` (in the photo's pixels).
///
/// `build` makes the mapping from the factors that turn photo pixels into the level's, and the
/// level's long side.
fn lines_on_copy(
    pyramid: &LumaPyramid,
    corners: &Quad,
    size_mm: (f64, f64),
    build: &dyn Fn(f64, f64, f64) -> Result<Box<dyn PageMap>, EngineError>,
) -> Result<CopyLines, EngineError> {
    let level = analysis_level(pyramid, corners).ok_or_else(|| EngineError::msg("the photo has no luma"))?;
    let full = pyramid.full().unwrap_or(level);
    let (kx, ky) = (f64::from(level.width()) / f64::from(full.width()).max(1.0), f64::from(level.height()) / f64::from(full.height()).max(1.0));
    let (w, h) = copy_size(size_mm);
    let mapping = build(kx, ky, f64::from(level.width().max(level.height())))?;
    let plane = warp_luma(level, mapping.as_ref(), w, h)?;
    let dpi = f64::from(w.max(h)) / (size_mm.0.max(size_mm.1) / 25.4);
    Ok(lines_in_copy(&plane, dpi)?)
}

/// A26 on the copy straightened by the page's corners (A12).
fn lines_by_corners(pyramid: &LumaPyramid, corners: &Quad, size_mm: (f64, f64)) -> Result<CopyLines, EngineError> {
    lines_on_copy(pyramid, corners, size_mm, &|kx, ky, long| {
        let scaled = Quad(corners.0.map(|p| Point::new(p.x * kx, p.y * ky)));
        Ok(Box::new(Homography::from_unit_square(&scaled, long)?))
    })
}

/// A26 on the copy flattened by the sheet (A28).
fn lines_by_sheet(pyramid: &LumaPyramid, corners: &Quad, size_mm: (f64, f64), sheet: &Sheet, camera: &Camera) -> Result<CopyLines, EngineError> {
    let camera = camera_of(camera, sheet);
    lines_on_copy(pyramid, corners, size_mm, &|kx, ky, _| Ok(Box::new(LevelMap { inner: unroll(sheet, &camera)?, kx, ky })))
}

// ---- the whole of A29 for one page ----

/// Decides whether a page is flattened, from the photo (A25's samples are in the page's sides).
///
/// The evidence is the page's text lines (A26) and sides (A25), the fit is A27's, and the check
/// that settles it is A26 again on the page straightened both ways. A page set to Off needs none
/// of this and gets no sheet; neither does a page whose sheet couldn't be fitted.
pub fn flatten_page(photo: &Rgb8, page: &Page, geometry: &PageGeometry, token: &CancellationToken) -> Result<FlattenResult, EngineError> {
    let nothing = FlattenResult { fit: None, note: None };
    if page.flatten == FlattenMode::Off {
        return Ok(nothing);
    }
    let pyramid = build_pyramid(&photo_luma(photo)?)?;
    token.check()?;
    // Step 1 of A29's evidence: the text lines, on the page straightened by its corners.
    let size_mm = geometry.page_size_mm;
    let lines_corners = lines_by_corners(&pyramid, &page.corners, size_mm)?;
    token.check()?;
    let opposite = opposite_curved(&page.sides);
    if page.flatten == FlattenMode::Auto && line_count(Some(&lines_corners)) < EVIDENCE_LINES && !opposite {
        // Check 1 fails, and there is nothing to fit.
        return Ok(nothing);
    }
    // A27: the plane fit and the curved candidate.
    let evidence = {
        let text = text_evidence(&pyramid, &page.corners, size_mm)?;
        let (lines, text_direction) = text.unwrap_or((Vec::new(), TextDirection::Across));
        SheetEvidence { corners: corner_evidence(page, (photo.width(), photo.height())), lines, text_direction, sides: side_evidence(page) }
    };
    let camera = geometry.camera;
    let fits = fit_sheets(&evidence, &camera, camera.source != FocalSource::Known)?;
    token.check()?;
    let diagonal = page.corners.0[0].distance(page.corners.0[2]);
    let mut input = FlattenInput {
        mode: page.flatten,
        plane: fits.plane.as_ref(),
        curved: fits.curved.as_ref(),
        sides: &page.sides,
        lines_corners: Some(&lines_corners),
        lines_flat: None,
        diagonal_px: diagonal,
        camera: Some(&camera),
        unavailable: false,
    };
    let Some(curved) = fits.curved.as_ref() else {
        // Nothing could be fitted: On says so, and Auto leaves the page as it is.
        let note = (page.flatten == FlattenMode::On).then(|| COULD_NOT_FLATTEN.to_owned());
        return Ok(FlattenResult { fit: None, note });
    };
    let decision = match early_checks(&input) {
        Some(done) => done,
        None => {
            // Check 4 needs the page straightened by the sheet, and A26 run on it.
            let flat = lines_by_sheet(&pyramid, &page.corners, size_mm, curved, &camera)?;
            input.lines_flat = Some(&flat);
            decide(&input)?
        }
    };
    let note = (page.flatten == FlattenMode::On && !decision.flattened).then(|| COULD_NOT_FLATTEN.to_owned());
    Ok(FlattenResult { fit: Some(SheetFit { sheet: curved.clone(), decision }), note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::recipe::SideSample;

    fn camera() -> Camera {
        Camera { width: 1000.0, height: 750.0, f: 1.0, source: FocalSource::Known }
    }

    /// A sheet in front of the camera: bend across, `bend` the profile's height at its middle
    /// knots as a share of the chord, `cost` the fit's cost.
    fn sheet(bend: f64, cost: f64) -> Sheet {
        let mut profile = [0.0; 10];
        for (k, value) in profile.iter_mut().enumerate() {
            // A smooth arch over the ten free knots.
            let t = (k as f64 + 1.0) / 11.0;
            *value = bend * 4.0 * t * (1.0 - t);
        }
        Sheet {
            mode: scancraft_core::recipe::BendMode::Across,
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation: [-0.4, -0.5, 1.5],
            chord: 0.8,
            profile,
            focal: 1.0,
            focal_source: FocalSource::Known,
            cost,
        }
    }

    fn lines(count: usize, wobble: f64, direction: TextDirection) -> CopyLines {
        let lines = (0..count)
            .map(|i| {
                (0..32)
                    .map(|k| {
                        let along = 50.0 + k as f64 * 40.0;
                        let across = 100.0 + i as f64 * 30.0 + if k % 2 == 0 { wobble } else { -wobble };
                        match direction {
                            TextDirection::Across => Point::new(along, across),
                            TextDirection::Down => Point::new(across, along),
                        }
                    })
                    .collect()
            })
            .collect();
        CopyLines { lines, direction, median_piece_height: 8.0 }
    }

    const STRAIGHT: [SideShape; 4] = [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight];

    fn curved_sides() -> [SideShape; 4] {
        let traced = |curved| SideShape::Traced { samples: vec![SideSample { point: Point::new(0.0, 0.0), weight: 1.0 }; 3], bow: 0.02, curved };
        [traced(true), SideShape::Straight, traced(true), SideShape::Straight]
    }

    fn input<'a>(mode: FlattenMode, plane: &'a Sheet, curved: &'a Sheet, sides: &'a [SideShape; 4], camera: &'a Camera) -> FlattenInput<'a> {
        FlattenInput {
            mode,
            plane: Some(plane),
            curved: Some(curved),
            sides,
            lines_corners: None,
            lines_flat: None,
            diagonal_px: 300.0,
            camera: Some(camera),
            unavailable: false,
        }
    }

    #[test]
    fn straightness_is_the_rms_of_the_points_from_their_own_line() {
        let one = CopyLines {
            lines: vec![vec![Point::new(0.0, 10.0), Point::new(1.0, 12.0), Point::new(2.0, 10.0), Point::new(3.0, 12.0)]],
            direction: TextDirection::Across,
            median_piece_height: 5.0,
        };
        assert!((straightness(&one).unwrap() - 1.0).abs() < 1e-12);
        // Lines that run down the copy are measured across it.
        let down = CopyLines {
            lines: vec![vec![Point::new(10.0, 0.0), Point::new(12.0, 1.0), Point::new(10.0, 2.0), Point::new(12.0, 3.0)]],
            direction: TextDirection::Down,
            median_piece_height: 5.0,
        };
        assert!((straightness(&down).unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(straightness(&CopyLines { lines: vec![], direction: TextDirection::Across, median_piece_height: 0.0 }), None);
    }

    #[test]
    fn off_and_on_do_what_they_say() {
        let (plane, curved, camera) = (sheet(0.0, 10.0), sheet(0.05, 9.0), camera());
        let off = decide(&input(FlattenMode::Off, &plane, &curved, &STRAIGHT, &camera)).unwrap();
        assert!(!off.flattened && off.reason == FlattenReason::Off);
        // On flattens with a valid candidate, whatever the checks say: no lines, no curved sides.
        let on = decide(&input(FlattenMode::On, &plane, &curved, &STRAIGHT, &camera)).unwrap();
        assert!(on.flattened && on.reason == FlattenReason::On);
    }

    #[test]
    fn an_invalid_fit_under_on_falls_back_to_the_corners_with_the_note() {
        let (plane, camera) = (sheet(0.0, 10.0), camera());
        let mut bad = sheet(0.05, f64::NAN);
        bad.cost = f64::NAN;
        let none = FlattenInput { curved: None, ..input(FlattenMode::On, &plane, &bad, &STRAIGHT, &camera) };
        for input in [none, input(FlattenMode::On, &plane, &bad, &STRAIGHT, &camera)] {
            let decision = decide(&input).unwrap();
            assert!(!decision.flattened && decision.reason == FlattenReason::OnButNoFit, "{decision:?}");
        }
    }

    #[test]
    fn flattening_is_unavailable_for_a_full_image_or_a_pdf_page() {
        let (plane, curved, camera) = (sheet(0.0, 10.0), sheet(0.05, 9.0), camera());
        for mode in [FlattenMode::Auto, FlattenMode::On, FlattenMode::Off] {
            let decision = decide(&FlattenInput { unavailable: true, ..input(mode, &plane, &curved, &STRAIGHT, &camera) }).unwrap();
            assert!(!decision.flattened && decision.reason == FlattenReason::Unavailable);
        }
    }

    #[test]
    fn auto_needs_evidence_a_real_bend_a_gain_and_straighter_lines_in_that_order() {
        let camera = camera();
        let (plane, bent) = (sheet(0.0, 10.0), sheet(0.08, 5.0));
        let many = lines(8, 1.0, TextDirection::Across);
        let straight_flat = lines(8, 0.2, TextDirection::Across);

        // Check 1: no lines and no curved sides.
        let none = decide(&input(FlattenMode::Auto, &plane, &bent, &STRAIGHT, &camera)).unwrap();
        assert_eq!(none.reason, FlattenReason::NoEvidence);
        // Two curved opposite sides count as evidence, but a curved candidate that equals the plane
        // moves nothing: check 2.
        let same = decide(&input(FlattenMode::Auto, &plane, &plane, &curved_sides(), &camera)).unwrap();
        assert_eq!(same.reason, FlattenReason::NoBend);
        assert!(same.d95 < MIN_D95);
        // Check 3: it moves the page, but the cost is not lower by enough.
        let dear = sheet(0.08, 9.9);
        let gain = decide(&FlattenInput { lines_corners: Some(&many), ..input(FlattenMode::Auto, &plane, &dear, &STRAIGHT, &camera) }).unwrap();
        assert_eq!(gain.reason, FlattenReason::NoGain);
        assert!(gain.d95 >= MIN_D95, "{}", gain.d95);
        // Check 4: lines no straighter after flattening.
        let worse = decide(&FlattenInput {
            lines_corners: Some(&many),
            lines_flat: Some(&many),
            ..input(FlattenMode::Auto, &plane, &bent, &STRAIGHT, &camera)
        })
        .unwrap();
        assert_eq!(worse.reason, FlattenReason::NotStraighter);
        assert_eq!((worse.lines_corners, worse.lines_flat), (8, 8));
        // All four pass.
        let yes = decide(&FlattenInput {
            lines_corners: Some(&many),
            lines_flat: Some(&straight_flat),
            ..input(FlattenMode::Auto, &plane, &bent, &STRAIGHT, &camera)
        })
        .unwrap();
        assert!(yes.flattened && yes.reason == FlattenReason::Straighter, "{yes:?}");
        assert!(yes.straightness_flat.unwrap() < yes.straightness_corners.unwrap());
        assert!(yes.plane_cost > yes.curved_cost);
    }

    #[test]
    fn check_four_asks_for_less_when_there_are_more_lines() {
        let camera = camera();
        let (plane, bent) = (sheet(0.0, 10.0), sheet(0.08, 5.0));
        let run = |n: usize, before: f64, after: f64, sides: &[SideShape; 4]| {
            let (c, f) = (lines(n, before, TextDirection::Across), lines(n, after, TextDirection::Across));
            decide(&FlattenInput { lines_corners: Some(&c), lines_flat: Some(&f), ..input(FlattenMode::Auto, &plane, &bent, sides, &camera) })
                .unwrap()
        };
        // Six lines: 0.85 of the other's figure is enough. Five lines: it isn't, and 0.45 is.
        assert!(run(6, 1.0, 0.85, &STRAIGHT).flattened);
        assert!(!run(6, 1.0, 0.95, &STRAIGHT).flattened);
        assert!(!run(5, 1.0, 0.85, &STRAIGHT).flattened);
        assert!(run(5, 1.0, 0.45, &STRAIGHT).flattened);
    }

    #[test]
    fn with_fewer_than_four_lines_only_two_curved_opposite_sides_pass_the_last_check() {
        let camera = camera();
        let (plane, bent) = (sheet(0.0, 10.0), sheet(0.08, 5.0));
        let (c, f) = (lines(3, 1.0, TextDirection::Across), lines(3, 0.1, TextDirection::Across));
        let with = |sides: &[SideShape; 4]| {
            decide(&FlattenInput { lines_corners: Some(&c), lines_flat: Some(&f), ..input(FlattenMode::Auto, &plane, &bent, sides, &camera) })
                .unwrap()
        };
        assert!(with(&curved_sides()).flattened);
        // Without curved sides there is no evidence at all.
        assert_eq!(with(&STRAIGHT).reason, FlattenReason::NoEvidence);
    }

    #[test]
    fn opposite_curved_sides_are_traced_or_bent_ones() {
        let bent = SideShape::Bent { handles: [Point::new(0.0, 0.0); 3] };
        assert!(opposite_curved(&[bent.clone(), SideShape::Straight, bent.clone(), SideShape::Straight]));
        assert!(!opposite_curved(&[bent.clone(), bent.clone(), SideShape::Straight, SideShape::Straight]));
        assert!(!opposite_curved(&STRAIGHT));
    }
}
