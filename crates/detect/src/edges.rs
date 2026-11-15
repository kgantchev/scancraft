//! A5: colour edges (specification A5). Owner: WP8.

use crate::DetectError;
use scancraft_core::image::{BitPlane, PlaneF32, RgbF32};
use scancraft_core::{Step, not_implemented};

/// A5's output: both edge maps, plus magnitude G and direction θ for A9 and A10.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeMaps {
    pub strict: BitPlane,
    pub sensitive: BitPlane,
    pub magnitude: PlaneF32,
    /// Radians.
    pub direction: PlaneF32,
}

/// A5 on the work image (RGB, long side 1024 px).
pub fn colour_edges(work: &RgbF32) -> Result<EdgeMaps, DetectError> {
    let _ = work;
    not_implemented(Step::A5)
}
