//! A10: sub-pixel edge refinement, and the luma pyramid it builds (specification A10). Owner: WP8.

use crate::{DetectError, Polarity};
use scancraft_core::geom::Quad;
use scancraft_core::image::PlaneF32;
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::{Step, not_implemented};

/// Builds the pyramid from the full-resolution luma (level 0 at long side 1024 px).
pub fn build_pyramid(full_luma: &PlaneF32) -> Result<LumaPyramid, DetectError> {
    let _ = full_luma;
    not_implemented(Step::A10)
}

/// A10's result, in full-resolution photo pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Refined {
    pub corners: Quad,
    /// Sides kept as they were (A10 step 7), top, right, bottom, left.
    pub unrefined: [bool; 4],
    pub polarity: [Polarity; 4],
}

/// Refines `quad`, given in work-image pixels; `seed` comes from `determinism::seed_for`.
pub fn refine(quad: &Quad, pyramid: &LumaPyramid, seed: u64) -> Result<Refined, DetectError> {
    let _ = (quad, pyramid, seed);
    not_implemented(Step::A10)
}
