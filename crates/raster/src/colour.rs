//! sRGB and Oklab (specification §7.3, A19). Owner: WP2.

use crate::RasterError;
use scancraft_core::image::{Rgb8, RgbF32};
use scancraft_core::{Step, not_implemented};

/// sRGB 8-bit to f32 in [0, 1], still sRGB-encoded.
pub fn to_f32(src: &Rgb8) -> Result<RgbF32, RasterError> {
    let _ = src;
    not_implemented(Step::A19)
}

/// f32 in [0, 1] to 8 bits, clamped and rounded.
pub fn to_u8(src: &RgbF32) -> Result<Rgb8, RasterError> {
    let _ = src;
    not_implemented(Step::A19)
}

/// Oklab (Ottosson 2020) of one sRGB-encoded colour.
pub fn oklab(rgb: [f32; 3]) -> Result<[f32; 3], RasterError> {
    let _ = rgb;
    not_implemented(Step::A19)
}
