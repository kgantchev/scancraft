//! A23: PDF assembly with pdf-writer, written through craft-atomic.

use crate::IoError;
use crate::encode::EncodedImage;
use crate::text_layer::TextLayer;
use scancraft_core::{Step, not_implemented};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct PdfPage {
    pub image: EncodedImage,
    /// MediaBox width and height in points.
    pub size_pt: (f64, f64),
    /// Where the image sits on the page, in points: x, y, width, height (fitted, never stretched).
    pub image_rect_pt: [f64; 4],
    pub text: Option<TextLayer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocInfo {
    pub title: String,
    /// "ScanCraft <version> (algorithms <ALGO_VERSION>)".
    pub producer: String,
}

/// Writes the PDF to a temporary file next to `dest`, flushes it, then renames it over `dest`.
pub fn write_pdf(pages: &[PdfPage], info: &DocInfo, dest: &Path) -> Result<(), IoError> {
    let _ = (pages, info, dest);
    not_implemented(Step::A23)
}
