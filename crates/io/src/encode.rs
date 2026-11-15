//! A22: image encoding (JPEG through craft-jpeg, JBIG2 generic through craft-stream-codecs).

use crate::IoError;
use scancraft_core::image::{BitPlane, Gray8, Rgb8};
use scancraft_core::recipe::Quality;
use scancraft_core::{Step, not_implemented};

/// A finished page image, ready to encode.
#[derive(Debug, Clone, PartialEq)]
pub enum PageImage {
    Colour(Rgb8),
    Grey(Gray8),
    Bits(BitPlane),
}

/// The bytes that go into the PDF unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodedImage {
    Jpeg {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
    },
    Jbig2 {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
    /// The fallback when JBIG2 encoding fails: Flate-compressed 1-bit rows.
    Flate1Bit {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
}

pub fn encode(image: &PageImage, quality: Quality) -> Result<EncodedImage, IoError> {
    let _ = (image, quality);
    not_implemented(Step::A22)
}
