//! PNG encoding for the hand-off to Vision (A8 step 1, A20 step 1, A15 step 2).
//!
//! Vision takes an image as bytes in an `NSData`, so each image is written as a PNG with fast
//! compression. Nothing here touches Apple frameworks, so the encoding is tested on every
//! platform.

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use scancraft_core::image::{Gray8, Rgb8};
use scancraft_core::traits::ServiceError;

/// Writes a grey page as an 8-bit greyscale PNG (A20 step 1, A15 step 2).
pub(crate) fn encode_gray_png(page: &Gray8) -> Result<Vec<u8>, ServiceError> {
    encode(page.data(), page.width(), page.height(), 1, ExtendedColorType::L8)
}

/// Writes a colour image as an 8-bit RGB PNG (A8 step 1).
pub(crate) fn encode_rgb_png(image: &Rgb8) -> Result<Vec<u8>, ServiceError> {
    let flat: Vec<u8> = image.data().iter().flatten().copied().collect();
    encode(&flat, image.width(), image.height(), 3, ExtendedColorType::Rgb8)
}

/// `PngEncoder::write_image` asserts that the buffer holds exactly width × height × channels
/// bytes and panics otherwise, so the length is checked first and a mismatch becomes an error.
fn encode(bytes: &[u8], width: u32, height: u32, channels: u64, colour: ExtendedColorType) -> Result<Vec<u8>, ServiceError> {
    let expected = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(channels));
    if width == 0 || height == 0 || expected != u64::try_from(bytes.len()).ok() {
        return Err(ServiceError::Failed("the image buffer does not match its size".to_owned()));
    }
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(bytes, width, height, colour)
        .map_err(|e| ServiceError::Failed(format!("PNG encoding failed: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{encode, encode_gray_png, encode_rgb_png};
    use image::ExtendedColorType;
    use scancraft_core::image::Plane;

    /// The fields of a PNG's IHDR chunk: width, height, bit depth, colour type.
    fn header(png: &[u8]) -> (u32, u32, u8, u8) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A], "PNG signature");
        assert_eq!(&png[12..16], b"IHDR");
        let be = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
        (be(16), be(20), png[24], png[25])
    }

    #[test]
    fn encode_gray_png_writes_an_eight_bit_greyscale_png() {
        let page = Plane::from_vec(5, 3, (0..15u8).map(|i| i * 17).collect()).unwrap();
        let png = encode_gray_png(&page).unwrap();
        // Colour type 0 is greyscale.
        assert_eq!(header(&png), (5, 3, 8, 0));
        // The same input gives the same bytes.
        assert_eq!(png, encode_gray_png(&page).unwrap());
        // And the pixels survive the round trip.
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().into_luma8();
        assert_eq!(back.as_raw().as_slice(), page.data());
    }

    #[test]
    fn encode_rgb_png_writes_an_eight_bit_rgb_png() {
        let image = Plane::from_vec(2, 2, vec![[1u8, 2, 3], [4, 5, 6], [7, 8, 9], [250, 251, 252]]).unwrap();
        let png = encode_rgb_png(&image).unwrap();
        // Colour type 2 is truecolour.
        assert_eq!(header(&png), (2, 2, 8, 2));
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().into_rgb8();
        assert_eq!(back.as_raw().as_slice(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 250, 251, 252]);
    }

    #[test]
    fn a_buffer_of_the_wrong_length_is_an_error_not_a_panic() {
        // `write_image` would panic on this; the length check turns it into an error.
        assert!(encode(&[0u8; 5], 2, 3, 1, ExtendedColorType::L8).is_err());
        assert!(encode(&[0u8; 6], 2, 3, 3, ExtendedColorType::Rgb8).is_err());
        assert!(encode(&[], 0, 0, 1, ExtendedColorType::L8).is_err());
        // A size whose product overflows is an error as well.
        assert!(encode(&[0u8; 1], u32::MAX, u32::MAX, 3, ExtendedColorType::Rgb8).is_err());
    }
}
