//! A18 steps 1–5, the lightest/darkest midpoint rule, shared by A18 and A26. Owner: WP2.

use crate::RasterError;
use scancraft_core::image::{BitPlane, PlaneF32};
use scancraft_core::{Step, not_implemented};

/// Ink where Y < 0.25, or where M − m ≥ g and Y ≤ (M + m)/2 (A18 step 5), at `dpi` pixels per
/// inch and `strength` 0..=100.
pub fn midpoint_ink(luma: &PlaneF32, dpi: f64, strength: u8) -> Result<BitPlane, RasterError> {
    let _ = (luma, dpi, strength);
    not_implemented(Step::A18)
}
