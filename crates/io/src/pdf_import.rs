//! A24: PDF import through craft-pdf's bounded reader and rasteriser.

use crate::IoError;
use scancraft_core::image::Rgb8;
use scancraft_core::{Step, not_implemented};

/// An opened PDF.
pub struct PdfDocument {
    bytes: Vec<u8>,
}

impl PdfDocument {
    pub fn open(bytes: Vec<u8>) -> Result<PdfDocument, IoError> {
        let _ = PdfDocument { bytes };
        not_implemented(Step::A24)
    }

    pub fn page_count(&self) -> u32 {
        let _ = &self.bytes;
        0
    }

    /// Renders page `index` at 300 DPI, lowered to fit 40 megapixels, composited over white.
    pub fn render(&self, index: u32) -> Result<Rgb8, IoError> {
        let _ = index;
        not_implemented(Step::A24)
    }
}
