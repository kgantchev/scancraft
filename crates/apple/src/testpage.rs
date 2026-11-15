//! Synthetic text pages for the live Vision tests, drawn by the tests themselves.
//!
//! The repository's fixtures hold word bars, not letters, so Vision reads nothing from them.
//! These pages hold real words in capital letters, set in a 5 × 7 pixel font whose shapes were
//! drawn for this file (no font file and no other product's glyphs are involved), and they come
//! with the true box of every word. The words are common English, picked with a seeded
//! `Xorshift64Star`, so a page is the same on every run.
//!
//! WP4's generator will make fuller pages; these exist so WP5's tests do not wait for it.

use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::{Gray8, Plane};

/// The glyph's rows, top to bottom, five bits each with the leftmost pixel in bit 4.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        _ => return None,
    })
}

const WORDS: &[&str] = &[
    "THE", "PAGE", "WAS", "FOUND", "UNDER", "AN", "OLD", "TABLE", "NEAR", "THE", "WINDOW", "LIGHT", "FELL", "ACROSS", "PAPER", "AND", "SHOWED",
    "EVERY", "LINE", "CLEARLY", "WE", "READ", "EACH", "WORD", "TWICE", "BEFORE", "WE", "PUT", "IT", "AWAY", "SMALL", "TOWNS", "KEEP", "THEIR",
    "RECORDS", "IN", "WOODEN", "BOXES", "ON", "HIGH", "SHELVES", "WHERE", "DUST", "GATHERS", "SLOWLY", "MORNING", "BRINGS", "COLD", "WATER", "AND",
    "FRESH", "BREAD", "FROM", "THE", "BAKERY", "ACROSS", "THE", "RIVER", "ANOTHER", "LETTER", "ARRIVED", "TODAY", "WITH", "NEWS", "ABOUT", "THE",
    "GARDEN", "AND", "THE", "ROAD",
];

/// A word of the page and its true box, (left, top, right, bottom) in pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TruthWord {
    pub text: String,
    pub rect: [f32; 4],
}

/// A page of text and what is on it.
pub(crate) struct TestPage {
    pub image: Gray8,
    /// The lines, top to bottom, each a list of words left to right.
    pub lines: Vec<Vec<TruthWord>>,
}

impl TestPage {
    /// The page's text, one string with a space between words and a newline between lines.
    pub(crate) fn text(&self) -> String {
        let lines: Vec<String> = self.lines.iter().map(|l| l.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")).collect();
        lines.join("\n")
    }
}

/// Draws a page of `width` × `height` pixels. `cell` is the side of one font pixel: a letter is
/// 5 cells wide and 7 tall, so its height in pixels is `7 · cell`. Dark ink on white, with the
/// text starting at `top_margin` and running down the page.
pub(crate) fn render(width: u32, height: u32, cell: u32, seed: u64) -> TestPage {
    let mut rng = Xorshift64Star::new(seed);
    let mut pixels = vec![255u8; (width as usize) * (height as usize)];
    let margin = width / 10;
    let advance = 6 * cell;
    let space = 4 * cell;
    let glyph_h = 7 * cell;
    let pitch = glyph_h * 2;
    let mut lines = Vec::new();
    let mut top = height / 12;
    while top + glyph_h <= height - height / 12 {
        let mut line: Vec<TruthWord> = Vec::new();
        let mut left = margin;
        loop {
            let word = WORDS[rng.below(WORDS.len() as u64) as usize];
            let word_w = word.len() as u32 * advance - cell;
            if left + word_w > width - margin {
                break;
            }
            draw_word(&mut pixels, width, word, left, top, cell);
            line.push(TruthWord { text: word.to_owned(), rect: [left as f32, top as f32, (left + word_w) as f32, (top + glyph_h) as f32] });
            left += word_w + space;
        }
        lines.push(line);
        top += pitch;
    }
    TestPage { image: Plane::from_vec(width, height, pixels).expect("page size is valid"), lines }
}

fn draw_word(pixels: &mut [u8], width: u32, word: &str, left: u32, top: u32, cell: u32) {
    for (i, c) in word.chars().enumerate() {
        let Some(rows) = glyph(c) else { continue };
        let x0 = left + i as u32 * 6 * cell;
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits >> (4 - col) & 1 == 0 {
                    continue;
                }
                for dy in 0..cell {
                    let y = top + row as u32 * cell + dy;
                    let start = (y * width + x0 + col * cell) as usize;
                    pixels[start..start + cell as usize].fill(25);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn a_page_is_the_same_on_every_run_and_holds_words_inside_its_margins() {
        let a = render(1000, 1400, 4, 7);
        assert_eq!(a.image, render(1000, 1400, 4, 7).image);
        assert_ne!(a.image, render(1000, 1400, 4, 8).image);
        assert!(a.lines.len() > 10);
        for word in a.lines.iter().flatten() {
            assert!(word.rect[0] >= 100.0 && word.rect[2] <= 900.0, "{word:?}");
            assert!(word.rect[3] > word.rect[1] && word.rect[2] > word.rect[0]);
        }
    }
}
