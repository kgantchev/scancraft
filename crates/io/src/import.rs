//! A1 (decode and orient) and A2 (convert to sRGB).
//!
//! The decoder (`craft-image-codecs`) identifies the format from the bytes, enforces the size
//! limits before it allocates, and applies the EXIF orientation and HEIF's `irot`, `imir` and
//! `clap`. This module then composites transparency over white at the source's depth, converts to
//! sRGB at full depth with `craft-cms`, rounds to 8 bits, and reads the two EXIF values A11 needs.

use crate::IoError;
use craft_cms::builtin::{Builtin, cicp_rgb};
use craft_cms::{ColorSpace, Intent, Profile, TransformOptions, cached};
use craft_image_codecs::{ChannelLayout, DecodeOptions, Format, Image, Limits, SampleType};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSlice, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, MAX_PIXELS, MAX_SIDE, Plane, Rgb8};
use scancraft_core::recipe::{FocalInput, ImportOrigin};

/// The most TIFF pages one file gives, as for a PDF (A24).
pub const MAX_PAGES: usize = 500;
/// The most pixels all the pages of one file may hold together.
const MAX_TOTAL_PIXELS: u64 = 4 * MAX_PIXELS;
/// The decoder's allocation budget, as its own default on a 64-bit machine.
const MAX_ALLOC: u64 = 8 << 30;
/// EXIF's `DigitalZoomRatio`.
const TAG_DIGITAL_ZOOM: u16 = 0xA404;

/// One decoded, upright, sRGB image; a multi-page TIFF gives several.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedImage {
    pub rgb: Rgb8,
    pub focal: FocalInput,
    pub origin: ImportOrigin,
}

fn decode_options() -> DecodeOptions {
    DecodeOptions { limits: Limits::new(MAX_SIDE, MAX_SIDE, MAX_PIXELS, MAX_ALLOC), keep_orientation: false }
}

fn unsupported(e: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(e.to_string())
}

/// A1 and A2 on a file's bytes: format from the bytes, size limits, EXIF and HEIF orientation,
/// transparency over white, ICC or nclx to sRGB.
pub fn load(bytes: &[u8]) -> Result<Vec<LoadedImage>, IoError> {
    if bytes.is_empty() {
        return Err(IoError::Unsupported("the file is empty".to_string()));
    }
    let format = craft_image_codecs::detect(bytes).ok_or_else(|| IoError::Unsupported("this is not a supported image format".to_string()))?;
    let options = decode_options();
    if format != Format::Tiff {
        let image = craft_image_codecs::decode_as_with(format, bytes, &options).map_err(unsupported)?;
        return Ok(vec![finish(&image, ImportOrigin::File)?]);
    }
    let pages = tiff_main_pages(bytes)?;
    if pages.len() <= 1 {
        let image = craft_image_codecs::decode_as_with(format, bytes, &options).map_err(unsupported)?;
        return Ok(vec![finish(&image, ImportOrigin::File)?]);
    }
    let mut out = Vec::with_capacity(pages.len().min(MAX_PAGES));
    let mut total = 0u64;
    for (index, page) in pages.iter().take(MAX_PAGES).enumerate() {
        total = total.saturating_add(u64::from(page.1) * u64::from(page.2));
        if total > MAX_TOTAL_PIXELS {
            return Err(IoError::Unsupported("the TIFF's pages are too large to import together".to_string()));
        }
        let image = craft_image_codecs::decode_tiff_page(bytes, page.0, &options).map_err(unsupported)?;
        out.push(finish(&image, ImportOrigin::TiffPage { index: index as u32 })?);
    }
    Ok(out)
}

/// The main pages of a TIFF, as (index into the file's directories, width, height).
fn tiff_main_pages(bytes: &[u8]) -> Result<Vec<(usize, u32, u32)>, IoError> {
    use craft_image_codecs::TiffPageKind;
    let info = craft_image_codecs::tiff_info(bytes).map_err(unsupported)?;
    Ok(info
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.parent.is_none() && p.kind == TiffPageKind::Page)
        .map(|(i, p)| (i, p.width, p.height))
        .collect())
}

/// How many pages a TIFF holds, so a caller can say how many `load` left out (A1; as A24 does).
pub fn tiff_page_count(bytes: &[u8]) -> usize {
    tiff_main_pages(bytes).map_or(0, |pages| pages.len())
}

/// The hex SHA-256 that names a source in the session folder (A1 step 6).
pub fn source_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// One decoded image to sRGB and the focal input.
fn finish(image: &Image, origin: ImportOrigin) -> Result<LoadedImage, IoError> {
    let rgb = to_srgb(image)?;
    Ok(LoadedImage { rgb, focal: focal_input(image.meta.exif.as_deref()), origin })
}

/// A1 step 5: `FocalLengthIn35mmFilm` and `DigitalZoomRatio`; nothing else of the EXIF is kept.
fn focal_input(exif: Option<&[u8]>) -> FocalInput {
    let Some(exif) = exif else { return FocalInput::default() };
    let usable = |v: f64| (v.is_finite() && v > 0.0).then_some(v);
    let f35_mm = craft_metadata::read_exif(exif).focal_length_35mm.and_then(usable);
    // The metadata crate has no field for the zoom, so it comes from the tag rows, whose value is
    // a trimmed decimal, or a dash when the denominator is 0.
    let digital_zoom = craft_metadata::file_tag_rows(exif)
        .into_iter()
        .find(|row| row.tag == TAG_DIGITAL_ZOOM && row.group == "EXIF")
        .and_then(|row| row.value.trim().parse::<f64>().ok())
        .and_then(usable);
    FocalInput { f35_mm, digital_zoom }
}

/// The samples of three colour channels, at the source's depth.
enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

/// A1 step 4 and A2: alpha over white, then the colour conversion, then 8 bits.
fn to_srgb(image: &Image) -> Result<Rgb8, IoError> {
    let (width, height) = image.dimensions();
    let source = source_profile(image);
    let layout = if image.layout().has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    let sample = match image.sample_type() {
        SampleType::U8 => SampleType::U8,
        SampleType::U16 => SampleType::U16,
        SampleType::F16 | SampleType::F32 => SampleType::F32,
    };
    // Grey, CMYK and Lab become RGB here; the conversion drops an ICC profile of another model,
    // so such an image is then treated as sRGB (A2 step 3).
    let image = image.converted(layout, sample);
    let row = width as usize * layout.channels();
    let samples = match sample {
        SampleType::U8 => Samples::U8(composite_u8(image.data(), row, layout)),
        SampleType::U16 => {
            let data = image.to_u16_samples().ok_or_else(|| IoError::Unsupported("the image's samples could not be read".to_string()))?;
            Samples::U16(composite_u16(&data, row, layout))
        }
        _ => {
            let data = image.to_f32_samples().ok_or_else(|| IoError::Unsupported("the image's samples could not be read".to_string()))?;
            Samples::F32(composite_f32(&data, row, layout))
        }
    };
    let bytes = match source.as_ref().map(|profile| (profile, cached(profile, Builtin::Srgb.profile(), transform_options()))) {
        Some((_, Ok(transform))) => convert(&transform, samples)?,
        _ => to_u8(samples),
    };
    let pixels = bytes.as_chunks::<3>().0.to_vec();
    Ok(Plane::from_vec(width, height, pixels)?)
}

fn transform_options() -> TransformOptions {
    TransformOptions { intent: Intent::RelativeColorimetric, bpc: true, precise_float: true }
}

/// The profile the pixels are in, when it isn't sRGB already: the embedded ICC profile (A2 step
/// 1), else the built-in profile an `nclx` names (step 2). `None` means sRGB (step 3).
fn source_profile(image: &Image) -> Option<Profile> {
    let srgb = Builtin::Srgb.profile().to_bytes();
    let profile = match &image.icc {
        // A profile that doesn't parse is not a broken file: the pixels are taken as sRGB.
        Some(bytes) => Profile::parse(bytes).ok(),
        // A built-in profile is parsed from its own bytes, like an embedded one. The shared
        // transform store is keyed by the profile's bytes, and a transform built from the exact
        // in-memory profile differs by a level from one built from the parsed bytes, so which one
        // was built first would otherwise decide the pixels.
        None => image
            .cicp
            .and_then(|c| cicp_rgb(c.primaries, c.transfer, image.sample_type().is_float()))
            .and_then(|p| Profile::parse(&p.to_bytes()).ok()),
    }?;
    // A transform from sRGB to sRGB would only add rounding.
    if profile.to_bytes() == srgb {
        return None;
    }
    // Only RGB profiles fit RGB pixels.
    (profile.color_space == ColorSpace::Rgb).then_some(profile)
}

fn convert(transform: &craft_cms::Transform, samples: Samples) -> Result<Vec<u8>, IoError> {
    let wrong = |e: craft_cms::CmsError| IoError::Unsupported(format!("the colour profile could not be applied: {e}"));
    match samples {
        Samples::U8(src) => {
            let mut dst = vec![0u8; src.len()];
            transform.convert_u8(&src, 3, &mut dst, 3, false).map_err(wrong)?;
            Ok(dst)
        }
        Samples::U16(src) => {
            let mut dst = vec![0u16; src.len()];
            transform.convert_u16(&src, 3, &mut dst, 3, false).map_err(wrong)?;
            Ok(to_u8(Samples::U16(dst)))
        }
        Samples::F32(src) => {
            let mut dst = vec![0f32; src.len()];
            transform.convert_f32(&src, 3, &mut dst, 3, false).map_err(wrong)?;
            Ok(to_u8(Samples::F32(dst)))
        }
    }
}

/// The nearest 8-bit value of a 16-bit one: v × 255 / 65535, rounded (not truncated).
fn round_16_to_8(v: u16) -> u8 {
    ((u32::from(v) + 128) / 257) as u8
}

fn to_u8(samples: Samples) -> Vec<u8> {
    match samples {
        Samples::U8(v) => v,
        Samples::U16(v) => samples_map(&v, round_16_to_8),
        Samples::F32(v) => samples_map(&v, |x| if x.is_finite() { (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 }),
    }
}

/// Maps every sample in fixed chunks. Each output depends on its own input only, so the result
/// can't depend on the number of threads (§7.6).
fn samples_map<T: Copy + Sync, U: Copy + Send + Default>(src: &[T], f: impl Fn(T) -> U + Sync) -> Vec<U> {
    const CHUNK: usize = 1 << 16;
    let mut out = vec![U::default(); src.len()];
    out.par_chunks_mut(CHUNK).zip(src.par_chunks(CHUNK)).for_each(|(o, s)| {
        for (slot, &v) in o.iter_mut().zip(s) {
            *slot = f(v);
        }
    });
    out
}

/// c′ = a·c + (1 − a) on 8-bit samples, rounded; drops the alpha. `row` is the samples per row.
fn composite_u8(data: &[u8], row: usize, layout: ChannelLayout) -> Vec<u8> {
    if !layout.has_alpha() {
        return data.to_vec();
    }
    over_white(data, row, 4, |px, out| {
        let a = u32::from(px[3]);
        for c in 0..3 {
            out[c] = ((a * u32::from(px[c]) + (255 - a) * 255 + 127) / 255) as u8;
        }
    })
}

/// The same on 16-bit samples.
fn composite_u16(data: &[u16], row: usize, layout: ChannelLayout) -> Vec<u16> {
    if !layout.has_alpha() {
        return data.to_vec();
    }
    over_white(data, row, 4, |px, out| {
        let a = u64::from(px[3]);
        for c in 0..3 {
            out[c] = ((a * u64::from(px[c]) + (65535 - a) * 65535 + 32767) / 65535) as u16;
        }
    })
}

/// The same on float samples.
fn composite_f32(data: &[f32], row: usize, layout: ChannelLayout) -> Vec<f32> {
    if !layout.has_alpha() {
        return data.to_vec();
    }
    over_white(data, row, 4, |px, out| {
        let a = if px[3].is_finite() { px[3].clamp(0.0, 1.0) } else { 1.0 };
        for c in 0..3 {
            out[c] = a * px[c] + (1.0 - a);
        }
    })
}

/// Runs `f` on every 4-sample pixel, writing 3 samples, over bands of 64 rows.
fn over_white<T: Copy + Send + Sync + Default>(data: &[T], row: usize, stride: usize, f: impl Fn(&[T], &mut [T]) + Sync) -> Vec<T> {
    let pixels = data.len() / stride;
    let mut out = vec![T::default(); pixels * 3];
    let width = (row / stride).max(1);
    let band_pixels = width * BAND_ROWS as usize;
    out.par_chunks_mut(band_pixels * 3).zip(data.par_chunks(band_pixels * stride)).for_each(|(o, s)| {
        for (dst, src) in o.chunks_exact_mut(3).zip(s.chunks_exact(stride)) {
            f(src, dst);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{MAX_PAGES, focal_input, load, round_16_to_8, source_id, to_srgb};
    use craft_cms::builtin::Builtin;
    use craft_cms::{Intent, Transform};
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    use scancraft_core::image::Rgb8;
    use scancraft_core::recipe::ImportOrigin;

    // ---- fixtures ----

    /// A little-endian TIFF EXIF block with an orientation, a digital zoom and a 35 mm focal length.
    fn exif_block(orientation: u16, zoom: Option<(u32, u32)>, f35: Option<u16>) -> Vec<u8> {
        let mut entries: Vec<(u16, u16, u32)> = vec![(0x0112, 3, u32::from(orientation))];
        let exif_ifd_offset = 8 + 2 + 2 * 12 + 4;
        entries.push((0x8769, 4, exif_ifd_offset));
        let mut exif_entries: Vec<(u16, u16, u32)> = Vec::new();
        let rational_offset = exif_ifd_offset + 2 + 2 * 12 + 4;
        if zoom.is_some() {
            exif_entries.push((0xA404, 5, rational_offset));
        }
        if let Some(f) = f35 {
            exif_entries.push((0xA405, 3, u32::from(f)));
        }
        let mut out = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        let ifd = |entries: &[(u16, u16, u32)], out: &mut Vec<u8>| {
            out.extend((entries.len() as u16).to_le_bytes());
            for &(tag, kind, value) in entries {
                out.extend(tag.to_le_bytes());
                out.extend(kind.to_le_bytes());
                out.extend(1u32.to_le_bytes());
                out.extend(value.to_le_bytes());
            }
            out.extend(0u32.to_le_bytes());
        };
        ifd(&entries, &mut out);
        // Pad the Exif IFD's entry count to a fixed size so the offsets above hold.
        let before = out.len();
        ifd(&exif_entries, &mut out);
        out.resize(before + 2 + 2 * 12 + 4, 0);
        if let Some((n, d)) = zoom {
            out.extend(n.to_le_bytes());
            out.extend(d.to_le_bytes());
        }
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// A PNG of `image`; with `exif`, an `eXIf` chunk placed before the image data, where decoders
    /// read an orientation from it.
    fn png_with(image: &Image, exif: Option<Vec<u8>>) -> Vec<u8> {
        let png = craft_image_codecs::encode(image, Format::Png, &EncodeOptions { embed_icc: true, ..EncodeOptions::default() }).unwrap();
        let Some(exif) = exif else { return png };
        // The signature (8 bytes) and the IHDR chunk (4 + 4 + 13 + 4) come first.
        let after_ihdr = 8 + 25;
        let mut chunk = b"eXIf".to_vec();
        chunk.extend(&exif);
        let mut out = png[..after_ihdr].to_vec();
        out.extend((exif.len() as u32).to_be_bytes());
        out.extend(&chunk);
        out.extend(crc32(&chunk).to_be_bytes());
        out.extend(&png[after_ihdr..]);
        out
    }

    /// An asymmetric upright picture of 7 × 5 pixels, so that any mix-up of axes shows.
    fn upright() -> Vec<[u8; 3]> {
        (0..35u32).map(|i| [(i * 7 % 256) as u8, (i * 31 % 256) as u8, (255 - i * 5) as u8]).collect()
    }

    /// What a decoder does to the stored pixels for each EXIF orientation value.
    fn orient(pixels: &[[u8; 3]], w: usize, h: usize, o: u16) -> (Vec<[u8; 3]>, usize, usize) {
        let get = |x: usize, y: usize| pixels[y * w + x];
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        let mut out = vec![[0u8; 3]; ow * oh];
        for y in 0..oh {
            for x in 0..ow {
                // The stored position that lands on output (x, y).
                let (sx, sy) = match o {
                    1 => (x, y),
                    2 => (w - 1 - x, y),
                    3 => (w - 1 - x, h - 1 - y),
                    4 => (x, h - 1 - y),
                    5 => (y, x),
                    6 => (y, h - 1 - x),
                    7 => (w - 1 - y, h - 1 - x),
                    _ => (w - 1 - y, x),
                };
                out[y * ow + x] = get(sx, sy);
            }
        }
        (out, ow, oh)
    }

    /// The pixels to store so that a decoder honouring `o` shows `upright`.
    fn stored_for(o: u16) -> (Vec<[u8; 3]>, usize, usize) {
        // The inverse of `orient`: the rotations undo each other, the rest are their own inverse.
        let inverse = match o {
            6 => 8,
            8 => 6,
            other => other,
        };
        orient(&upright(), 7, 5, inverse)
    }

    fn rgb_image(w: u32, h: u32, pixels: &[[u8; 3]]) -> Image {
        Image::from_u8(w, h, ChannelLayout::Rgb, pixels.iter().flatten().copied().collect()).unwrap()
    }

    fn rgb_pixels(image: &Rgb8) -> Vec<[u8; 3]> {
        image.data().to_vec()
    }

    /// A PNG with an IHDR of the wanted size and nothing else: enough for the decoder's size
    /// checks to fire.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]);
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        out.extend(13u32.to_be_bytes());
        let mut chunk = b"IHDR".to_vec();
        chunk.extend(&ihdr);
        out.extend(&chunk);
        out.extend(crc32(&chunk).to_be_bytes());
        out
    }

    // ---- A1 ----

    #[test]
    fn the_eight_exif_orientations_give_the_same_upright_pixels() {
        for o in 1..=8u16 {
            let (stored, w, h) = stored_for(o);
            let image = rgb_image(w as u32, h as u32, &stored);
            let png = png_with(&image, Some(exif_block(o, None, None)));
            let loaded = load(&png).unwrap();
            assert_eq!(loaded.len(), 1);
            let rgb = &loaded[0].rgb;
            assert_eq!((rgb.width(), rgb.height()), (7, 5), "orientation {o}");
            assert_eq!(rgb_pixels(rgb), upright(), "orientation {o}");
        }
    }

    #[test]
    fn an_empty_file_is_an_error() {
        assert!(load(b"").is_err());
        assert!(load(b"not an image at all").is_err());
    }

    #[test]
    fn a_truncated_file_is_an_error() {
        let png = png_with(&rgb_image(7, 5, &upright()), None);
        assert!(load(&png[..png.len() / 2]).is_err());
        assert!(load(&png[..20]).is_err());
    }

    #[test]
    fn a_one_by_a_hundred_thousand_image_is_an_error() {
        assert!(load(&png_header(1, 100_000)).is_err());
    }

    #[test]
    fn a_seventy_thousand_square_header_is_an_error() {
        let started = std::time::Instant::now();
        assert!(load(&png_header(70_000, 70_000)).is_err());
        // Nothing was allocated or decoded for it.
        assert!(started.elapsed().as_secs() < 5);
        // 20 000 × 20 000 fits each side but not the 100 megapixels.
        assert!(load(&png_header(20_000, 20_000)).is_err());
    }

    #[test]
    fn the_focal_length_and_digital_zoom_are_read() {
        let focal = focal_input(Some(&exif_block(1, Some((3, 2)), Some(26))));
        assert_eq!(focal.f35_mm, Some(26.0));
        assert_eq!(focal.digital_zoom, Some(1.5));
        // A zero denominator gives no zoom, and no EXIF gives nothing.
        assert_eq!(focal_input(Some(&exif_block(1, Some((1, 0)), None))).digital_zoom, None);
        assert_eq!(focal_input(None), Default::default());
        assert_eq!(focal_input(Some(b"garbage")), Default::default());
        // And through `load`.
        let png = png_with(&rgb_image(7, 5, &upright()), Some(exif_block(1, Some((3, 2)), Some(26))));
        let loaded = load(&png).unwrap();
        assert_eq!(loaded[0].focal.f35_mm, Some(26.0));
        assert_eq!(loaded[0].focal.digital_zoom, Some(1.5));
        assert_eq!(loaded[0].origin, ImportOrigin::File);
    }

    #[test]
    fn the_source_id_is_the_sha256_of_the_bytes() {
        assert_eq!(source_id(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(source_id(b"").len(), 64);
    }

    /// A TIFF of uncompressed 8-bit grey pages, each (width, height, pixels), little-endian.
    fn tiff_gray(pages: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![b'I', b'I', 42, 0, 0, 0, 0, 0];
        let mut previous_next = 4usize;
        for (w, h, data) in pages {
            let ifd_offset = out.len() as u32;
            out[previous_next..previous_next + 4].copy_from_slice(&ifd_offset.to_le_bytes());
            let strip_offset = ifd_offset + 2 + 9 * 12 + 4;
            let entries: [(u16, u16, u32); 9] = [
                (256, 4, *w),
                (257, 4, *h),
                (258, 3, 8),
                (259, 3, 1),
                (262, 3, 1),
                (273, 4, strip_offset),
                (277, 3, 1),
                (278, 4, *h),
                (279, 4, data.len() as u32),
            ];
            out.extend((entries.len() as u16).to_le_bytes());
            for (tag, kind, value) in entries {
                out.extend(tag.to_le_bytes());
                out.extend(kind.to_le_bytes());
                out.extend(1u32.to_le_bytes());
                out.extend(value.to_le_bytes());
            }
            previous_next = out.len();
            out.extend(0u32.to_le_bytes());
            out.extend(data);
        }
        out
    }

    #[test]
    fn a_multi_page_tiff_gives_one_image_per_page_in_order() {
        let pages: Vec<(u32, u32, Vec<u8>)> = (0..3u8).map(|p| (4, 3, (0..12).map(|i| p * 50 + i).collect())).collect();
        let loaded = load(&tiff_gray(&pages)).unwrap();
        assert_eq!(loaded.len(), 3);
        for (p, image) in loaded.iter().enumerate() {
            assert_eq!(image.origin, ImportOrigin::TiffPage { index: p as u32 });
            assert_eq!((image.rgb.width(), image.rgb.height()), (4, 3));
            assert_eq!(image.rgb.get(1, 0).unwrap()[0], p as u8 * 50 + 1);
        }
        // A single-page TIFF is one image from the file.
        let one = load(&tiff_gray(&pages[..1])).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].origin, ImportOrigin::File);
        assert_eq!(super::tiff_page_count(&tiff_gray(&pages)), 3);
        let many: Vec<(u32, u32, Vec<u8>)> = (0..MAX_PAGES + 3).map(|_| (2, 2, vec![9, 9, 9, 9])).collect();
        assert_eq!(load(&tiff_gray(&many)).unwrap().len(), MAX_PAGES);
    }

    // ---- A2 ----

    fn p3_png(pixels: &[[u8; 3]], w: u32, h: u32) -> Vec<u8> {
        let image = rgb_image(w, h, pixels).with_icc(Some(Builtin::DisplayP3.profile().to_bytes().to_vec()));
        craft_image_codecs::encode(&image, Format::Png, &EncodeOptions { embed_icc: true, ..EncodeOptions::default() }).unwrap()
    }

    fn eval_to_srgb(from: Builtin, rgb: [u8; 3]) -> [u8; 3] {
        let transform = Transform::new(from.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        let input = rgb.map(|v| f32::from(v) / 255.0);
        let mut out = [0f32; 3];
        transform.eval(&input, &mut out).unwrap();
        out.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    #[test]
    fn a_display_p3_grey_card_stays_neutral() {
        let greys: Vec<[u8; 3]> = (0..16u32).map(|i| [(i * 16 + 8) as u8; 3]).collect();
        let loaded = load(&p3_png(&greys, 4, 4)).unwrap();
        for p in loaded[0].rgb.data() {
            let spread = p.iter().max().unwrap() - p.iter().min().unwrap();
            assert!(spread <= 1, "{p:?}");
        }
    }

    #[test]
    fn the_result_matches_the_single_colour_eval() {
        let colours: Vec<[u8; 3]> =
            vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [200, 120, 40], [30, 60, 220], [250, 250, 20], [90, 90, 90], [10, 200, 180]];
        let loaded = load(&p3_png(&colours, 4, 2)).unwrap();
        for (got, source) in loaded[0].rgb.data().iter().zip(&colours) {
            let expected = eval_to_srgb(Builtin::DisplayP3, *source);
            for c in 0..3 {
                assert!((i32::from(got[c]) - i32::from(expected[c])).abs() <= 1, "{source:?}: {got:?} vs {expected:?}");
            }
        }
    }

    #[test]
    fn an_image_without_colour_information_is_treated_as_srgb() {
        let pixels = upright();
        let png = png_with(&rgb_image(7, 5, &pixels), None);
        assert_eq!(rgb_pixels(&load(&png).unwrap()[0].rgb), pixels);
        // An embedded sRGB profile changes nothing either.
        let image = rgb_image(7, 5, &pixels).with_icc(Some(Builtin::Srgb.profile().to_bytes().to_vec()));
        assert_eq!(rgb_pixels(&to_srgb(&image).unwrap()), pixels);
        // A profile that does not parse is not a broken file.
        let broken = rgb_image(7, 5, &pixels).with_icc(Some(vec![1, 2, 3, 4]));
        assert_eq!(rgb_pixels(&to_srgb(&broken).unwrap()), pixels);
    }

    #[test]
    fn a_heif_nclx_without_a_profile_uses_the_matching_builtin() {
        use craft_image_codecs::Cicp;
        let sources: Vec<[u8; 3]> = vec![[255, 0, 0], [20, 200, 90], [10, 10, 255], [128, 128, 128]];
        for (primaries, builtin) in [(12u16, Some(Builtin::DisplayP3)), (9, Some(Builtin::Rec2020)), (1, None)] {
            let mut image = rgb_image(4, 1, &sources);
            image.cicp = Some(Cicp { primaries, transfer: if primaries == 9 { 1 } else { 13 }, matrix: 0, full_range: true });
            let out = to_srgb(&image).unwrap();
            for (got, source) in out.data().iter().zip(&sources) {
                let expected = builtin.map_or(*source, |b| eval_to_srgb(b, *source));
                for c in 0..3 {
                    assert!((i32::from(got[c]) - i32::from(expected[c])).abs() <= 1, "primaries {primaries}: {source:?}: {got:?} vs {expected:?}");
                }
            }
        }
        // A description we have no profile for is taken as sRGB.
        let mut odd = rgb_image(4, 1, &sources);
        odd.cicp = Some(Cicp { primaries: 7, transfer: 13, matrix: 0, full_range: true });
        assert_eq!(rgb_pixels(&to_srgb(&odd).unwrap()), sources);
    }

    #[test]
    fn a_sixteen_bit_source_converts_at_full_depth() {
        // 16-bit P3 values: the result matches the exact evaluation of the 16-bit colour, rounded.
        let samples: Vec<u16> = vec![65535, 0, 0, 0, 65535, 0, 12345, 54321, 33333, 40000, 40000, 40001];
        let image = Image::from_u16(2, 2, ChannelLayout::Rgb, &samples).unwrap().with_icc(Some(Builtin::DisplayP3.profile().to_bytes().to_vec()));
        let out = to_srgb(&image).unwrap();
        let transform = Transform::new(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
        for (i, got) in out.data().iter().enumerate() {
            let input: Vec<f32> = samples[i * 3..i * 3 + 3].iter().map(|&v| f32::from(v) / 65535.0).collect();
            let mut exact = [0f32; 3];
            transform.eval(&input, &mut exact).unwrap();
            for c in 0..3 {
                let want = (exact[c].clamp(0.0, 1.0) * 255.0).round() as i32;
                assert!((i32::from(got[c]) - want).abs() <= 1, "pixel {i} channel {c}: {} vs {want}", got[c]);
            }
        }
        // With no profile, 16 bits are rounded, not truncated.
        let plain = Image::from_u16(1, 1, ChannelLayout::Rgb, &[129, 65535, 32896]).unwrap();
        assert_eq!(to_srgb(&plain).unwrap().data()[0], [1, 255, 128]);
        assert_eq!((round_16_to_8(0), round_16_to_8(128), round_16_to_8(129), round_16_to_8(65535)), (0, 0, 1, 255));
    }

    #[test]
    fn transparency_is_composited_over_white() {
        let rgba: Vec<u8> = vec![10, 20, 30, 255, 10, 20, 30, 0, 100, 100, 100, 128, 0, 0, 0, 64];
        let image = Image::from_u8(4, 1, ChannelLayout::Rgba, rgba).unwrap();
        let out = to_srgb(&image).unwrap();
        let px = out.data();
        assert_eq!(px[0], [10, 20, 30]);
        assert_eq!(px[1], [255, 255, 255]);
        // c′ = a·c + (1 − a) with a = 128/255.
        assert_eq!(px[2], [177, 177, 177]);
        assert_eq!(px[3], [191, 191, 191]);
        // 16-bit alpha keeps its depth, and grey with alpha works.
        let wide = Image::from_u16(2, 1, ChannelLayout::Rgba, &[0, 0, 0, 0, 65535, 0, 0, 65535]).unwrap();
        assert_eq!(to_srgb(&wide).unwrap().data(), &[[255, 255, 255], [255, 0, 0]]);
        let grey = Image::from_u8(2, 1, ChannelLayout::GrayA, vec![40, 255, 40, 0]).unwrap();
        assert_eq!(to_srgb(&grey).unwrap().data(), &[[40, 40, 40], [255, 255, 255]]);
    }

    #[test]
    fn one_and_eight_threads_agree() {
        let image = Image::from_u16(
            97,
            61,
            ChannelLayout::Rgba,
            &(0..97 * 61 * 4).map(|i: u32| (i.wrapping_mul(2654435761) >> 16) as u16).collect::<Vec<u16>>(),
        )
        .unwrap()
        .with_icc(Some(Builtin::DisplayP3.profile().to_bytes().to_vec()));
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| to_srgb(&image).unwrap());
        assert_eq!(run(1), run(8));
    }

    /// The shared transform store is keyed by a profile's bytes. A transform built from the
    /// exact in-memory built-in profile differs by a level from one built from the parsed bytes,
    /// so the pixels must not depend on which of the two reached the store first.
    #[test]
    fn the_pixels_do_not_depend_on_what_the_shared_store_holds() {
        use craft_image_codecs::Cicp;
        let samples: Vec<u16> = (0..100 * 100 * 3).map(|i: u32| (i.wrapping_mul(2654435761) >> 16) as u16).collect();
        let p3_bytes = Builtin::DisplayP3.profile().to_bytes().to_vec();
        // The reference: a transform built straight from the parsed bytes, never shared.
        let parsed = craft_cms::Profile::parse(&p3_bytes).unwrap();
        let reference = Transform::with_options(&parsed, Builtin::Srgb.profile(), super::transform_options()).unwrap();
        let mut expected = vec![0u16; samples.len()];
        reference.convert_u16(&samples, 3, &mut expected, 3, false).unwrap();
        let expected: Vec<u8> = expected.iter().map(|&v| super::round_16_to_8(v)).collect();
        // An nclx image fills the store first, then an ICC image of the same profile reads it.
        let mut by_nclx = Image::from_u16(100, 100, ChannelLayout::Rgb, &samples).unwrap();
        by_nclx.cicp = Some(Cicp { primaries: 12, transfer: 13, matrix: 0, full_range: true });
        let first = to_srgb(&by_nclx).unwrap();
        let by_icc = Image::from_u16(100, 100, ChannelLayout::Rgb, &samples).unwrap().with_icc(Some(p3_bytes));
        let second = to_srgb(&by_icc).unwrap();
        let flat = |image: &Rgb8| -> Vec<u8> { image.data().iter().flatten().copied().collect() };
        assert_eq!(flat(&second), expected);
        assert_eq!(flat(&first), expected);
    }
}
