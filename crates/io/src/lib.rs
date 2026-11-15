//! ScanCraft import and export (specification §6.1, L2). Owner: WP3 (build plan §4).
//!
//! The text layer, word fitting, Type3 fonts and image pages are ported from PdfCraft
//! (Apache-2.0) at a recorded commit; each ported file keeps PdfCraft's notice (§5.3, `NOTICE`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod encode;
pub mod import;
pub mod pdf_import;
pub mod pdf_write;
pub mod text_layer;

use scancraft_core::CoreError;
use scancraft_raster::RasterError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum IoError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Raster(#[from] RasterError),
    /// The file isn't a supported format, with the reason shown to the user (§4.8).
    #[error("unsupported file: {0}")]
    Unsupported(String),
    #[error("this PDF is password-protected")]
    Encrypted,
    #[error("file system: {0}")]
    Fs(String),
}
