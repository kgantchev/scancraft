//! A17 (Color Document, Grayscale) and A18 (Black & White).

use crate::EnhanceError;
use crate::even_light::EvenLight;
use scancraft_core::image::{BitPlane, PlaneF32, RgbF32};
use scancraft_core::{Step, not_implemented};

pub fn color_document(light: &EvenLight, strength: u8, dpi: f64) -> Result<RgbF32, EnhanceError> {
    let _ = (light, strength, dpi);
    not_implemented(Step::A17)
}

pub fn grayscale(light: &EvenLight, strength: u8, dpi: f64) -> Result<PlaneF32, EnhanceError> {
    let _ = (light, strength, dpi);
    not_implemented(Step::A17)
}

/// `dpi` is the Black & White output DPI (300, or 400 with Best quality).
pub fn black_white(light: &EvenLight, strength: u8, dpi: f64) -> Result<BitPlane, EnhanceError> {
    let _ = (light, strength, dpi);
    not_implemented(Step::A18)
}
