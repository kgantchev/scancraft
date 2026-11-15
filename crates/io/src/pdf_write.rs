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

    let mut f = pdf.type3_font(font_id);
    f.name(Name(b"PCFallback"));
    f.bbox(Rect::new(0.0, -300.0, 1000.0, 1000.0));
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
