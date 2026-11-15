//! A16: Even Light (specification A16).

use crate::EnhanceError;
use scancraft_core::image::{BitPlane, RgbF32};
use scancraft_core::{Step, not_implemented};

/// A16's output: the lighting-corrected page and the object mask O (photos, logos, fingers).
#[derive(Debug, Clone, PartialEq)]
pub struct EvenLight {
    pub corrected: RgbF32,
    /// At the analysis copy's size (long side 1000 px).
    pub objects: BitPlane,
}

/// `page` is the warped page at output resolution; `dpi` its density.
pub fn even_light(page: &RgbF32, dpi: f64, keep_paper_colour: bool) -> Result<EvenLight, EnhanceError> {
    let _ = (page, dpi, keep_paper_colour);
    not_implemented(Step::A16)
}
