//! A7: line candidates (specification A7). Owner: WP8.

use crate::DetectError;
use crate::edges::EdgeMaps;
use scancraft_core::geom::Quad;
use scancraft_core::{Step, not_implemented};

/// Candidate quadrilaterals in work-image pixels, canonical order, at most 1024.
pub fn line_candidates(maps: &EdgeMaps) -> Result<Vec<Quad>, DetectError> {
    let _ = maps;
    not_implemented(Step::A7)
}
