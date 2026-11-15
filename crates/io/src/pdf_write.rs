// The image pages and the Type3 font writer are ported from PdfCraft
// (https://github.com/storytold/pdfcraft), commit c944f106dd63eb9a33c7348eed1f03548cbd6468:
// `from_images` and `add_image` from `crates/create/src/lib.rs`, and `type3_font_dict` and
// `unicode_hex` from `crates/edit/src/text/fallback.rs`, rewritten here on `pdf-writer` with empty
// glyphs and a fixed advance.
//
// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option; see
// LICENSE-MIT, LICENSE-APACHE and NOTICE in this repository.

//! A23: PDF assembly with pdf-writer, written through craft-atomic.
//!
//! One page per entry, in order. Each page holds one image XObject whose bytes are A22's, written
//! unchanged, drawn by `q w 0 0 h x y cm /Im1 Do Q`, followed by the text layer of A21 as a second
//! content stream. The document information holds a title, a producer and the two dates, and
//! nothing from the photos' EXIF.

use crate::IoError;
use crate::encode::EncodedImage;
use crate::text_layer::{LayerFont, MAX_CODES_PER_FONT, TYPE3_ADVANCE_EM, TextLayer};
use chrono::{Datelike, Timelike, Utc};
use pdf_writer::types::FontFlags;
use pdf_writer::{Content, Date, Filter, Finish, Name, Pdf, Rect, Ref, TextStr};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// The largest page PDF viewers take, in points (200 inches).
const MAX_PAGE_PT: f64 = 14_400.0;
/// The most entries in one `bfchar` block (ISO 32000-1 §9.7.6.3).
const BFCHAR_BLOCK: usize = 100;

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

fn unsupported(why: &str) -> IoError {
    IoError::Unsupported(why.to_string())
}

fn fs(e: impl std::fmt::Display) -> IoError {
    IoError::Fs(e.to_string())
}

/// Writes the PDF to a temporary file next to `dest`, flushes it, then renames it over `dest`.
pub fn write_pdf(pages: &[PdfPage], info: &DocInfo, dest: &Path) -> Result<(), IoError> {
    let bytes = build_pdf(pages, info)?;
    write_atomically(dest, &bytes, |file, bytes| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

/// Hands `bytes` to `sink` writing into a staged file next to `dest`; only when the sink succeeds
/// is the file renamed over `dest`. Anything that goes wrong leaves `dest` as it was, with no stray
/// file beside it.
fn write_atomically(dest: &Path, bytes: &[u8], sink: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>) -> Result<(), IoError> {
    craft_atomic::atomic_write_with(dest, |file| sink(file, bytes)).map_err(fs)
}

/// The PDF file's bytes.
pub fn build_pdf(pages: &[PdfPage], info: &DocInfo) -> Result<Vec<u8>, IoError> {
    if pages.is_empty() {
        return Err(unsupported("the document has no pages"));
    }
    let mut next = 1i32;
    let mut alloc = move || {
        let r = Ref::new(next);
        next += 1;
        r
    };
    let (catalog_id, tree_id, info_id) = (alloc(), alloc(), alloc());
    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(tree_id);
    write_info(&mut pdf, info_id, info);

    let mut page_ids = Vec::with_capacity(pages.len());
    // Helvetica is one object for the whole document; Type3 fonts are per page.
    let mut helvetica: Option<Ref> = None;
    for page in pages {
        let (w, h) = page.size_pt;
        let [x, y, iw, ih] = page.image_rect_pt;
        let finite = [w, h, x, y, iw, ih].iter().all(|v| v.is_finite());
        if !finite || w <= 0.0 || h <= 0.0 || w > MAX_PAGE_PT || h > MAX_PAGE_PT || iw <= 0.0 || ih <= 0.0 {
            return Err(unsupported("a page has no usable size"));
        }
        let (page_id, image_id, content_id) = (alloc(), alloc(), alloc());
        page_ids.push(page_id);
        write_image(&mut pdf, image_id, &page.image)?;

        // The image: filled to its rectangle, never stretched (the caller fitted the rectangle).
        let mut content = Content::new();
        content.save_state();
        content.transform([iw as f32, 0.0, 0.0, ih as f32, x as f32, y as f32]);
        content.x_object(Name(b"Im1"));
        content.restore_state();
        pdf.stream(content_id, &content.finish());

        // The text layer, in its own stream, shifted to the image's corner.
        let mut contents = vec![content_id];
        let mut fonts: Vec<(String, Ref)> = Vec::new();
        if let Some(layer) = page.text.as_ref().filter(|l| !l.content.is_empty()) {
            let text_id = alloc();
            let mut stream = Vec::with_capacity(layer.content.len() + 40);
            let shifted = x != 0.0 || y != 0.0;
            if shifted {
                stream.extend_from_slice(format!("q 1 0 0 1 {} {} cm\n", crate::text_layer::num(x), crate::text_layer::num(y)).as_bytes());
            }
            stream.extend_from_slice(&layer.content);
            if shifted {
                stream.extend_from_slice(b"Q\n");
            }
            pdf.stream(text_id, &stream);
            contents.push(text_id);
            for font in &layer.fonts {
                match font {
                    LayerFont::Helvetica { name } => {
                        let id = *helvetica.get_or_insert_with(|| {
                            let id = alloc();
                            let mut f = pdf.type1_font(id);
                            f.base_font(Name(b"Helvetica"));
                            f.encoding_predefined(Name(b"WinAnsiEncoding"));
                            id
                        });
                        fonts.push((name.clone(), id));
                    }
                    LayerFont::Type3 { name, to_unicode } => {
                        let id = write_type3(&mut pdf, &mut alloc, to_unicode)?;
                        fonts.push((name.clone(), id));
                    }
                }
            }
        }

        let mut p = pdf.page(page_id);
        p.media_box(Rect::new(0.0, 0.0, w as f32, h as f32));
        p.parent(tree_id);
        p.contents_array(contents);
        let mut resources = p.resources();
        resources.x_objects().pair(Name(b"Im1"), image_id);
        if !fonts.is_empty() {
            let mut dict = resources.fonts();
            for (name, id) in &fonts {
                dict.pair(Name(name.as_bytes()), *id);
            }
        }
        resources.finish();
        p.finish();
    }
    pdf.pages(tree_id).kids(page_ids).count(pages.len() as i32);
    Ok(pdf.finish())
}

/// The document information: the title, the producer and the two dates; nothing else.
fn write_info(pdf: &mut Pdf, id: Ref, info: &DocInfo) {
    let now = Utc::now();
    let date = || {
        Date::new(now.year().clamp(0, 9999) as u16)
            .month(now.month() as u8)
            .day(now.day() as u8)
            .hour(now.hour() as u8)
            .minute(now.minute() as u8)
            .second(now.second() as u8)
            .utc_offset_hour(0)
            .utc_offset_minute(0)
    };
    let mut d = pdf.document_info(id);
    d.title(TextStr(&info.title));
    d.producer(TextStr(&info.producer));
    d.creation_date(date());
    d.modified_date(date());
}

/// One image XObject holding A22's bytes unchanged.
fn write_image(pdf: &mut Pdf, id: Ref, image: &EncodedImage) -> Result<(), IoError> {
    let size = |w: u32, h: u32| -> Result<(i32, i32), IoError> {
        Ok((i32::try_from(w).map_err(|_| unsupported("an image is too wide"))?, i32::try_from(h).map_err(|_| unsupported("an image is too tall"))?))
    };
    match image {
        EncodedImage::Jpeg { bytes, width, height, channels } => {
            let (w, h) = size(*width, *height)?;
            let mut x = pdf.image_xobject(id, bytes);
            x.filter(Filter::DctDecode);
            x.width(w);
            x.height(h);
            match channels {
                1 => x.color_space().device_gray(),
                3 => x.color_space().device_rgb(),
                _ => return Err(unsupported("a JPEG must have one or three channels")),
            };
            x.bits_per_component(8);
        }
        EncodedImage::Jbig2 { bytes, width, height } | EncodedImage::Flate1Bit { bytes, width, height } => {
            let (w, h) = size(*width, *height)?;
            let jbig2 = matches!(image, EncodedImage::Jbig2 { .. });
            let mut x = pdf.image_xobject(id, bytes);
            x.filter(if jbig2 { Filter::Jbig2Decode } else { Filter::FlateDecode });
            x.width(w);
            x.height(h);
            // No /Decode: the default [0 1] makes 0 black, which is how the samples were made.
            x.color_space().device_gray();
            x.bits_per_component(1);
        }
    }
    Ok(())
}

/// A Type3 font with one empty glyph per code, a half-em advance and a ToUnicode map: the port of
/// PdfCraft's `type3_font_dict` for a layer that embeds no font.
fn write_type3(pdf: &mut Pdf, alloc: &mut impl FnMut() -> Ref, to_unicode: &[(u8, char)]) -> Result<Ref, IoError> {
    if to_unicode.is_empty() || to_unicode.len() > MAX_CODES_PER_FONT {
        return Err(unsupported("a text layer font has no usable codes"));
    }
    // Codes run from 1 and are contiguous: the font's glyph names and widths depend on it.
    if !to_unicode.iter().enumerate().all(|(i, (code, _))| usize::from(*code) == i + 1) {
        return Err(unsupported("a text layer font's codes are not 1, 2, 3, …"));
    }
    let font_id = alloc();
    let cmap_id = alloc();
    let descriptor_id = alloc();
    let mut procs: BTreeMap<String, Ref> = BTreeMap::new();
    let advance = (TYPE3_ADVANCE_EM * 1000.0).round();
    // Every glyph is empty: `wx wy d0` with the advance and no path, so rendering mode 3 has
    // nothing to draw and every viewer agrees on the width.
    let glyph = format!("{advance} 0 d0\n");
    for (code, _) in to_unicode {
        let id = alloc();
        pdf.stream(id, glyph.as_bytes());
        procs.insert(format!("g{code:02X}"), id);
    }
    let cmap = to_unicode_cmap(to_unicode);
    pdf.stream(cmap_id, &cmap);
    // The descriptor gives viewers and extraction tools the ascender and descender they size a
    // word's box by; without one, poppler assumes 0.95 and −0.35 em, a box 1.4 times too tall.
    let mut descriptor = pdf.font_descriptor(descriptor_id);
    descriptor.name(Name(b"PCFallback"));
    descriptor.flags(FontFlags::SYMBOLIC);
    descriptor.bbox(Rect::new(0.0, -210.0, 1000.0, 720.0));
    descriptor.italic_angle(0.0);
    descriptor.ascent(720.0);
    descriptor.descent(-210.0);
    descriptor.cap_height(720.0);
    descriptor.stem_v(80.0);
    descriptor.finish();

    let mut f = pdf.type3_font(font_id);
    f.name(Name(b"PCFallback"));
    // The glyphs are empty, so the box is the ascender–descender span the text layer fits to a
    // word's box (PdfCraft's [0 -300 1000 1000] holds real outlines); viewers take a word's
    // height from it.
    f.bbox(Rect::new(0.0, -210.0, 1000.0, 720.0));
    f.matrix([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
    f.first_char(1);
    f.last_char(u8::try_from(to_unicode.len()).map_err(|_| unsupported("too many codes"))?);
    f.widths(std::iter::repeat_n(advance as f32, to_unicode.len()));
    f.encoding_custom().differences().consecutive(1, procs.keys().map(|n| Name(n.as_bytes())));
    {
        let mut dict = f.char_procs();
        for (name, id) in &procs {
            dict.pair(Name(name.as_bytes()), *id);
        }
    }
    f.font_descriptor(descriptor_id);
    f.to_unicode(cmap_id);
    f.finish();
    Ok(font_id)
}

/// The UTF-16BE hex of a character: four digits, or eight for a surrogate pair.
fn unicode_hex(ch: char) -> String {
    let mut units = [0u16; 2];
    ch.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect()
}

/// The ToUnicode CMap of a Type3 font: code → character, in `bfchar` blocks of at most 100.
fn to_unicode_cmap(to_unicode: &[(u8, char)]) -> Vec<u8> {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
    );
    for block in to_unicode.chunks(BFCHAR_BLOCK) {
        cmap.push_str(&format!("{} beginbfchar\n", block.len()));
        for (code, ch) in block {
            cmap.push_str(&format!("<{code:02X}> <{}>\n", unicode_hex(*ch)));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{DocInfo, PdfPage, build_pdf, write_atomically, write_pdf};
    use crate::IoError;
    use crate::encode::{EncodedImage, PageImage, encode};
    use crate::text_layer::text_layer;
    use scancraft_core::image::{BitPlane, Plane, Rgb8};
    use scancraft_core::recipe::{Quality, TextLine, Word};
    use std::path::{Path, PathBuf};

    fn info() -> DocInfo {
        DocInfo { title: "Quarterly report".to_string(), producer: "ScanCraft 0.1.0 (algorithms 1)".to_string() }
    }

    /// A smooth colour picture of `w × h` pixels, so JPEG's loss stays small.
    fn picture(w: u32, h: u32) -> Rgb8 {
        let data = (0..w * h).map(|i| [((i % w) * 255 / w) as u8, ((i / w) * 255 / h) as u8, 128]).collect();
        Plane::from_vec(w, h, data).unwrap()
    }

    fn jpeg_page(w: u32, h: u32, size_pt: (f64, f64), text: Option<crate::text_layer::TextLayer>) -> PdfPage {
        PdfPage {
            image: encode(&PageImage::Colour(picture(w, h)), Quality::Balanced).unwrap(),
            size_pt,
            image_rect_pt: [0.0, 0.0, size_pt.0, size_pt.1],
            text,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scancraft-io-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tool outside the repository, or `None` (the test then says why it skipped).
    fn tool(name: &str, candidates: &[&str]) -> Option<String> {
        let found = candidates.iter().find(|p| Path::new(p).exists()).map(|p| (*p).to_string());
        found.or_else(|| std::process::Command::new(name).arg("--version").output().ok().map(|_| name.to_string()))
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> usize {
        haystack.windows(needle.len()).filter(|w| *w == needle).count()
    }

    /// The page rendered by craft-pdf's rasteriser at exactly `w × h` pixels.
    fn render_exact(pdf: &[u8], index: usize, w: u32, h: u32) -> Vec<u8> {
        use craft_pdf::interpret::InterpreterSettings;
        use craft_pdf::raster::{Options, RenderCache};
        let doc = craft_pdf::open(std::sync::Arc::new(pdf.to_vec()), None).unwrap();
        let page = &doc.pages()[index];
        let (wp, hp) = page.render_dimensions();
        let options = Options { scale: [w as f32 / wp, h as f32 / hp], size: Some([w, h]), ..Options::default() };
        let raster = craft_pdf::raster::render(page, &RenderCache::new(), &InterpreterSettings::default(), &options).unwrap();
        assert_eq!((raster.width, raster.height), (w, h));
        // Premultiplied RGBA over white, as A24 does.
        raster.bytes().as_chunks::<4>().0.iter().flat_map(|p| [0, 1, 2].map(|c| (u16::from(p[c]) + 255 - u16::from(p[3])).min(255) as u8)).collect()
    }

    // ---- structure ----

    #[test]
    fn an_empty_document_and_unusable_sizes_are_refused() {
        assert!(matches!(build_pdf(&[], &info()), Err(IoError::Unsupported(_))));
        for bad in [(0.0, 100.0), (100.0, -1.0), (f64::NAN, 100.0), (100.0, f64::INFINITY), (20_000.0, 100.0)] {
            let mut page = jpeg_page(16, 16, (100.0, 100.0), None);
            page.size_pt = bad;
            assert!(build_pdf(&[page], &info()).is_err(), "{bad:?}");
        }
        let mut page = jpeg_page(16, 16, (100.0, 100.0), None);
        page.image_rect_pt = [0.0, 0.0, 0.0, 50.0];
        assert!(build_pdf(&[page], &info()).is_err());
    }

    #[test]
    fn the_document_information_holds_the_title_and_producer() {
        let bytes = build_pdf(&[jpeg_page(32, 32, (100.0, 100.0), None)], &info()).unwrap();
        let doc = craft_pdf::open(std::sync::Arc::new(bytes), None).unwrap();
        let meta = doc.metadata();
        assert_eq!(meta.title.as_deref(), Some(&b"Quarterly report"[..]));
        assert_eq!(meta.producer.as_deref(), Some(&b"ScanCraft 0.1.0 (algorithms 1)"[..]));
        let (created, modified) = (meta.creation_date.unwrap(), meta.modification_date.unwrap());
        assert!(created.year >= 2026 && modified.year >= 2026);
        assert_eq!((created.year, created.month, created.day), (modified.year, modified.month, modified.day));
    }

    #[test]
    fn nothing_from_the_exif_reaches_the_pdf() {
        let bytes = build_pdf(&[jpeg_page(32, 32, (100.0, 100.0), None)], &info()).unwrap();
        for needle in [&b"Exif"[..], b"/Author", b"/Keywords", b"/Subject", b"xpacket", b"<x:xmpmeta", b"/Metadata", b"ICC_PROFILE", b"/GPS"] {
            assert_eq!(contains(&bytes, needle), 0, "{}", String::from_utf8_lossy(needle));
        }
    }

    #[test]
    fn the_media_box_is_the_physical_size() {
        let bytes = build_pdf(&[jpeg_page(32, 48, (595.28, 841.89), None), jpeg_page(32, 32, (100.0, 50.0), None)], &info()).unwrap();
        let doc = craft_pdf::open(std::sync::Arc::new(bytes), None).unwrap();
        let sizes: Vec<(f32, f32)> = doc.pages().iter().map(|p| p.base_dimensions()).collect();
        assert!((sizes[0].0 - 595.28).abs() < 1e-3 && (sizes[0].1 - 841.89).abs() < 1e-3, "{sizes:?}");
        assert_eq!(sizes[1], (100.0, 50.0));
    }

    #[test]
    fn the_image_bytes_are_written_unchanged_into_the_pdf() {
        let jpeg = jpeg_page(40, 30, (100.0, 75.0), None);
        let mut bits = BitPlane::new(64, 32).unwrap();
        for y in 8..24 {
            for x in 10..50 {
                bits.set(x, y, (x + y) % 3 != 0);
            }
        }
        let jbig2 = PdfPage {
            image: encode(&PageImage::Bits(bits), Quality::Small).unwrap(),
            size_pt: (64.0, 32.0),
            image_rect_pt: [0.0, 0.0, 64.0, 32.0],
            text: None,
        };
        let bytes = build_pdf(&[jpeg.clone(), jbig2.clone()], &info()).unwrap();
        for page in [&jpeg, &jbig2] {
            let (EncodedImage::Jpeg { bytes: b, .. } | EncodedImage::Jbig2 { bytes: b, .. } | EncodedImage::Flate1Bit { bytes: b, .. }) = &page.image;
            assert_eq!(contains(&bytes, b), 1);
        }
        assert_eq!(contains(&bytes, b"/Filter /DCTDecode"), 1);
        assert_eq!(contains(&bytes, b"/Filter /JBIG2Decode"), 1);
        // JBIG2 has no /Decode array: the default 0 = black is what the encoder produced.
        assert_eq!(contains(&bytes, b"/Decode"), 0);
        assert_eq!(contains(&bytes, b"/ColorSpace /DeviceRGB"), 1);
        assert_eq!(contains(&bytes, b"/ColorSpace /DeviceGray"), 1);
    }

    #[test]
    fn a_page_of_black_text_renders_black() {
        // Black ink on a white page: the bit polarity of A22 and of ISO 32000-2 §7.4.7 agree.
        let mut bits = BitPlane::new(64, 32).unwrap();
        for y in 8..24 {
            for x in 20..28 {
                bits.set(x, y, true);
            }
        }
        let page = PdfPage {
            image: encode(&PageImage::Bits(bits.clone()), Quality::Best).unwrap(),
            size_pt: (64.0, 32.0),
            image_rect_pt: [0.0, 0.0, 64.0, 32.0],
            text: None,
        };
        let bytes = build_pdf(&[page], &info()).unwrap();
        let rendered = render_exact(&bytes, 0, 64, 32);
        for y in 0..32u32 {
            for x in 0..64u32 {
                let want = if bits.get(x, y).unwrap() { 0u8 } else { 255 };
                let at = ((y * 64 + x) * 3) as usize;
                assert_eq!(&rendered[at..at + 3], &[want; 3], "({x}, {y})");
            }
        }
    }

    #[test]
    fn each_page_renders_as_its_encoded_image() {
        let page = jpeg_page(300, 200, (72.0, 48.0), None);
        let EncodedImage::Jpeg { bytes: jpeg, .. } = &page.image else { panic!("not a JPEG") };
        let decoded = craft_image_codecs::decode(jpeg).unwrap();
        let bytes = build_pdf(&[page], &info()).unwrap();
        let rendered = render_exact(&bytes, 0, 300, 200);
        let source = decoded.as_u8().unwrap();
        let mean = rendered.iter().zip(source).map(|(&a, &b)| f64::from(a.abs_diff(b))).sum::<f64>() / source.len() as f64;
        assert!(mean <= 1.0, "mean difference {mean}");
    }

    // ---- fitting ----

    #[test]
    fn a_fixed_page_size_fits_the_image_without_stretching() {
        // A 300 × 200 image (3 : 2) on a 200 × 200 pt page: fitted to the width, centred.
        let mut page = jpeg_page(300, 200, (200.0, 200.0), None);
        page.image_rect_pt = [0.0, 100.0 / 3.0, 200.0, 400.0 / 3.0];
        let bytes = build_pdf(&[page], &info()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("200 0 0 133.33333 0 33.333332 cm"), "{}", text.lines().filter(|l| l.contains(" cm")).collect::<Vec<_>>().join("|"));
        // The page's white shows above and below the picture.
        let rendered = render_exact(&bytes, 0, 200, 200);
        assert_eq!(&rendered[(10 * 200 + 100) * 3..(10 * 200 + 100) * 3 + 3], &[255, 255, 255]);
        assert_ne!(&rendered[(100 * 200 + 100) * 3..(100 * 200 + 100) * 3 + 3], &[255, 255, 255]);
    }

    #[test]
    fn the_text_layer_follows_the_image_when_it_is_fitted() {
        let words = vec![TextLine {
            words: vec![Word { text: "Hello".to_string(), rect: [30.0, 40.0, 150.0, 90.0], confidence: 0.9, source: "t".to_string() }],
        }];
        let layer = text_layer(&words, (300, 200), (200.0, 133.33333)).unwrap();
        let mut page = jpeg_page(300, 200, (200.0, 200.0), Some(layer));
        page.image_rect_pt = [0.0, 33.333332, 200.0, 133.33333];
        let bytes = build_pdf(&[page], &info()).unwrap();
        // The layer is shifted by the image's corner.
        assert_eq!(contains(&bytes, b"q 1 0 0 1 0 33.3333 cm\n/OCR BMC"), 1);
    }

    // ---- fonts and the layer ----

    fn words_page() -> (PdfPage, Vec<Word>) {
        let words: Vec<Word> = [
            ("Quarterly", [100.0, 100.0, 520.0, 160.0]),
            ("report", [560.0, 100.0, 900.0, 160.0]),
            ("Привет", [100.0, 220.0, 420.0, 280.0]),
            ("Mañana", [460.0, 220.0, 800.0, 280.0]),
        ]
        .iter()
        .map(|(t, r)| Word { text: (*t).to_string(), rect: *r, confidence: 0.9, source: "t".to_string() })
        .collect();
        let layer = text_layer(&[TextLine { words: words.clone() }], (1000, 400), (240.0, 96.0)).unwrap();
        (jpeg_page(1000, 400, (240.0, 96.0), Some(layer)), words)
    }

    #[test]
    fn the_page_renders_the_same_with_and_without_the_layer() {
        let (with, _) = words_page();
        let mut without = with.clone();
        without.text = None;
        let a = render_exact(&build_pdf(&[with], &info()).unwrap(), 0, 400, 160);
        let b = render_exact(&build_pdf(&[without], &info()).unwrap(), 0, 400, 160);
        assert_eq!(a, b);
    }

    #[test]
    fn the_fonts_are_a_standard_helvetica_and_a_type3_with_a_to_unicode_map() {
        let (page, _) = words_page();
        let bytes = build_pdf(&[page], &info()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/BaseFont /Helvetica") && text.contains("/Encoding /WinAnsiEncoding"));
        // Helvetica is a standard font: no descriptor and no embedded file. The Type3 font has the
        // one descriptor, for its ascender and descender, and still no file.
        assert_eq!(text.matches("/Type /FontDescriptor").count(), 1);
        assert!(!text.contains("/FontFile"));
        assert!(text.contains("/Subtype /Type3") && text.contains("/FirstChar 1") && text.contains("/ToUnicode"));
        assert!(text.contains("/Widths [500 500 500 500 500 500 500]") || text.contains("/Widths [500 500 500"), "widths");
        // The ToUnicode map has the Cyrillic letters as UTF-16BE (П is U+041F), as a plain stream.
        assert!(text.contains("beginbfchar") && text.contains("<041F>"));
        assert!(!text.contains("/Type /CMap"));
    }

    #[test]
    fn a_to_unicode_map_splits_into_blocks_of_a_hundred() {
        let to_unicode: Vec<(u8, char)> =
            (1..=255u8).map(|c| (c, char::from_u32(0x4e00 + u32::from(c)).unwrap())).chain(std::iter::empty()).collect();
        let cmap = String::from_utf8(super::to_unicode_cmap(&to_unicode)).unwrap();
        assert_eq!(cmap.matches("beginbfchar").count(), 3);
        for block in cmap.split("beginbfchar").skip(1) {
            assert!(block.lines().filter(|l| l.starts_with('<')).count() <= 100);
        }
        // A character above U+FFFF is a surrogate pair.
        let astral = String::from_utf8(super::to_unicode_cmap(&[(1, '\u{1F600}')])).unwrap();
        assert!(astral.contains("<01> <D83DDE00>"), "{astral}");
    }

    // ---- the file ----

    #[test]
    fn qpdf_check_reports_no_errors() {
        let Some(qpdf) = tool("qpdf", &["/opt/homebrew/bin/qpdf", "/usr/local/bin/qpdf", "/usr/bin/qpdf"]) else {
            eprintln!("skipped: qpdf is not installed here, so `qpdf --check` was not run");
            return;
        };
        let dir = temp_dir("qpdf");
        let (page, _) = words_page();
        let mut bits = BitPlane::new(200, 100).unwrap();
        for y in 20..80 {
            bits.set(50, y, true);
        }
        let second = PdfPage {
            image: encode(&PageImage::Bits(bits), Quality::Balanced).unwrap(),
            size_pt: (48.0, 24.0),
            image_rect_pt: [0.0, 0.0, 48.0, 24.0],
            text: None,
        };
        let path = dir.join("out.pdf");
        write_pdf(&[page, second], &info(), &path).unwrap();
        let out = std::process::Command::new(qpdf).arg("--check").arg(&path).output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{said}");
        assert!(said.contains("No syntax or stream encoding errors found"), "{said}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_extracted_text_holds_the_words_in_order() {
        let Some(pdftotext) = tool("pdftotext", &["/usr/local/bin/pdftotext", "/opt/homebrew/bin/pdftotext", "/usr/bin/pdftotext"]) else {
            eprintln!("skipped: pdftotext is not installed here, so the extraction test was not run");
            return;
        };
        let dir = temp_dir("pdftotext");
        let (page, words) = words_page();
        let (path, html) = (dir.join("words.pdf"), dir.join("words.html"));
        write_pdf(&[page], &info(), &path).unwrap();
        let status = std::process::Command::new(pdftotext).arg("-bbox").arg(&path).arg(&html).status().unwrap();
        assert!(status.success());
        let text = std::fs::read_to_string(&html).unwrap();
        // <word xMin=".." yMin=".." xMax=".." yMax="..">text</word>, in points, y down.
        let mut found: Vec<(String, [f64; 4])> = Vec::new();
        for part in text.split("<word ").skip(1) {
            let attr = |name: &str| -> f64 { part.split(&format!("{name}=\"")).nth(1).unwrap().split('"').next().unwrap().parse().unwrap() };
            let body = part.split('>').nth(1).unwrap().split('<').next().unwrap().to_string();
            found.push((body, [attr("xMin"), attr("yMin"), attr("xMax"), attr("yMax")]));
        }
        let scale = (240.0 / 1000.0, 96.0 / 400.0);
        let mut matched = 0;
        let mut cursor = 0;
        for word in &words {
            let want = [
                f64::from(word.rect[0]) * scale.0,
                f64::from(word.rect[1]) * scale.1,
                f64::from(word.rect[2]) * scale.0,
                f64::from(word.rect[3]) * scale.1,
            ];
            if let Some(at) = found[cursor..].iter().position(|(t, _)| *t == word.text) {
                let b = found[cursor + at].1;
                let (ix, iy) = ((want[2].min(b[2]) - want[0].max(b[0])).max(0.0), (want[3].min(b[3]) - want[1].max(b[1])).max(0.0));
                let inter = ix * iy;
                let union = (want[2] - want[0]) * (want[3] - want[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
                let iou = inter / union;
                eprintln!("{}: IoU {iou:.2}", word.text);
                if iou >= 0.8 {
                    matched += 1;
                }
                cursor += at + 1;
            }
        }
        assert_eq!(matched, words.len(), "found {found:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    // ---- writing ----

    #[test]
    fn an_export_killed_halfway_leaves_the_destination_untouched() {
        let dir = temp_dir("atomic");
        let path = dir.join("keep.pdf");
        write_pdf(&[jpeg_page(32, 32, (100.0, 100.0), None)], &info(), &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        // A write that fails after half the bytes.
        let bytes = build_pdf(&[jpeg_page(64, 64, (100.0, 100.0), None)], &info()).unwrap();
        let result = write_atomically(&path, &bytes, |file, bytes| {
            use std::io::Write;
            file.write_all(&bytes[..bytes.len() / 2])?;
            Err(std::io::Error::other("the process was killed"))
        });
        assert!(matches!(result, Err(IoError::Fs(_))));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // No stray file beside it.
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("keep.pdf")]);
        // A write to a folder that doesn't exist is an error, not a panic.
        assert!(write_pdf(&[jpeg_page(8, 8, (10.0, 10.0), None)], &info(), &dir.join("nope").join("x.pdf")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_export_replaces_the_old_one_whole() {
        let dir = temp_dir("replace");
        let path = dir.join("doc.pdf");
        write_pdf(&[jpeg_page(32, 32, (100.0, 100.0), None)], &info(), &path).unwrap();
        write_pdf(&[jpeg_page(32, 32, (50.0, 50.0), None), jpeg_page(32, 32, (50.0, 50.0), None)], &info(), &path).unwrap();
        let doc = craft_pdf::open(std::sync::Arc::new(std::fs::read(&path).unwrap()), None).unwrap();
        assert_eq!(doc.pages().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
