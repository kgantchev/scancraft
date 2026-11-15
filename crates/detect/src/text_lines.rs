//! A26: text lines, a port of pdfcraft-android's strip tracker (specification A26). Owner: WP10.

use crate::DetectError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::PlaneF32;
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::{Step, not_implemented};
use scancraft_geom::sheet::{LineEvidence, TextDirection};

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
    let _ = (copy, dpi);
    not_implemented(Step::A26)
}

/// Steps 1–7 for a page outline: the evidence A27 fits to, in photo pixels. `None` when fewer
/// than 4 lines are found.
pub fn text_evidence(
    pyramid: &LumaPyramid,
    corners: &Quad,
    page_size_mm: (f64, f64),
) -> Result<Option<(Vec<LineEvidence>, TextDirection)>, DetectError> {
    let _ = (pyramid, corners, page_size_mm);
    not_implemented(Step::A26)
}
