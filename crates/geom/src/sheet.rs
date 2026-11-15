//! A27 (sheet fit) and A28 (unrolling). Owner: WP1. The cubic spline and the Levenberg–Marquardt
//! solver are WP1's own modules; only what other packages call is fixed here.

use crate::GeomError;
use crate::solver::{ExtraTerm, LmRow, LmSystem, cauchy_loss, cauchy_weight};
use crate::spline::Profile;
use scancraft_core::camera::{Camera, FocalSource};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::map::PageMap;
use scancraft_core::recipe::{BendMode, Sheet};
use scancraft_core::{Step, not_implemented};

/// The rotation vector's first column in the shared unknowns.
const ROTATION: usize = 0;
/// The translation's first column.
const TRANSLATION: usize = 3;
/// The chord's column.
const CHORD: usize = 6;
/// The profile's first column.
const PROFILE: usize = 7;
/// The focal length's column, present only when the focal length is fitted.
const FOCAL: usize = 17;
/// The number of shared unknowns that are not a text line's coordinate.
const GLOBALS: usize = 17;
/// The number of shared unknowns with the focal length fitted.
const GLOBALS_WITH_FOCAL: usize = 18;
/// The most text lines the fit takes (A26 keeps at most 120).
const MAX_LINES: usize = 120;
/// The most observations the fit takes.
const MAX_OBSERVATIONS: usize = 20_000;
/// The weight of the bending penalty (A27's cost).
const BENDING_WEIGHT: f64 = 0.5;
/// The spread of the focal length prior, in ln f.
const FOCAL_SPREAD: f64 = 0.3;
/// The damping the solver starts with, and the factor it grows by.
const MU_START: f64 = 1e-3;
const NU_START: f64 = 2.0;
/// The damping above which the solver gives up.
const MU_LIMIT: f64 = 1e12;
/// The most iterations the solver takes.
const MAX_ITERATIONS: usize = 60;
/// A step that lowers the cost by less than this share of it has converged.
const CONVERGED: f64 = 1e-7;
/// The bounds a coordinate stays within.
const COORD_MIN: f64 = -0.05;
const COORD_MAX: f64 = 1.05;
/// The steepest profile the fit accepts (72°).
const MAX_SLOPE: f64 = 3.0;
/// The focal lengths the fit accepts.
const FOCAL_MIN: f64 = 0.35;
const FOCAL_MAX: f64 = 4.0;

/// Which way the page's text runs (A26 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDirection {
    Across,
    Down,
}

/// One text line from A26, in photo pixels, with each point's σ.
#[derive(Debug, Clone, PartialEq)]
pub struct LineEvidence {
    pub points: Vec<Point>,
    pub sigma: Vec<f64>,
}

/// Samples along one side: from A25, or 64 points on the user's bent curve.
#[derive(Debug, Clone, PartialEq)]
pub struct SideEvidence {
    pub side: Side,
    pub points: Vec<Point>,
    pub sigma: f64,
}

/// How firmly each corner is held (A27's evidence table).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerEvidence {
    pub point: Point,
    pub sigma: f64,
}

/// Everything A27 fits to.
///
/// The four corners are in the canonical order TL, TR, BR, BL, the same as
/// [`scancraft_core::geom::Quad`].
#[derive(Debug, Clone, PartialEq)]
pub struct SheetEvidence {
    pub corners: [CornerEvidence; 4],
    pub lines: Vec<LineEvidence>,
    pub text_direction: TextDirection,
    pub sides: Vec<SideEvidence>,
}

/// A27's output: the plane fit and the curved candidate (the better of the two bend directions).
#[derive(Debug, Clone, PartialEq)]
pub struct SheetFits {
    pub plane: Option<Sheet>,
    pub curved: Option<Sheet>,
}

/// A27. `camera` comes from A11; `fit_focal` is true when EXIF gave no focal length.
pub fn fit_sheets(evidence: &SheetEvidence, camera: &Camera, fit_focal: bool) -> Result<SheetFits, GeomError> {
    let corners = evidence_quad(evidence);
    if !corners.is_finite() {
        return Err(GeomError::Degenerate("the corners are not finite"));
    }
    let profile = Profile::new();
    let across = fit_one(evidence, camera, &profile, BendMode::Across, fit_focal, false);
    let down = fit_one(evidence, camera, &profile, BendMode::Down, fit_focal, false);
    // The curved candidate is the bent fit with the lower cost. The plane fit follows it in the
    // same bend direction, so that A29 compares two placements of the same page.
    let curved = match (across, down) {
        (Some(a), Some(d)) => Some(if a.cost <= d.cost { a } else { d }),
        (Some(a), None) => Some(a),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    };
    let plane = match &curved {
        Some(sheet) => fit_one(evidence, camera, &profile, sheet.mode, fit_focal, true),
        None => None,
    };
    Ok(SheetFits { plane, curved })
}

/// The evidence's corners as a quadrilateral.
fn evidence_quad(evidence: &SheetEvidence) -> Quad {
    Quad([evidence.corners[0].point, evidence.corners[1].point, evidence.corners[2].point, evidence.corners[3].point])
}

/// Where an observation's sheet coordinates come from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    /// A fixed coordinate.
    Fixed(f64),
    /// The point's own coordinate.
    Own,
    /// A text line's shared coordinate.
    Line(usize),
}

/// One measured point: where it lies on the sheet and where the photo saw it.
#[derive(Debug, Clone, PartialEq)]
struct Observation {
    t: Source,
    b: Source,
    /// The index of the point's own coordinate, when it has one.
    own: Option<usize>,
    /// The index of the line's shared coordinate, when the point is on a text line.
    line: Option<usize>,
    observed: Point,
    sigma: f64,
}

/// The observations of one bend direction, with their starting values.
#[derive(Debug, Clone, PartialEq)]
struct Prepared {
    observations: Vec<Observation>,
    /// The starting value of each line's shared coordinate.
    line_starts: Vec<f64>,
    /// The starting value of each point's own coordinate.
    point_starts: Vec<f64>,
    /// True when a text line's shared coordinate is the profile's `t`.
    shared_is_t: bool,
}

/// The state of the fit: the model and the per-line and per-point coordinates.
#[derive(Debug, Clone, PartialEq)]
struct State {
    mode: BendMode,
    rotation: [[f64; 3]; 3],
    translation: [f64; 3],
    chord: f64,
    profile: [f64; 10],
    focal: f64,
    /// The shared coordinate of each text line.
    lines: Vec<f64>,
    /// The own coordinate of each point.
    points: Vec<f64>,
}

/// True when the text lines' shared coordinate is the profile's `t`.
fn line_is_t(mode: BendMode, direction: TextDirection) -> bool {
    (mode == BendMode::Across) == (direction == TextDirection::Down)
}

/// The fixed coordinate of a side, and whether it is `t` or `b`.
fn side_placement(side: Side, mode: BendMode) -> (bool, f64) {
    match (mode, side) {
        (BendMode::Across, Side::Top) => (false, 0.0),
        (BendMode::Across, Side::Bottom) => (false, 1.0),
        (BendMode::Across, Side::Left) => (true, 0.0),
        (BendMode::Across, Side::Right) => (true, 1.0),
        (BendMode::Down, Side::Top) => (true, 0.0),
        (BendMode::Down, Side::Bottom) => (true, 1.0),
        (BendMode::Down, Side::Left) => (false, 0.0),
        (BendMode::Down, Side::Right) => (false, 1.0),
    }
}

/// The sheet coordinates of the four corners, in the order TL, TR, BR, BL.
fn corner_sheet_coordinates(mode: BendMode) -> [(f64, f64); 4] {
    match mode {
        BendMode::Across => [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        BendMode::Down => [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)],
    }
}

/// The sheet's own frame: the model of A27.
fn sheet_point(mode: BendMode, t: f64, b: f64, chord: f64, eta: f64) -> [f64; 3] {
    match mode {
        BendMode::Across => [chord * t, b, -chord * eta],
        BendMode::Down => [b, chord * t, -chord * eta],
    }
}

/// The derivative of the sheet point by the chord.
fn sheet_d_chord(mode: BendMode, t: f64, eta: f64) -> [f64; 3] {
    match mode {
        BendMode::Across => [t, 0.0, -eta],
        BendMode::Down => [0.0, t, -eta],
    }
}

/// The derivative of the sheet point by `t`.
fn sheet_d_t(mode: BendMode, chord: f64, eta_d: f64) -> [f64; 3] {
    match mode {
        BendMode::Across => [chord, 0.0, -chord * eta_d],
        BendMode::Down => [0.0, chord, -chord * eta_d],
    }
}

/// The derivative of the sheet point by `b`.
fn sheet_d_b(mode: BendMode) -> [f64; 3] {
    match mode {
        BendMode::Across => [0.0, 1.0, 0.0],
        BendMode::Down => [1.0, 0.0, 0.0],
    }
}

/// The evidence, prepared for one bend direction.
fn prepare(evidence: &SheetEvidence, camera: &Camera, mode: BendMode) -> Result<Prepared, GeomError> {
    let corners = evidence_quad(evidence);
    let homography = crate::homography::Homography::from_unit_square(&corners, camera.long_side())?;
    let mut observations: Vec<Observation> = Vec::new();
    let mut line_values: Vec<Vec<f64>> = Vec::new();
    let mut point_values: Vec<f64> = Vec::new();

    // The corners: both coordinates are fixed.
    let corner_coordinates = corner_sheet_coordinates(mode);
    for (index, corner) in evidence.corners.iter().enumerate() {
        let (t, b) = corner_coordinates[index];
        observations.push(Observation {
            t: Source::Fixed(t),
            b: Source::Fixed(b),
            own: None,
            line: None,
            observed: corner.point,
            sigma: corner.sigma.max(1e-3),
        });
    }

    // The text lines: the line's shared coordinate and the point's own coordinate.
    let shared_is_t = line_is_t(mode, evidence.text_direction);
    let mut line_index = 0usize;
    for line in evidence.lines.iter().take(MAX_LINES) {
        if line.points.is_empty() {
            continue;
        }
        let mut values = Vec::with_capacity(line.points.len());
        for (index, point) in line.points.iter().enumerate() {
            let (t, b) = inverse_sheet(&homography, *point, mode);
            let own = point_values.len();
            point_values.push(if shared_is_t { b } else { t });
            values.push(if shared_is_t { t } else { b });
            observations.push(Observation {
                t: if shared_is_t { Source::Line(line_index) } else { Source::Own },
                b: if shared_is_t { Source::Own } else { Source::Line(line_index) },
                own: Some(own),
                line: Some(line_index),
                observed: *point,
                sigma: line.sigma.get(index).copied().unwrap_or(2.0).max(1e-3),
            });
        }
        line_values.push(values);
        line_index += 1;
    }

    // The sides: one coordinate is fixed, the other is the point's own.
    for side in &evidence.sides {
        let (fixed_is_t, fixed) = side_placement(side.side, mode);
        for point in &side.points {
            let (t, b) = inverse_sheet(&homography, *point, mode);
            let own = point_values.len();
            point_values.push(if fixed_is_t { b } else { t });
            observations.push(Observation {
                t: if fixed_is_t { Source::Fixed(fixed) } else { Source::Own },
                b: if fixed_is_t { Source::Own } else { Source::Fixed(fixed) },
                own: Some(own),
                line: None,
                observed: *point,
                sigma: side.sigma.max(1e-3),
            });
        }
    }
    observations.truncate(MAX_OBSERVATIONS);

    // Each line's shared coordinate starts at the median of its points.
    let line_starts = line_values.iter().map(|values| median(values)).collect();
    Ok(Prepared { observations, line_starts, point_starts: point_values, shared_is_t })
}

/// The sheet coordinates a photo position maps back to, from the homography's inverse.
fn inverse_sheet(homography: &crate::homography::Homography, point: Point, mode: BendMode) -> (f64, f64) {
    let (u, v) = homography.invert(point.x, point.y).unwrap_or((0.5, 0.5));
    let (t, b) = match mode {
        BendMode::Across => (u, v),
        BendMode::Down => (v, u),
    };
    (t.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

/// The median of a set of values.
fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.5;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        let a = sorted.get(middle - 1).copied().unwrap_or(0.0);
        let b = sorted.get(middle).copied().unwrap_or(0.0);
        0.5 * (a + b)
    } else {
        sorted.get(middle).copied().unwrap_or(0.0)
    }
}

/// The starting pose, from the corners' homography (A27's start).
fn start_pose(corners: &Quad, camera: &Camera, mode: BendMode) -> Option<([[f64; 3]; 3], [f64; 3], f64)> {
    let homography = crate::homography::Homography::from_unit_square(corners, camera.long_side()).ok()?;
    let [a, b, c, d, e, f, g, h] = homography.m;
    let (w, hh) = (camera.width, camera.height);
    let fp = camera.focal_px();
    if !fp.is_finite() || fp <= 0.0 {
        return None;
    }
    // K⁻¹ for the camera of §7.8.
    let unproject = |x: f64, y: f64, z: f64| -> [f64; 3] { [(x - w / 2.0) / fp, (y - hh / 2.0) / fp, z] };
    let mut m1 = unproject(a, d, g);
    let mut m2 = unproject(b, e, h);
    let mut m3 = unproject(c, f, 1.0);
    if m3[2] < 0.0 {
        m1 = [-m1[0], -m1[1], -m1[2]];
        m2 = [-m2[0], -m2[1], -m2[2]];
        m3 = [-m3[0], -m3[1], -m3[2]];
    }
    let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let (l1, l2) = (length(m1), length(m2));
    if !l1.is_finite() || !l2.is_finite() || l1 <= 0.0 || l2 <= 0.0 {
        return None;
    }
    // The rulings have length 1, so which of the two directions carries the length depends on the
    // mode: in bend-across the rulings run down the page, in bend-down they run across it.
    let (lambda, chord) = match mode {
        BendMode::Across => (l2, l1 / l2),
        BendMode::Down => (l1, l2 / l1),
    };
    if !lambda.is_finite() || lambda <= 0.0 || !chord.is_finite() || chord <= 0.0 {
        return None;
    }
    let u1 = [m1[0] / l1, m1[1] / l1, m1[2] / l1];
    let u2 = [m2[0] / l2, m2[1] / l2, m2[2] / l2];
    let u3 = [u1[1] * u2[2] - u1[2] * u2[1], u1[2] * u2[0] - u1[0] * u2[2], u1[0] * u2[1] - u1[1] * u2[0]];
    let rotation = crate::solver::nearest_rotation(&[u1, u2, u3]);
    let translation = [m3[0] / lambda, m3[1] / lambda, m3[2] / lambda];
    Some((rotation, translation, chord))
}

/// Runs one fit: the bend direction's curved fit, or the plane fit.
fn fit_one(evidence: &SheetEvidence, camera: &Camera, profile: &Profile, mode: BendMode, fit_focal: bool, plane: bool) -> Option<Sheet> {
    let corners = evidence_quad(evidence);
    let prepared = prepare(evidence, camera, mode).ok()?;
    let (rotation, translation, chord) = start_pose(&corners, camera, mode)?;
    let mut state = State {
        mode,
        rotation,
        translation,
        chord,
        profile: [0.0; 10],
        focal: camera.f,
        lines: prepared.line_starts.clone(),
        points: prepared.point_starts.clone(),
    };
    let cost = run_fit(&mut state, &prepared, camera, profile, fit_focal, plane)?;
    let sheet = Sheet {
        mode: state.mode,
        rotation: [
            state.rotation[0][0],
            state.rotation[0][1],
            state.rotation[0][2],
            state.rotation[1][0],
            state.rotation[1][1],
            state.rotation[1][2],
            state.rotation[2][0],
            state.rotation[2][1],
            state.rotation[2][2],
        ],
        translation: state.translation,
        chord: state.chord,
        profile: state.profile,
        focal: state.focal,
        focal_source: if fit_focal { FocalSource::Fitted } else { camera.source },
        cost,
    };
    if valid(&sheet, profile) { Some(sheet) } else { None }
}

/// A27's validity rules.
fn valid(sheet: &Sheet, profile: &Profile) -> bool {
    if !sheet.chord.is_finite() || sheet.chord <= 0.0 {
        return false;
    }
    if !sheet.focal.is_finite() || sheet.focal < FOCAL_MIN || sheet.focal > FOCAL_MAX {
        return false;
    }
    if !sheet.cost.is_finite() {
        return false;
    }
    // The profile's slope is piecewise linear, so its steepest point is at a knot.
    for knot in profile.knots() {
        if profile.derivative(&sheet.profile, *knot).abs() > MAX_SLOPE {
            return false;
        }
    }
    true
}

/// The Levenberg–Marquardt loop (A27's solver).
fn run_fit(state: &mut State, prepared: &Prepared, camera: &Camera, profile: &Profile, fit_focal: bool, plane: bool) -> Option<f64> {
    let mut mu = MU_START;
    let mut nu = NU_START;
    let mut cost = f64::INFINITY;
    for _ in 0..MAX_ITERATIONS {
        let (rows, residual_cost) = build_rows(state, prepared, camera, profile, fit_focal, plane)?;
        cost = residual_cost + bending_cost(state, profile, plane) + focal_cost(state, camera, fit_focal);
        let shared = shared_count(fit_focal, prepared.line_starts.len());
        let extra = extra_terms(state, profile, camera, fit_focal, plane);
        let system = LmSystem { shared, rows, extra, damping: mu };
        let Some(delta) = system.step() else {
            mu *= nu;
            nu *= 2.0;
            if mu > MU_LIMIT {
                break;
            }
            continue;
        };
        let own = system.own_steps(&delta);
        let predicted =
            system.predicted_decrease(&delta, &own) + model_bending(state, profile, plane, &delta) + model_focal(state, camera, fit_focal, &delta);
        let mut next = state.clone();
        apply_step(&mut next, &delta, &own, fit_focal, plane);
        let Some(next_cost) = evaluate(&next, prepared, camera, profile, fit_focal, plane) else {
            mu *= nu;
            nu *= 2.0;
            if mu > MU_LIMIT {
                break;
            }
            continue;
        };
        let decrease = cost - next_cost;
        let gain = if predicted > 1e-300 { decrease / predicted } else { 0.0 };
        if gain > 0.0 {
            *state = next;
            cost = next_cost;
            if decrease.abs() < CONVERGED * cost.abs().max(1e-12) {
                break;
            }
            mu *= (1.0 - (2.0 * gain - 1.0).powi(3)).max(1.0 / 3.0);
            nu = NU_START;
        } else {
            mu *= nu;
            nu *= 2.0;
            if mu > MU_LIMIT {
                break;
            }
        }
    }
    if cost.is_finite() { Some(cost) } else { None }
}

/// The number of shared unknowns.
fn shared_count(fit_focal: bool, lines: usize) -> usize {
    (if fit_focal { GLOBALS_WITH_FOCAL } else { GLOBALS }) + lines
}

/// The residual rows of the current state, and the cost of the residuals.
fn build_rows(state: &State, prepared: &Prepared, camera: &Camera, profile: &Profile, fit_focal: bool, plane: bool) -> Option<(Vec<LmRow>, f64)> {
    let mut rows = Vec::with_capacity(prepared.observations.len());
    let mut cost = 0.0;
    let first_line = shared_count(fit_focal, 0);
    for observation in &prepared.observations {
        let (t, b) = resolve(observation, state);
        let eta = if plane { 0.0 } else { profile.value(&state.profile, t) };
        let eta_d = if plane { 0.0 } else { profile.derivative(&state.profile, t) };
        let p = sheet_point(state.mode, t, b, state.chord, eta);
        let q = camera_space(&state.rotation, &state.translation, &p);
        let projected = project(camera, q, state.focal)?;
        let sigma = observation.sigma;
        let e = [(projected.point.x - observation.observed.x) / sigma, (projected.point.y - observation.observed.y) / sigma];
        let e2 = e[0] * e[0] + e[1] * e[1];
        cost += 0.5 * cauchy_loss(e2);
        let weight = cauchy_weight(e2);

        // The derivatives of the camera-space point.
        let d_chord = rotate(&state.rotation, &sheet_d_chord(state.mode, t, eta));
        let d_t = rotate(&state.rotation, &sheet_d_t(state.mode, state.chord, eta_d));
        let d_b = rotate(&state.rotation, &sheet_d_b(state.mode));
        let v = [q[0] - state.translation[0], q[1] - state.translation[1], q[2] - state.translation[2]];

        let mut columns: Vec<(usize, f64, f64)> = Vec::with_capacity(GLOBALS_WITH_FOCAL + 1);
        // The rotation vector: δQ = δω × (Q − T).
        for k in 0..3 {
            let axis = [if k == 0 { 1.0 } else { 0.0 }, if k == 1 { 1.0 } else { 0.0 }, if k == 2 { 1.0 } else { 0.0 }];
            let crossed = [axis[1] * v[2] - axis[2] * v[1], axis[2] * v[0] - axis[0] * v[2], axis[0] * v[1] - axis[1] * v[0]];
            push_column(&mut columns, ROTATION + k, scale(apply(&projected.jac, &crossed), sigma));
        }
        // The translation.
        for k in 0..3 {
            let unit = [if k == 0 { 1.0 } else { 0.0 }, if k == 1 { 1.0 } else { 0.0 }, if k == 2 { 1.0 } else { 0.0 }];
            push_column(&mut columns, TRANSLATION + k, scale(apply(&projected.jac, &unit), sigma));
        }
        // The chord.
        push_column(&mut columns, CHORD, scale(apply(&projected.jac, &d_chord), sigma));
        // The profile's values.
        if !plane {
            let basis = profile.basis(t);
            for (j, value) in basis.iter().enumerate() {
                let d = [0.0, 0.0, -state.chord * value];
                push_column(&mut columns, PROFILE + j, scale(apply(&projected.jac, &rotate(&state.rotation, &d)), sigma));
            }
        }
        // The focal length.
        if fit_focal {
            push_column(&mut columns, FOCAL, [projected.df[0] / sigma, projected.df[1] / sigma]);
        }
        // The line's shared coordinate.
        if let Some(line) = observation.line {
            let derivative = if prepared.shared_is_t { d_t } else { d_b };
            push_column(&mut columns, first_line + line, scale(apply(&projected.jac, &derivative), sigma));
        }
        // The point's own coordinate.
        let own = observation.own.map(|_| {
            let derivative = if observation.t == Source::Own { d_t } else { d_b };
            scale(apply(&projected.jac, &derivative), sigma)
        });
        rows.push(LmRow { e, w: weight, columns, a: own.unwrap_or([0.0, 0.0]) });
    }
    Some((rows, cost))
}

/// The sheet coordinates of an observation.
fn resolve(observation: &Observation, state: &State) -> (f64, f64) {
    let t = match observation.t {
        Source::Fixed(value) => value,
        Source::Own => state.points.get(observation.own.unwrap_or(0)).copied().unwrap_or(0.5),
        Source::Line(index) => state.lines.get(index).copied().unwrap_or(0.5),
    };
    let b = match observation.b {
        Source::Fixed(value) => value,
        Source::Own => state.points.get(observation.own.unwrap_or(0)).copied().unwrap_or(0.5),
        Source::Line(index) => state.lines.get(index).copied().unwrap_or(0.5),
    };
    (t, b)
}

/// The camera-space point: Q = R·P + T.
fn camera_space(rotation: &[[f64; 3]; 3], translation: &[f64; 3], p: &[f64; 3]) -> [f64; 3] {
    let r = crate::solver::mat_vec(rotation, *p);
    [r[0] + translation[0], r[1] + translation[1], r[2] + translation[2]]
}

/// A vector rotated by the sheet's rotation.
fn rotate(rotation: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    crate::solver::mat_vec(rotation, *v)
}

/// The photo position of a camera-space point, and its derivatives.
struct Projected {
    point: Point,
    /// The derivative of the photo position by the camera-space coordinates: `jac[axis][coord]`.
    jac: [[f64; 3]; 2],
    /// The derivative of the photo position by the focal length.
    df: [f64; 2],
}

/// Projects a camera-space point; `None` when it is behind the camera.
fn project(camera: &Camera, q: [f64; 3], focal: f64) -> Option<Projected> {
    let z = q[2];
    if !z.is_finite() || z <= 0.0 {
        return None;
    }
    let long = camera.long_side();
    let fp = focal * long;
    if !fp.is_finite() || fp <= 0.0 {
        return None;
    }
    let point = Point::new(camera.width / 2.0 + fp * q[0] / z, camera.height / 2.0 + fp * q[1] / z);
    let jac = [[fp / z, 0.0, -fp * q[0] / (z * z)], [0.0, fp / z, -fp * q[1] / (z * z)]];
    // jac[0] is the photo x row and jac[1] the photo y row, each over the camera's X, Y and Z.
    let df = [long * q[0] / z, long * q[1] / z];
    Some(Projected { point, jac, df })
}

/// The derivative of the photo position by a camera-space direction.
fn apply(jac: &[[f64; 3]; 2], d: &[f64; 3]) -> [f64; 2] {
    [jac[0][0] * d[0] + jac[0][1] * d[1] + jac[0][2] * d[2], jac[1][0] * d[0] + jac[1][1] * d[1] + jac[1][2] * d[2]]
}

/// A vector divided by a scalar.
fn scale(v: [f64; 2], s: f64) -> [f64; 2] {
    [v[0] / s, v[1] / s]
}

/// Adds one column to a row's Jacobian: the unknown's index and the residual's derivatives.
fn push_column(columns: &mut Vec<(usize, f64, f64)>, index: usize, value: [f64; 2]) {
    columns.push((index, value[0], value[1]));
}

/// The cost of the bending penalty.
fn bending_cost(state: &State, profile: &Profile, plane: bool) -> f64 {
    if plane {
        return 0.0;
    }
    0.5 * BENDING_WEIGHT * profile.bending_integral(&state.profile)
}

/// The cost of the focal length prior.
fn focal_cost(state: &State, camera: &Camera, fit_focal: bool) -> f64 {
    if !fit_focal || camera.f.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return 0.0;
    }
    let difference = (state.focal.ln() - camera.f.ln()) / FOCAL_SPREAD;
    0.5 * difference * difference
}

/// The extra quadratic terms of the normal equations: the bending penalty and the focal prior.
fn extra_terms(state: &State, profile: &Profile, camera: &Camera, fit_focal: bool, plane: bool) -> Vec<ExtraTerm> {
    let mut terms = Vec::new();
    if !plane {
        let q = profile.bending_quadratic();
        let mut gradient = [0.0; 10];
        for (i, row) in q.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                gradient[i] += value * state.profile[j];
            }
        }
        for (i, row) in q.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                terms.push(ExtraTerm {
                    row: PROFILE + i,
                    column: PROFILE + j,
                    second: 0.5 * BENDING_WEIGHT * value,
                    first: 0.5 * BENDING_WEIGHT * gradient[i],
                });
            }
        }
    }
    if fit_focal && camera.f > 0.0 && state.focal > 0.0 {
        let difference = state.focal.ln() - camera.f.ln();
        let gradient = difference / (FOCAL_SPREAD * FOCAL_SPREAD * state.focal);
        let second = (1.0 - difference) / (FOCAL_SPREAD * FOCAL_SPREAD * state.focal * state.focal);
        terms.push(ExtraTerm { row: FOCAL, column: FOCAL, second, first: gradient });
    }
    terms
}

/// The decrease the bending penalty's model predicts.
fn model_bending(state: &State, profile: &Profile, plane: bool, delta: &[f64]) -> f64 {
    if plane {
        return 0.0;
    }
    let mut next = state.profile;
    for (j, value) in next.iter_mut().enumerate() {
        *value += delta.get(PROFILE + j).copied().unwrap_or(0.0);
    }
    0.5 * BENDING_WEIGHT * (profile.bending_integral(&next) - profile.bending_integral(&state.profile))
}

/// The decrease the focal prior's model predicts.
fn model_focal(state: &State, camera: &Camera, fit_focal: bool, delta: &[f64]) -> f64 {
    if !fit_focal || camera.f <= 0.0 || state.focal <= 0.0 {
        return 0.0;
    }
    let step = delta.get(FOCAL).copied().unwrap_or(0.0);
    let difference = state.focal.ln() - camera.f.ln();
    let gradient = difference / (FOCAL_SPREAD * FOCAL_SPREAD * state.focal);
    let second = (1.0 - difference) / (FOCAL_SPREAD * FOCAL_SPREAD * state.focal * state.focal);
    gradient * step + 0.5 * second * step * step
}

/// Applies a step to the state.
fn apply_step(state: &mut State, delta: &[f64], own: &[f64], fit_focal: bool, plane: bool) {
    let omega = [
        delta.get(ROTATION).copied().unwrap_or(0.0),
        delta.get(ROTATION + 1).copied().unwrap_or(0.0),
        delta.get(ROTATION + 2).copied().unwrap_or(0.0),
    ];
    let rotation = crate::solver::rodrigues(omega);
    state.rotation = crate::solver::mat_mul(&rotation, &state.rotation);
    for (k, value) in state.translation.iter_mut().enumerate() {
        *value += delta.get(TRANSLATION + k).copied().unwrap_or(0.0);
    }
    state.chord += delta.get(CHORD).copied().unwrap_or(0.0);
    if !plane {
        for (j, value) in state.profile.iter_mut().enumerate() {
            *value += delta.get(PROFILE + j).copied().unwrap_or(0.0);
        }
    }
    if fit_focal {
        state.focal += delta.get(FOCAL).copied().unwrap_or(0.0);
    }
    let first_line = shared_count(fit_focal, 0);
    for (index, value) in state.lines.iter_mut().enumerate() {
        *value += delta.get(first_line + index).copied().unwrap_or(0.0);
        *value = value.clamp(COORD_MIN, COORD_MAX);
    }
    for (index, value) in state.points.iter_mut().enumerate() {
        *value += own.get(index).copied().unwrap_or(0.0);
        *value = value.clamp(COORD_MIN, COORD_MAX);
    }
}

/// The cost of a state, from scratch.
fn evaluate(state: &State, prepared: &Prepared, camera: &Camera, profile: &Profile, fit_focal: bool, plane: bool) -> Option<f64> {
    let (_rows, residual_cost) = build_rows(state, prepared, camera, profile, fit_focal, plane)?;
    Some(residual_cost + bending_cost(state, profile, plane) + focal_cost(state, camera, fit_focal))
}

/// An unrolled sheet (A28): a [`PageMap`] from the flat page to the photo.
#[derive(Debug, Clone, PartialEq)]
pub struct Unrolled {
    sheet: Sheet,
    camera: Camera,
}

impl Unrolled {
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }
    pub fn camera(&self) -> &Camera {
        &self.camera
    }
    /// Width ÷ height of the flattened page (A28 step 2).
    pub fn ratio(&self) -> Result<f64, GeomError> {
        not_implemented(Step::A28)
    }
    /// A11 step 7's Ŵ and Ĥ measured along the fitted outline (A28 step 3).
    pub fn side_lengths_px(&self) -> Result<(f64, f64), GeomError> {
        not_implemented(Step::A28)
    }
    /// The fitted outline's four sides, 256 points each, top, right, bottom, left (A28 step 6).
    pub fn outline(&self) -> Result<[Vec<Point>; 4], GeomError> {
        not_implemented(Step::A28)
    }
    /// The fitted outline's corners.
    pub fn corners(&self) -> Result<Quad, GeomError> {
        not_implemented(Step::A28)
    }
}

impl PageMap for Unrolled {
    fn map(&self, u: f64, v: f64) -> Option<Point> {
        let _ = (u, v);
        None
    }
}

/// A28 steps 1–5.
pub fn unroll(sheet: &Sheet, camera: &Camera) -> Result<Unrolled, GeomError> {
    let _ = (sheet, camera);
    not_implemented(Step::A28)
}
