//! A19: shared helpers (specification A19). Owner: WP2.

use crate::RasterError;
use scancraft_core::image::{BitPlane, Plane, PlaneF32};
use scancraft_core::{Step, not_implemented};

/// Minimum filter over a square of odd side `w` (van Herk; Gil and Werman).
pub fn min_filter(src: &PlaneF32, w: u32) -> Result<PlaneF32, RasterError> {
    let _ = (src, w);
    not_implemented(Step::A19)
}

/// Maximum filter over a square of odd side `w`.
pub fn max_filter(src: &PlaneF32, w: u32) -> Result<PlaneF32, RasterError> {
    let _ = (src, w);
    not_implemented(Step::A19)
}

/// Closing with an octagon of radius `r`, standing in for a disk.
pub fn octagon_close(src: &PlaneF32, r: u32) -> Result<PlaneF32, RasterError> {
    let _ = (src, r);
    not_implemented(Step::A19)
}

/// 3 × 3 median (Paeth's sorting network).
pub fn median3(src: &PlaneF32) -> Result<PlaneF32, RasterError> {
    let _ = src;
    not_implemented(Step::A19)
}

/// Unsharp mask: Y + amount · (Y − G_σ ∗ Y) where the difference exceeds `threshold`.
pub fn unsharp(src: &PlaneF32, sigma: f32, amount: f32, threshold: f32) -> Result<PlaneF32, RasterError> {
    let _ = (src, sigma, amount, threshold);
    not_implemented(Step::A19)
}

/// 8-connected components of the set bits: a label per pixel (0 for unset) and the count.
pub fn connected_components(bits: &BitPlane) -> Result<(Plane<u32>, u32), RasterError> {
    let _ = bits;
    not_implemented(Step::A19)
}

/// Normalized convolution (Knutsson and Westin): (G_σ ∗ (c · I)) / (G_σ ∗ c), with the local
/// confidence G_σ ∗ c returned too.
pub fn normalized_convolution(values: &PlaneF32, confidence: &BitPlane, sigma: f32) -> Result<(PlaneF32, PlaneF32), RasterError> {
    let _ = (values, confidence, sigma);
    not_implemented(Step::A19)
}
