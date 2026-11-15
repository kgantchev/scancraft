// Ported from PdfCraft (https://github.com/storytold/pdfcraft), commit
// c944f106dd63eb9a33c7348eed1f03548cbd6468: `PlacedWord::place`, `text_layer`, `ink_extent` and
// `num` from `crates/ocr/src/lib.rs`, and the idea of `TextPlan` from `crates/fonts/src/plan.rs`,
// simplified here: ScanCraft embeds no font, so every Type3 glyph is empty and advances a fixed
// half em.
//
// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option; see
// LICENSE-MIT, LICENSE-APACHE and NOTICE in this repository.

//! A21: the invisible text layer (specification A21, §5.3).
//!
//! Every recognised word is written as invisible text (rendering mode 3), stretched to its box, so
//! that selecting and searching highlight the right place on the page image. A word whose
//! characters all have a WinAnsi code is set in the standard `/Helvetica`; any other word is set
//! in a Type3 font with one code per character, empty glyphs and a ToUnicode map, so the page
//! needs no embedded font.

use crate::IoError;
use crate::winansi::{fits_win_ansi, helvetica_width, literal, win_ansi};
use scancraft_core::recipe::{TextLine, Word};
use std::collections::HashMap;

/// A page's text layer: the content stream `/OCR BMC q BT 3 Tr … ET Q EMC` and the fonts it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayer {
    pub content: Vec<u8>,
    /// Fonts by resource name; their objects are written by A23.
    pub fonts: Vec<LayerFont>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerFont {
    /// Standard Helvetica with WinAnsi encoding.
    Helvetica { name: String },
    /// A Type3 font with empty glyphs and a ToUnicode map.
    Type3 { name: String, to_unicode: Vec<(u8, char)> },
}

/// The resource name of the standard Helvetica.
pub const HELVETICA_NAME: &str = "PCHelv";
/// What every Type3 code advances, in em. The text layer sizes a word by it and A23 writes the
/// font's `/Widths` from it, so the two always agree.
pub const TYPE3_ADVANCE_EM: f64 = 0.5;
/// The most codes one Type3 font holds: a code is one byte, and 0 is not used.
pub const MAX_CODES_PER_FONT: usize = 255;
/// The most distinct characters one page's Type3 fonts hold; hostile text beyond it becomes `?`.
const MAX_TYPE3_CHARS: usize = 2400;
/// The most words and the most characters in one word a page's layer takes.
const MAX_WORDS: usize = 100_000;
const MAX_WORD_CHARS: usize = 4096;

/// Helvetica's ascender and descender (em fractions): a word box spans about this much.
const BOX_EM: f64 = 0.93;
const DESCENT: f64 = 0.21;
/// Helvetica's cap height and x-height (em fractions).
const CAP_HEIGHT: f64 = 0.718;
const X_HEIGHT: f64 = 0.523;

/// A word placed on a page, in PDF user space: the bottom-left corner of its box, and the vectors
/// along its bottom edge (`across`, left → right as read) and its left edge (`up`).
#[derive(Clone, Debug, PartialEq)]
struct PlacedWord {
    text: String,
    origin: [f64; 2],
    across: [f64; 2],
    up: [f64; 2],
}

impl PlacedWord {
    /// Places a pixel-space word with `to_user`, which maps an image pixel (x right, y down) to
    /// user space. `rect` is `[left, top, right, bottom]`.
    fn place(word: &Word, to_user: impl Fn(f32, f32) -> [f64; 2]) -> PlacedWord {
        let [l, t, r, b] = word.rect;
        let o = to_user(l, b);
        let br = to_user(r, b);
        let tl = to_user(l, t);
        PlacedWord { text: word.text.clone(), origin: o, across: [br[0] - o[0], br[1] - o[1]], up: [tl[0] - o[0], tl[1] - o[1]] }
    }
}

/// How far `text`'s ink reaches above and below the baseline, in em fractions: a word box is
/// tight around the ink, so "DT" spans the cap height only, "sun" the x-height, and "gap" reaches
/// down to the descender. Text without letters or digits keeps the full ascender-descender span.
fn ink_extent(text: &str) -> (f64, f64) {
    if !text.chars().any(char::is_alphanumeric) {
        return (BOX_EM - DESCENT, -DESCENT);
    }
    let tall = text
        .chars()
        .any(|c| c.is_uppercase() || c.is_ascii_digit() || "bdfhijklt!?/\\()[]{}|'\"#$%&@".contains(c) || (c.is_alphabetic() && !c.is_ascii()));
    let low = text.chars().any(|c| "gjpqy()[]{}|_".contains(c));
    (if tall { CAP_HEIGHT } else { X_HEIGHT }, if low { -DESCENT } else { 0.0 })
}

/// A number for a content stream: four decimals, trailing zeros trimmed, `-0` written as `0`.
pub(crate) fn num(v: f64) -> String {
    let s = format!("{:.4}", if v.abs() < 1e-9 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The Type3 fonts of a page: one code per distinct character, in first-use order, 255 to a font.
#[derive(Default)]
struct Plan {
    /// The characters of each font, by code − 1.
    fonts: Vec<Vec<char>>,
    index: HashMap<char, (usize, u8)>,
}

impl Plan {
    /// The font and code of `ch`, planning it if it is new; `None` when the page's Type3 fonts
    /// are full.
    fn code(&mut self, ch: char) -> Option<(usize, u8)> {
        if let Some(&found) = self.index.get(&ch) {
            return Some(found);
        }
        // The last slot is kept for `?`, which hostile text beyond the limit becomes.
        let planned: usize = self.fonts.iter().map(Vec::len).sum();
        if planned >= MAX_TYPE3_CHARS - usize::from(ch != '?') {
            return None;
        }
        if self.fonts.last().is_none_or(|font| font.len() >= MAX_CODES_PER_FONT) {
            self.fonts.push(Vec::new());
        }
        let index = self.fonts.len() - 1;
        let font = self.fonts.last_mut()?;
        font.push(ch);
        let found = (index, u8::try_from(font.len()).ok()?);
        self.index.insert(ch, found);
        Some(found)
    }

    /// The codes that show `text`, with `?` for what no longer fits.
    fn encode(&mut self, text: &str) -> Vec<(usize, u8)> {
        text.chars().filter_map(|ch| self.code(ch).or_else(|| self.code('?'))).collect()
    }
}

/// One run of a word's characters in one Type3 font: the font's index, the codes to show, and the
/// run's start as an offset in em from the word's origin.
struct Run {
    font: usize,
    codes: Vec<u8>,
    start: f64,
}

fn runs(encoded: &[(usize, u8)]) -> (Vec<Run>, f64) {
    let mut runs: Vec<Run> = Vec::new();
    let mut total = 0.0;
    for &(font, code) in encoded {
        match runs.last_mut() {
            Some(last) if last.font == font => last.codes.push(code),
            _ => runs.push(Run { font, codes: vec![code], start: total }),
        }
        total += TYPE3_ADVANCE_EM;
    }
    (runs, total)
}

fn type3_name(font: usize) -> String {
    if font == 0 { "PCOcr".to_string() } else { format!("PCOcr{font}") }
}

fn matrix(m: [f64; 6]) -> String {
    m.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// Builds the layer for words in page pixels on a page of `page_px` pixels and `page_pt` points.
///
/// `page_px` and `page_pt` describe the page image, not the MediaBox: when the image is fitted
/// and centred on a larger page, A23 shifts the layer by the image's offset.
pub fn text_layer(lines: &[TextLine], page_px: (u32, u32), page_pt: (f64, f64)) -> Result<TextLayer, IoError> {
    let (w_px, h_px) = (f64::from(page_px.0), f64::from(page_px.1));
    let usable = |v: f64| v.is_finite() && v > 0.0;
    if !usable(w_px) || !usable(h_px) || !usable(page_pt.0) || !usable(page_pt.1) {
        return Err(IoError::Unsupported("the page has no usable size".to_string()));
    }
    // A21 step 1: pixels to points, with y flipped.
    let to_user = |x: f32, y: f32| [f64::from(x) * page_pt.0 / w_px, page_pt.1 - f64::from(y) * page_pt.1 / h_px];

    let mut plan = Plan::default();
    let mut used_fonts: Vec<usize> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    let mut current = HELVETICA_NAME.to_string();
    let words = lines.iter().flat_map(|line| &line.words).take(MAX_WORDS);
    for word in words {
        if word.text.is_empty() || word.text.chars().count() > MAX_WORD_CHARS {
            continue;
        }
        let w = PlacedWord::place(word, to_user);
        let type3 = !fits_win_ansi(&w.text);
        let planned = type3.then(|| runs(&plan.encode(&w.text)));
        let width = planned.as_ref().map_or_else(|| helvetica_width(&w.text, 1.0), |(_, total)| *total);
        let height = w.up[0].hypot(w.up[1]);
        let across = w.across[0].hypot(w.across[1]);
        // Only positive, finite geometry reaches the content stream: a zero-sized box cannot be
        // drawn, and a NaN or infinite one would put non-finite numbers on the page. (NaN fails
        // every comparison, so the positivity checks already refuse it; the finiteness checks keep
        // that property explicit.)
        if width <= 0.0 || !(height > 0.0 && height.is_finite()) || !(across > 0.0 && across.is_finite()) || !w.origin.iter().all(|v| v.is_finite()) {
            continue;
        }
        // The em square. Helvetica words: the word's own ink (cap height or x-height, descenders
        // or not) spans the box, so the text is as tall as the scanned word. Type3 words: the
        // Helvetica letter heights do not describe the face, so the box spans the
        // ascender–descender height. Either way the width is stretched to fit the word.
        let (top, bottom) = if planned.is_some() { (BOX_EM - DESCENT, -DESCENT) } else { ink_extent(&w.text) };
        let em = [w.up[0] / (top - bottom), w.up[1] / (top - bottom)];
        let o = [w.origin[0] - em[0] * bottom, w.origin[1] - em[1] * bottom];
        match &planned {
            None => {
                let ax = [w.across[0] / width, w.across[1] / width];
                if current != HELVETICA_NAME {
                    body.extend_from_slice(format!("/{HELVETICA_NAME} 1 Tf ").as_bytes());
                    current = HELVETICA_NAME.to_string();
                }
                body.extend_from_slice(matrix([ax[0], ax[1], em[0], em[1], o[0], o[1]]).as_bytes());
                body.extend_from_slice(b" Tm ");
                body.extend(literal(&win_ansi(&w.text)));
                body.extend_from_slice(b" Tj\n");
            }
            Some((planned_runs, total)) => {
                for run in planned_runs {
                    let name = type3_name(run.font);
                    if current != name {
                        body.extend_from_slice(format!("/{name} 1 Tf ").as_bytes());
                        current = name;
                    }
                    if !used_fonts.contains(&run.font) {
                        used_fonts.push(run.font);
                    }
                    let m = [
                        w.across[0] / total,
                        w.across[1] / total,
                        em[0],
                        em[1],
                        o[0] + w.across[0] * run.start / total,
                        o[1] + w.across[1] * run.start / total,
                    ];
                    body.extend_from_slice(matrix(m).as_bytes());
                    body.extend_from_slice(b" Tm ");
                    body.extend(literal(&run.codes));
                    body.extend_from_slice(b" Tj\n");
                }
            }
        }
    }
    if body.is_empty() {
        return Ok(TextLayer { content: Vec::new(), fonts: Vec::new() });
    }
    // `3 Tr` and the font are graphics state, which `ET` doesn't reset: without `q … Q` they would
    // stay set for every later content stream on the page, so text added after OCR drew invisibly.
    let mut content = format!("/OCR BMC\nq\nBT\n3 Tr\n/{HELVETICA_NAME} 1 Tf\n").into_bytes();
    content.extend(body);
    content.extend_from_slice(b"ET\nQ\nEMC\n");
    let mut fonts = vec![LayerFont::Helvetica { name: HELVETICA_NAME.to_string() }];
    used_fonts.sort_unstable();
    for font in used_fonts {
        let to_unicode = plan.fonts.get(font).map(|chars| chars.iter().enumerate().map(|(i, &c)| (i as u8 + 1, c)).collect()).unwrap_or_default();
        fonts.push(LayerFont::Type3 { name: type3_name(font), to_unicode });
    }
    Ok(TextLayer { content, fonts })
}

#[cfg(test)]
mod tests {
    use super::{LayerFont, MAX_CODES_PER_FONT, TYPE3_ADVANCE_EM, num, text_layer};
    use crate::winansi::helvetica_width;
    use scancraft_core::recipe::{TextLine, Word};

    fn word(text: &str, rect: [f32; 4]) -> Word {
        Word { text: text.to_string(), rect, confidence: 0.9, source: "test".to_string() }
    }

    fn line(words: Vec<Word>) -> TextLine {
        TextLine { words }
    }

    /// The numbers and string of each `… Tm (text) Tj` in a content stream.
    fn shown(content: &[u8]) -> Vec<([f64; 6], Vec<u8>)> {
        let text = String::from_utf8_lossy(content);
        let mut out = Vec::new();
        for l in text.lines() {
            let Some(tm) = l.find(" Tm ") else { continue };
            let numbers: Vec<f64> = l[..tm].rsplit(' ').take(6).map(|v| v.parse().unwrap()).collect::<Vec<_>>().into_iter().rev().collect();
            let rest = &l[tm + 4..];
            let open = rest.find('(').unwrap();
            let close = rest.rfind(") Tj").unwrap();
            let mut bytes = Vec::new();
            let mut it = rest.as_bytes()[open + 1..close].iter().copied();
            while let Some(b) = it.next() {
                bytes.push(if b == b'\\' { it.next().unwrap() } else { b });
            }
            out.push(([numbers[0], numbers[1], numbers[2], numbers[3], numbers[4], numbers[5]], bytes));
        }
        out
    }

    /// An A4 page at 300 DPI.
    const PX: (u32, u32) = (2480, 3508);
    const PT: (f64, f64) = (595.28, 841.89);

    #[test]
    fn a_win_ansi_word_uses_standard_helvetica() {
        let layer = text_layer(&[line(vec![word("Hello", [100.0, 200.0, 400.0, 260.0])])], PX, PT).unwrap();
        assert_eq!(layer.fonts, vec![LayerFont::Helvetica { name: "PCHelv".to_string() }]);
        let content = String::from_utf8(layer.content.clone()).unwrap();
        assert!(content.starts_with("/OCR BMC\nq\nBT\n3 Tr\n/PCHelv 1 Tf\n"), "{content}");
        assert!(content.ends_with("ET\nQ\nEMC\n"));
        assert_eq!(shown(&layer.content)[0].1, b"Hello");
    }

    #[test]
    fn a_word_outside_win_ansi_uses_a_type3_font() {
        let layer = text_layer(&[line(vec![word("Привет", [100.0, 200.0, 500.0, 260.0])])], PX, PT).unwrap();
        let LayerFont::Type3 { name, to_unicode } = &layer.fonts[1] else { panic!("{:?}", layer.fonts) };
        assert_eq!(name, "PCOcr");
        assert_eq!(layer.fonts.len(), 2);
        // The shown codes map back to the word through the ToUnicode map.
        let codes = &shown(&layer.content)[0].1;
        let back: String = codes.iter().map(|c| to_unicode.iter().find(|(k, _)| k == c).unwrap().1).collect();
        assert_eq!(back, "Привет");
        // Codes start at 1, are contiguous and sorted.
        assert!(to_unicode.iter().enumerate().all(|(i, (code, _))| usize::from(*code) == i + 1));
    }

    #[test]
    fn a_character_keeps_its_code_across_words_and_fonts_split_at_255() {
        // 300 distinct characters need two fonts; a repeated character keeps its code.
        let first: String = (0x4e00u32..0x4e00 + 300).filter_map(char::from_u32).collect();
        let layer = text_layer(&[line(vec![word(&first, [0.0, 0.0, 600.0, 40.0]), word("一", [0.0, 50.0, 40.0, 90.0])])], PX, PT).unwrap();
        let fonts: Vec<_> = layer
            .fonts
            .iter()
            .filter_map(|f| if let LayerFont::Type3 { name, to_unicode } = f { Some((name.clone(), to_unicode.len())) } else { None })
            .collect();
        assert_eq!(fonts, vec![("PCOcr".to_string(), MAX_CODES_PER_FONT), ("PCOcr1".to_string(), 45)]);
        // The word of 300 characters is two runs, one per font, and the repeated character reuses code 1.
        let all = shown(&layer.content);
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].1, vec![1]);
    }

    #[test]
    fn the_word_matrix_fits_the_box() {
        let rect = [100.0f32, 200.0, 400.0, 260.0];
        let layer = text_layer(&[line(vec![word("Hello", rect)])], PX, PT).unwrap();
        let (m, _) = shown(&layer.content).remove(0);
        let sx = PT.0 / f64::from(PX.0);
        let sy = PT.1 / f64::from(PX.1);
        // Across: the matrix times the natural width gives the box's width in points.
        let natural = helvetica_width("Hello", 1.0);
        assert!((m[0] * natural - f64::from(rect[2] - rect[0]) * sx).abs() < 1e-3);
        assert!(m[1].abs() < 1e-9);
        // Up: "Hello" has tall letters and no descender, so the em is the box height over the cap height.
        let height = f64::from(rect[3] - rect[1]) * sy;
        assert!((m[3] - height / 0.718).abs() < 1e-3, "{m:?}");
        // The origin is the box's bottom-left corner in user space, y flipped.
        assert!((m[4] - 100.0 * sx).abs() < 1e-3);
        assert!((m[5] - (PT.1 - 260.0 * sy)).abs() < 1e-3);
        // A Type3 word's across vector spans half an em per code.
        let layer = text_layer(&[line(vec![word("Привет", [100.0, 200.0, 400.0, 260.0])])], PX, PT).unwrap();
        let (m, codes) = shown(&layer.content).remove(0);
        assert!((m[0] * TYPE3_ADVANCE_EM * codes.len() as f64 - 300.0 * sx).abs() < 1e-3);
    }

    #[test]
    fn no_text_operator_sits_outside_a_q_q_pair() {
        let words = vec![word("One", [10.0, 10.0, 90.0, 40.0]), word("Два", [100.0, 10.0, 200.0, 40.0]), word("three", [210.0, 10.0, 330.0, 40.0])];
        let layer = text_layer(&[line(words)], PX, PT).unwrap();
        let text = String::from_utf8(layer.content).unwrap();
        let mut depth = 0i32;
        for token in text.split_whitespace() {
            match token {
                "q" => depth += 1,
                "Q" => depth -= 1,
                "Tr" | "Tf" | "Tm" => assert!(depth >= 1, "{token} outside q … Q"),
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
    }

    #[test]
    fn unusable_words_and_pages_are_skipped_or_refused() {
        let bad = vec![
            word("nan", [f32::NAN, 0.0, 10.0, 10.0]),
            word("inf", [0.0, 0.0, f32::INFINITY, 10.0]),
            word("flat", [10.0, 10.0, 50.0, 10.0]),
            word("thin", [10.0, 10.0, 10.0, 50.0]),
            word("", [0.0, 0.0, 10.0, 10.0]),
        ];
        let layer = text_layer(&[line(bad)], PX, PT).unwrap();
        assert!(layer.content.is_empty() && layer.fonts.is_empty());
        for (px, pt) in
            [((0, 100), (100.0, 100.0)), ((100, 100), (0.0, 100.0)), ((100, 100), (f64::NAN, 100.0)), ((100, 100), (100.0, f64::INFINITY))]
        {
            assert!(text_layer(&[], px, pt).is_err(), "{px:?} {pt:?}");
        }
        // A good word beside bad ones still comes out.
        let mixed = vec![word("bad", [f32::NAN, 0.0, 1.0, 1.0]), word("good", [10.0, 10.0, 90.0, 40.0])];
        assert_eq!(shown(&text_layer(&[line(mixed)], PX, PT).unwrap().content).len(), 1);
    }

    #[test]
    fn a_hostile_word_cannot_blow_up_the_plan() {
        // Thousands of distinct characters stay inside the code space, the font count and the time.
        let wild: String = (0x4e00u32..0x4e00 + 5000).filter_map(char::from_u32).collect();
        let layer = text_layer(&[line(vec![word(&wild, [0.0, 0.0, 2000.0, 40.0]), word("after", [0.0, 100.0, 200.0, 140.0])])], PX, PT).unwrap();
        let fonts: Vec<usize> =
            layer.fonts.iter().filter_map(|f| if let LayerFont::Type3 { to_unicode, .. } = f { Some(to_unicode.len()) } else { None }).collect();
        assert!(fonts.len() <= 10 && fonts.iter().all(|&n| n <= MAX_CODES_PER_FONT), "{fonts:?}");
        assert!(fonts.iter().sum::<usize>() <= 2400);
        // Later words are still written.
        assert!(shown(&layer.content).iter().any(|(_, bytes)| bytes == b"after"));
        // A word of control characters and surrogates-adjacent characters doesn't panic either.
        let nasty = "\u{0}\u{1}\u{7f}\u{200b}\u{fffd}\u{10ffff}(\\)\r\n";
        assert!(text_layer(&[line(vec![word(nasty, [0.0, 0.0, 100.0, 20.0])])], PX, PT).is_ok());
        // And so does a huge number of words.
        let many: Vec<Word> = (0..1000).map(|i| word("w", [i as f32, 0.0, i as f32 + 5.0, 10.0])).collect();
        assert_eq!(shown(&text_layer(&[line(many)], PX, PT).unwrap().content).len(), 1000);
    }

    #[test]
    fn numbers_are_written_without_noise() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.000_000_1), "0");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(595.28), "595.28");
        assert_eq!(num(-12.0), "-12");
        assert_eq!(num(0.333_333), "0.3333");
    }

    #[test]
    fn the_ink_extent_follows_the_letters() {
        use super::ink_extent;
        assert_eq!(ink_extent("DT"), (0.718, 0.0));
        assert_eq!(ink_extent("sun"), (0.523, 0.0));
        assert_eq!(ink_extent("gap"), (0.523, -0.21));
        assert_eq!(ink_extent("..."), (0.93 - 0.21, -0.21));
    }

    #[test]
    fn a_rotated_page_keeps_words_inside_it() {
        // A box taller than wide (a rotated word) still gets finite numbers.
        let layer = text_layer(&[line(vec![word("Up", [100.0, 100.0, 140.0, 400.0])])], PX, PT).unwrap();
        let (m, _) = shown(&layer.content).remove(0);
        assert!(m.iter().all(|v| v.is_finite()));
    }
}
