//! A1 (decode and orient) and A2 (convert to sRGB).

use crate::IoError;
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{FocalInput, ImportOrigin};
use scancraft_core::{Step, not_implemented};

/// One decoded, upright, sRGB image; a multi-page TIFF gives several.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedImage {
    pub rgb: Rgb8,
    pub focal: FocalInput,
    pub origin: ImportOrigin,
}

/// A1 and A2 on a file's bytes: format from the bytes, size limits, EXIF and HEIF orientation,
/// transparency over white, ICC or nclx to sRGB.
pub fn load(bytes: &[u8]) -> Result<Vec<LoadedImage>, IoError> {
    let _ = bytes;
    not_implemented(Step::A1)
}

/// The hex SHA-256 that names a source in the session folder (A1 step 6).
pub fn source_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
