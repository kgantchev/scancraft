//! A6: contour candidates (specification A6). Owner: WP8.

use crate::DetectError;
use crate::edges::EdgeMaps;
use scancraft_core::geom::Quad;
use scancraft_core::{Step, not_implemented};

/// Candidate quadrilaterals in work-image pixels, canonical order.
pub fn contour_candidates(maps: &EdgeMaps) -> Result<Vec<Quad>, DetectError> {
    let _ = maps;
    not_implemented(Step::A6)
}
