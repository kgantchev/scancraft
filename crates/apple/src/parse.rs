//! The pure steps between Vision's answers and ScanCraft's types: coordinate conversion, word
//! splitting, score arithmetic and the proportional fallback (A8 step 2, A15 step 2, A20 step 4).
//!
//! Nothing here calls Apple frameworks. The live path and the tests that start from a recorded
//! Vision result go through the same functions, so what CI checks is what the app runs, and it
//! does not depend on what Vision returns on a given macOS release.

use scancraft_core::geom::{Point, Quad};
use scancraft_core::recipe::{TextLine, Word};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// `Word::source` for everything this crate produces.
pub(crate) const SOURCE: &str = "vision";

/// A quadrilateral in Vision's normalised coordinates: four (u, v) corners, origin at the bottom
/// left, each in [0, 1] for a box inside the image.
pub(crate) type NormQuad = [(f64, f64); 4];

/// One recognised line as Vision reported it, before any conversion to page pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VisionLine {
    /// The top candidate's string.
    pub text: String,
    /// The top candidate's confidence.
    pub confidence: f32,
    /// The observation's bounding box as (x, y, width, height), normalised, origin bottom left.
    pub line_box: [f64; 4],
    /// For each word `split_words(text)` yields, in order, the corners of the box Vision gave
    /// for it, or `None` when Vision gave no box. A list shorter than the word list counts as
    /// `None` for the rest.
    pub word_quads: Vec<Option<NormQuad>>,
}

/// The smallest text Vision should read, as a fraction of the image's height, for text that is
/// `min_px` tall on an image `height_px` tall. Never below 0.25%.
fn min_text_fraction(min_px: f32, height_px: u32) -> f32 {
    (min_px / height_px.max(1) as f32).max(0.0025)
}

/// A20 step 2: `max(0.0025, 12 / page height in px)`, so 6-point text at 300 DPI is still read.
pub(crate) fn a20_min_text_height(height_px: u32) -> f32 {
    min_text_fraction(12.0, height_px)
}

/// A15 reads a 1024 px page in Fast mode, where Vision skips text smaller than its minimum
/// (3% of the height unless set, which skips all body text on a page that size). 8 px reads the
/// 11 px text of a 9-point document on that page; 12 px, A20's value, reads none of it.
pub(crate) fn a15_min_text_height(height_px: u32) -> f32 {
    min_text_fraction(8.0, height_px)
}

/// What A20 sets: the caller's `RecognizeOptions::minimum_text_height` when it is a positive
/// number (at most 1), and A20's own rule for the page's height when it is 0.
pub(crate) fn text_height_for(requested: f32, height_px: u32) -> f32 {
    if requested.is_finite() && requested > 0.0 { requested.min(1.0) } else { a20_min_text_height(height_px) }
}

/// A8 step 2 and A20 step 4: a point from Vision's normalised coordinates (origin at the bottom
/// left) to image pixels, x = u · w and y = (1 − v) · h.
///
/// There is no half-pixel shift: §7.1 puts the image's extent at [0, w] × [0, h], which is the
/// extent Vision's normalised coordinates span.
pub(crate) fn vision_point_to_page(u: f64, v: f64, w: u32, h: u32) -> Point {
    Point::new(u * f64::from(w), (1.0 - v) * f64::from(h))
}

/// A normalised (x, y, width, height) rectangle to page pixels as (left, top, right, bottom).
pub(crate) fn vision_rect_to_page(rect: [f64; 4], w: u32, h: u32) -> [f32; 4] {
    let [x, y, rw, rh] = rect;
    let bottom_left = vision_point_to_page(x, y, w, h);
    let top_right = vision_point_to_page(x + rw, y + rh, w, h);
    [
        bottom_left.x.min(top_right.x) as f32,
        bottom_left.y.min(top_right.y) as f32,
        bottom_left.x.max(top_right.x) as f32,
        bottom_left.y.max(top_right.y) as f32,
    ]
}

/// The enclosing rectangle (left, top, right, bottom) of a quadrilateral in page pixels. `None`
/// when a corner is not finite.
pub(crate) fn vision_quad_to_rect(corners: NormQuad, w: u32, h: u32) -> Option<[f32; 4]> {
    let mut rect = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (u, v) in corners {
        let p = vision_point_to_page(u, v, w, h);
        if !p.is_finite() {
            return None;
        }
        rect = [rect[0].min(p.x), rect[1].min(p.y), rect[2].max(p.x), rect[3].max(p.y)];
    }
    Some(rect.map(|c| c as f32))
}

/// A8 step 2: the four corners of a rectangle observation, in canonical order (TL, TR, BR, BL), in
/// the pixels of the image Vision looked at. `None` when a corner is not finite or the
/// quadrilateral has no area, so A9 never sees a degenerate candidate.
pub(crate) fn corners_to_quad(corners: NormQuad, w: u32, h: u32) -> Option<Quad> {
    let points = corners.map(|(u, v)| vision_point_to_page(u, v, w, h));
    if !points.iter().all(|p| p.is_finite()) {
        return None;
    }
    let quad = Quad::canonical(points);
    // The shoelace formula on the ordered corners, so the answer does not depend on the order
    // Vision listed them in.
    let mut twice_area = 0.0;
    for (i, p) in quad.0.iter().enumerate() {
        let q = quad.0[(i + 1) % 4];
        twice_area += p.x * q.y - q.x * p.y;
    }
    if twice_area.abs() < 2.0 {
        return None;
    }
    Some(quad)
}

/// A word of a line: its text and where it sits in the line in UTF-16 units, which is what
/// `NSRange` counts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece<'a> {
    pub text: &'a str,
    pub utf16: Range<usize>,
}

/// A20 step 4: splits a line at Unicode word boundaries (UAX #29) and drops the pieces that are
/// only whitespace. Punctuation stays as its own word, so a comma still has a place in the
/// searchable text layer.
pub(crate) fn split_words(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut offset = 0usize;
    for piece in text.split_word_bounds() {
        // Offsets in UTF-16 units: one per BMP character, two per character outside it.
        let len: usize = piece.chars().map(char::len_utf16).sum();
        if !piece.chars().all(char::is_whitespace) {
            pieces.push(Piece { text: piece, utf16: offset..offset.saturating_add(len) });
        }
        offset = offset.saturating_add(len);
    }
    pieces
}

/// A15 step 2, for one rotation: the sum over results of confidence × character count, and the
/// number of characters. Each result is the top candidate's (confidence, character count). A
/// confidence that is not a finite number counts as zero.
pub(crate) fn score_rotation(results: &[(f32, u32)]) -> (f32, u32) {
    let mut score = 0.0f64;
    let mut chars = 0u32;
    for &(confidence, count) in results {
        let confidence = if confidence.is_finite() { f64::from(confidence) } else { 0.0 };
        score += confidence * f64::from(count);
        chars = chars.saturating_add(count);
    }
    (score as f32, chars)
}

/// The number of characters of a recognised string, counted as Unicode scalar values (A15).
pub(crate) fn char_count(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}

/// A20 step 4's fallback: when Vision gives no box for a word, the line's box (left, top, right,
/// bottom) is split in proportion to the words' UTF-16 lengths. Every box gets the line's full
/// height. No words, or lengths that add up to zero, give no boxes.
pub(crate) fn proportional_boxes(line_box: [f32; 4], lengths: &[usize]) -> Vec<[f32; 4]> {
    let total: usize = lengths.iter().fold(0usize, |sum, &n| sum.saturating_add(n));
    if total == 0 {
        return Vec::new();
    }
    let [left, top, right, bottom] = line_box;
    let width = f64::from(right) - f64::from(left);
    let mut boxes = Vec::with_capacity(lengths.len());
    let mut before = 0usize;
    for &n in lengths {
        let after = before.saturating_add(n);
        let from = f64::from(left) + width * before as f64 / total as f64;
        let to = f64::from(left) + width * after as f64 / total as f64;
        boxes.push([from as f32, top, to as f32, bottom]);
        before = after;
    }
    boxes
}

/// Builds the lines of the recognised text from what Vision reported, in the order it reported
/// them (which follows the layout's reading order). `w` and `h` are the page's size in pixels.
///
/// Every word gets Vision's own box when there is one, and a share of the line's box when there
/// is not. A word whose box cannot be made finite is left out, and so is a line left with no
/// words.
pub(crate) fn parse_lines(lines: &[VisionLine], w: u32, h: u32) -> Vec<TextLine> {
    lines.iter().filter_map(|line| parse_line(line, w, h)).collect()
}

fn parse_line(line: &VisionLine, w: u32, h: u32) -> Option<TextLine> {
    let pieces = split_words(&line.text);
    let lengths: Vec<usize> = pieces.iter().map(|p| p.utf16.len()).collect();
    let fallback = proportional_boxes(vision_rect_to_page(line.line_box, w, h), &lengths);
    let mut words = Vec::with_capacity(pieces.len());
    for (i, piece) in pieces.iter().enumerate() {
        let from_vision = line.word_quads.get(i).copied().flatten().and_then(|quad| vision_quad_to_rect(quad, w, h));
        let Some(rect) = from_vision.or_else(|| fallback.get(i).copied()) else { continue };
        if !rect.iter().all(|c| c.is_finite()) {
            continue;
        }
        words.push(Word { text: piece.text.to_owned(), rect: clamp_to_page(rect, w, h), confidence: line.confidence, source: SOURCE.to_owned() });
    }
    if words.is_empty() { None } else { Some(TextLine { words }) }
}

/// Keeps a box inside the page: Vision's boxes can reach a little past the edge.
fn clamp_to_page(rect: [f32; 4], w: u32, h: u32) -> [f32; 4] {
    let (w, h) = (w as f32, h as f32);
    [rect[0].clamp(0.0, w), rect[1].clamp(0.0, h), rect[2].clamp(0.0, w), rect[3].clamp(0.0, h)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn rect_near(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| near(*x, y))
    }

    #[test]
    fn min_text_heights_follow_the_page_height_and_never_drop_below_a_quarter_percent() {
        // A4 at 300 DPI is 3508 px tall: 12 px of it is 0.34%.
        assert!(near(a20_min_text_height(3508), 12.0 / 3508.0));
        // A tall poster-sized page would give less than 0.25%: held at 0.25%.
        assert_eq!(a20_min_text_height(10_000), 0.0025);
        assert!(near(a15_min_text_height(1024), 8.0 / 1024.0));
        assert!(a15_min_text_height(1024) < a20_min_text_height(1024));
        // A zero height does not divide by zero.
        assert!(a20_min_text_height(0).is_finite());
    }

    #[test]
    fn text_height_for_uses_the_option_when_it_is_positive_and_the_rule_when_it_is_zero() {
        assert_eq!(text_height_for(0.05, 3508), 0.05);
        assert_eq!(text_height_for(7.0, 3508), 1.0);
        for none in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(text_height_for(none, 3508), a20_min_text_height(3508));
        }
    }

    #[test]
    fn vision_point_to_page_flips_the_vertical_axis() {
        let p = vision_point_to_page(0.25, 0.75, 100, 200);
        assert_eq!((p.x, p.y), (25.0, 50.0));
        // The bottom-left corner of Vision's space is the bottom-left of the image.
        let p = vision_point_to_page(0.0, 0.0, 100, 200);
        assert_eq!((p.x, p.y), (0.0, 200.0));
    }

    #[test]
    fn vision_rect_to_page_gives_left_top_right_bottom() {
        let r = vision_rect_to_page([0.1, 0.2, 0.5, 0.3], 1000, 2000);
        assert!(rect_near(r, [100.0, 1000.0, 600.0, 1600.0]), "{r:?}");
    }

    #[test]
    fn vision_quad_to_rect_takes_the_enclosing_rectangle() {
        // A tilted word box: the enclosing rectangle spans every corner.
        let quad = [(0.10, 0.50), (0.30, 0.52), (0.31, 0.46), (0.11, 0.44)];
        let r = vision_quad_to_rect(quad, 1000, 1000).unwrap();
        assert!(rect_near(r, [100.0, 480.0, 310.0, 560.0]), "{r:?}");
        assert_eq!(vision_quad_to_rect([(f64::NAN, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)], 10, 10), None);
    }

    #[test]
    fn corners_to_quad_puts_the_corners_in_canonical_order() {
        // Vision's topLeft, topRight, bottomRight, bottomLeft for an upright page.
        let upright = [(0.2, 0.8), (0.8, 0.8), (0.8, 0.2), (0.2, 0.2)];
        let q = corners_to_quad(upright, 1000, 500).unwrap();
        for (corner, want) in [(q.tl(), (200.0, 100.0)), (q.tr(), (800.0, 100.0)), (q.br(), (800.0, 400.0)), (q.bl(), (200.0, 400.0))] {
            assert!((corner.x - want.0).abs() < 1e-9 && (corner.y - want.1).abs() < 1e-9, "{corner:?} vs {want:?}");
        }

        // The same corners in any other order give the same quad, and a page seen at an angle
        // still starts at its top-left corner and runs clockwise on screen.
        let tilted = [(0.30, 0.85), (0.80, 0.70), (0.70, 0.15), (0.20, 0.30)];
        let a = corners_to_quad(tilted, 1000, 1000).unwrap();
        let b = corners_to_quad([tilted[2], tilted[0], tilted[3], tilted[1]], 1000, 1000).unwrap();
        assert_eq!(a, b);
        assert!(a.tl().x < a.tr().x && a.bl().x < a.br().x);
        assert!(a.tl().y < a.bl().y && a.tr().y < a.br().y);
    }

    #[test]
    fn corners_to_quad_rejects_a_degenerate_result() {
        assert_eq!(corners_to_quad([(0.5, 0.5); 4], 1000, 1000), None);
        assert_eq!(corners_to_quad([(0.1, 0.1), (0.5, 0.5), (0.9, 0.9), (0.3, 0.3)], 1000, 1000), None);
        assert_eq!(corners_to_quad([(f64::INFINITY, 0.0), (0.1, 0.9), (0.9, 0.9), (0.9, 0.1)], 1000, 1000), None);
    }

    fn texts(text: &str) -> Vec<&str> {
        split_words(text).into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn split_words_drops_whitespace_and_keeps_punctuation() {
        assert_eq!(texts("Brr, it's 29.3°F!"), ["Brr", ",", "it's", "29.3", "°", "F", "!"]);
        assert_eq!(texts("  two   spaces\tand a tab "), ["two", "spaces", "and", "a", "tab"]);
    }

    #[test]
    fn split_words_reports_utf16_ranges() {
        // "é" is one UTF-16 unit (two bytes), "😀" is two units (four bytes).
        let pieces = split_words("é 😀 ab");
        let got: Vec<(&str, Range<usize>)> = pieces.iter().map(|p| (p.text, p.utf16.clone())).collect();
        assert_eq!(got, [("é", 0..1), ("😀", 2..4), ("ab", 5..7)]);
        // Each range picks the same text out of the UTF-16 form of the line.
        let units: Vec<u16> = "é 😀 ab".encode_utf16().collect();
        for p in &pieces {
            assert_eq!(String::from_utf16(&units[p.utf16.clone()]).unwrap(), p.text);
        }
    }

    #[test]
    fn split_words_of_an_empty_line_is_empty() {
        assert!(split_words("").is_empty());
        assert!(split_words(" \t\n ").is_empty());
    }

    #[test]
    fn score_rotation_sums_confidence_times_characters() {
        let (score, chars) = score_rotation(&[(0.9, 5), (0.5, 10)]);
        assert!(near(score, 9.5), "{score}");
        assert_eq!(chars, 15);
    }

    #[test]
    fn score_rotation_of_no_results_is_zero() {
        assert_eq!(score_rotation(&[]), (0.0, 0));
        // A confidence that is not a number adds characters but no score.
        assert_eq!(score_rotation(&[(f32::NAN, 4)]), (0.0, 4));
    }

    #[test]
    fn char_count_counts_scalar_values_not_bytes_or_utf16_units() {
        assert_eq!(char_count("abc"), 3);
        assert_eq!(char_count("é😀"), 2);
        assert_eq!(char_count(""), 0);
    }

    #[test]
    fn proportional_boxes_split_the_width_by_utf16_length() {
        let boxes = proportional_boxes([0.0, 0.0, 100.0, 20.0], &[1, 3]);
        assert_eq!(boxes.len(), 2);
        assert!(rect_near(boxes[0], [0.0, 0.0, 25.0, 20.0]), "{:?}", boxes[0]);
        assert!(rect_near(boxes[1], [25.0, 0.0, 100.0, 20.0]), "{:?}", boxes[1]);
    }

    #[test]
    fn proportional_boxes_of_an_empty_line_is_empty() {
        assert!(proportional_boxes([0.0, 0.0, 100.0, 20.0], &[]).is_empty());
        assert!(proportional_boxes([0.0, 0.0, 100.0, 20.0], &[0, 0]).is_empty());
    }

    fn line(text: &str, word_quads: Vec<Option<NormQuad>>) -> VisionLine {
        VisionLine { text: text.to_owned(), confidence: 0.75, line_box: [0.1, 0.8, 0.5, 0.05], word_quads }
    }

    fn quad_of(x: f64, y: f64, w: f64, h: f64) -> NormQuad {
        [(x, y + h), (x + w, y + h), (x + w, y), (x, y)]
    }

    #[test]
    fn parse_lines_builds_words_with_the_vision_source() {
        let l = line("Hello world", vec![Some(quad_of(0.1, 0.8, 0.2, 0.05)), Some(quad_of(0.35, 0.8, 0.25, 0.05))]);
        let lines = parse_lines(&[l], 1000, 2000);
        assert_eq!(lines.len(), 1);
        let words = &lines[0].words;
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hello", "world"]);
        assert!(rect_near(words[0].rect, [100.0, 300.0, 300.0, 400.0]), "{:?}", words[0].rect);
        assert!(rect_near(words[1].rect, [350.0, 300.0, 600.0, 400.0]), "{:?}", words[1].rect);
        for w in words {
            assert_eq!((w.confidence, w.source.as_str()), (0.75, "vision"));
        }
    }

    #[test]
    fn parse_lines_falls_back_to_the_proportional_split() {
        // The specification's A20 test: Vision gives no box for the second word, so it gets a share
        // of the line's box in proportion to the words' lengths ("ab" is 2 of 5 units, "cde" 3).
        let l = VisionLine {
            text: "ab cde".to_owned(),
            confidence: 0.5,
            line_box: [0.1, 0.5, 0.5, 0.1],
            word_quads: vec![Some(quad_of(0.1, 0.5, 0.2, 0.1)), None],
        };
        let lines = parse_lines(&[l], 1000, 1000);
        let words = &lines[0].words;
        // The first word keeps Vision's box; the second takes the share after the first two units.
        assert!(rect_near(words[0].rect, [100.0, 400.0, 300.0, 500.0]), "{:?}", words[0].rect);
        assert!(rect_near(words[1].rect, [300.0, 400.0, 600.0, 500.0]), "{:?}", words[1].rect);

        // With no boxes at all, the whole line is split by length.
        let none = VisionLine { word_quads: Vec::new(), ..line("ab cde", Vec::new()) };
        let words = &parse_lines(&[VisionLine { line_box: [0.1, 0.5, 0.5, 0.1], ..none }], 1000, 1000)[0].words;
        assert!(rect_near(words[0].rect, [100.0, 400.0, 300.0, 500.0]), "{:?}", words[0].rect);
        assert!(rect_near(words[1].rect, [300.0, 400.0, 600.0, 500.0]), "{:?}", words[1].rect);
    }

    #[test]
    fn parse_lines_keeps_the_observations_order() {
        let lines = [line("first", Vec::new()), line("second", Vec::new()), line("third", Vec::new())];
        let parsed = parse_lines(&lines, 100, 100);
        let order: Vec<&str> = parsed.iter().map(|l| l.words[0].text.as_str()).collect();
        assert_eq!(order, ["first", "second", "third"]);
    }

    #[test]
    fn parse_lines_skips_what_cannot_be_placed_and_keeps_boxes_on_the_page() {
        // A line with no words, and a line whose box is not a number, leave nothing behind.
        let empty = line("   ", Vec::new());
        let broken = VisionLine { line_box: [f64::NAN, 0.0, 0.1, 0.1], ..line("word", Vec::new()) };
        assert!(parse_lines(&[empty, broken], 100, 100).is_empty());

        // A box that runs past the page edge is held at the edge.
        let over = line("edge", vec![Some(quad_of(0.9, 0.9, 0.3, 0.3))]);
        let words = &parse_lines(&[over], 100, 100)[0].words;
        assert!(rect_near(words[0].rect, [90.0, 0.0, 100.0, 10.0]), "{:?}", words[0].rect);
    }
}
