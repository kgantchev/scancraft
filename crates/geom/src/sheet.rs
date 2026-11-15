//! A27 (sheet fit) and A28 (unrolling). Owner: WP1. The cubic spline and the Levenberg–Marquardt
//! solver are WP1's own modules; only what other packages call is fixed here.

use crate::GeomError;
use scancraft_core::camera::Camera;
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::map::PageMap;
use scancraft_core::recipe::Sheet;
use scancraft_core::{Step, not_implemented};

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
    let _ = (evidence, camera, fit_focal);
    not_implemented(Step::A27)
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
