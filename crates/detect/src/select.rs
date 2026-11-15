//! A9: score and select (specification A9). Owner: WP8; WP11 adds the scoring of curved sides.

use crate::DetectError;
use crate::edges::EdgeMaps;
use scancraft_core::geom::Quad;
use scancraft_core::image::RgbF32;
use scancraft_core::{Step, not_implemented};

/// A9 step 5's score of one candidate in work-image pixels; `tau` is 3 before refinement, 2 after.
pub fn score(quad: &Quad, maps: &EdgeMaps, work: &RgbF32, tau: f64) -> Result<f64, DetectError> {
    let _ = (quad, maps, work, tau);
    not_implemented(Step::A9)
}
