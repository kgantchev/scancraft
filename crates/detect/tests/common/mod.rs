//! Pages for the tests of A26 and A30, drawn with the scene generator's stroke alphabet and its
//! exact anti-aliasing.
#![allow(dead_code)]

use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{Plane, PlaneF32, Rgb8, RgbF32};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_raster::downscale::area_downscale;
use scancraft_synth::font::{UNITS_PER_EM, glyph, text_units};
use scancraft_synth::page::{Canvas, INK, PAPER, PX_PER_PT};

const WORDS: &[&str] = &[
    "the",
    "of",
    "and",
    "to",
    "in",
    "is",
    "that",
    "for",
    "it",
    "as",
    "with",
    "was",
    "on",
    "are",
    "be",
    "this",
    "by",
    "at",
    "from",
    "or",
    "an",
    "page",
    "paper",
    "light",
    "table",
    "photo",
    "camera",
    "scan",
    "line",
    "edge",
    "corner",
    "text",
    "word",
    "small",
    "large",
    "report",
    "number",
    "total",
    "date",
    "name",
    "letter",
    "sheet",
    "book",
    "note",
    "plan",
    "which",
    "their",
    "would",
    "about",
    "other",
    "these",
    "after",
    "first",
    "water",
    "little",
    "never",
    "great",
    "where",
    "again",
    "house",
    "place",
    "right",
    "think",
    "world",
    "still",
    "between",
    "because",
    "through",
    "against",
    "without",
    "another",
    "important",
    "together",
    "document",
    "quarterly",
    "invoice",
    "agreement",
    "schedule",
    "measure",
    "pattern",
];

/// The x-height of the alphabet, as a share of the em: the letters' bodies are four of the six
/// grid units, and an em is 6 / 0.72 units.
pub const X_HEIGHT: f64 = 4.0 / UNITS_PER_EM;

/// An A4 sheet in points.
pub const A4: (f64, f64) = (595.28, 841.89);
/// The long side of the analysis copy (A26 step 1).
pub const COPY_LONG_SIDE: u32 = 1600;

/// A sheet of `lines` lines of `size_pt` text, one column, with the margins of 40 pt.
pub fn text_sheet(lines: usize, size_pt: f64, leading: f64, seed: u64) -> (Canvas, Vec<f64>) {
    let mut c = Canvas::new(A4, PAPER).expect("sheet");
    let mut rng = Xorshift64Star::new(seed);
    let mut baselines = Vec::with_capacity(lines);
    for line in 0..lines {
        let baseline = 60.0 + line as f64 * leading;
        let mut x = 40.0;
        loop {
            let word = WORDS[rng.below(WORDS.len() as u64) as usize];
            let width_of = text_units(word) * size_pt / UNITS_PER_EM;
            if x + width_of > A4.0 - 40.0 {
                break;
            }
            x = c.text(x, baseline, size_pt, word, INK) + glyph(' ').advance * size_pt / UNITS_PER_EM;
        }
        baselines.push(baseline);
    }
    (c, baselines)
}

/// A sheet of `lines` lines of text in two columns, the left one and the right one with the same
/// baselines.
pub fn two_column_sheet(lines: usize, size_pt: f64, leading: f64, seed: u64) -> (Canvas, Vec<f64>) {
    let mut c = Canvas::new(A4, PAPER).expect("sheet");
    let mut rng = Xorshift64Star::new(seed);
    let mut baselines = Vec::with_capacity(lines);
    for line in 0..lines {
        let baseline = 60.0 + line as f64 * leading;
        for column in 0..2 {
            let left = 40.0 + column as f64 * 260.0;
            let mut x = left;
            loop {
                let word = WORDS[rng.below(WORDS.len() as u64) as usize];
                let width_of = text_units(word) * size_pt / UNITS_PER_EM;
                if x + width_of > left + 235.0 {
                    break;
                }
                x = c.text(x, baseline, size_pt, word, INK) + glyph(' ').advance * size_pt / UNITS_PER_EM;
            }
        }
        baselines.push(baseline);
    }
    (c, baselines)
}

/// An open book: two pages side by side with a gutter, whose darkness is `gutter` (0 for no gutter
/// at all) and whose centre leans from `lean_from` to `lean_to` of the width over the height.
pub fn spread_sheet(gutter: f64, lean_from: f64, lean_to: f64, size_pt: f64) -> Canvas {
    let (width, height) = (A4.0 * 2.0, A4.1);
    let mut c = Canvas::new((width, height), PAPER).expect("sheet");
    let mut rng = Xorshift64Star::new(9);
    // The text of both pages, with the margins of the gutter.
    for page in 0..2 {
        // The text stops short of the gutter, as it does in a real book: no line crosses the fold.
        let left = f64::from(page) * A4.0 + 40.0;
        let right = f64::from(page + 1) * A4.0 - 80.0;
        let mut baseline = 60.0;
        while baseline < height - 40.0 {
            let mut x = left;
            loop {
                let word = WORDS[rng.below(WORDS.len() as u64) as usize];
                let width_of = text_units(word) * size_pt / UNITS_PER_EM;
                if x + width_of > right {
                    break;
                }
                x = c.text(x, baseline, size_pt, word, INK) + glyph(' ').advance * size_pt / UNITS_PER_EM;
            }
            baseline += 14.0;
        }
    }
    // The gutter: a dark band whose centre leans across the height. It is drawn as three bands
    // a row, so that its edge is soft without needing a colour per pixel.
    let sheet_width = f64::from(c.width);
    for y in 0..c.height {
        let v = f64::from(y) / f64::from(c.height - 1);
        let centre = (lean_from + (lean_to - lean_from) * v) * sheet_width;
        let centre_pt = centre / PX_PER_PT;
        // The shadow of the fold: a dark band at the fold, and a soft shade around it that widens
        // as the fold leans, the way the shade of a book's gutter does.
        for (offset, factor, half) in [(0.0f64, 0.35f64, 2.5), (0.0, 0.65, 22.0)] {
            let x0 = (centre_pt + offset - half).max(0.0);
            let x1 = (centre_pt + offset + half).min(width);
            let y0 = f64::from(y) / PX_PER_PT;
            let y1 = (f64::from(y) + 1.0) / PX_PER_PT;
            let paper = f64::from(PAPER[0]);
            let level = (paper * (1.0 - (1.0 - factor) * gutter)).round() as u8;
            c.fill_rect((x0, y0, x1, y1), [level, level, level], false);
        }
    }
    c
}

/// The canvas as a colour plane in [0, 1].
pub fn to_rgb(c: &Canvas) -> RgbF32 {
    let mut data = Vec::with_capacity(c.width as usize * c.height as usize);
    for y in 0..c.height {
        for x in 0..c.width {
            data.push(c.pixel(x, y).unwrap_or([0; 3]).map(|v| f32::from(v) / 255.0));
        }
    }
    Plane::from_vec(c.width, c.height, data).expect("canvas")
}

/// The canvas as 8-bit sRGB.
pub fn to_rgb8(c: &Canvas) -> Rgb8 {
    let mut data = Vec::with_capacity(c.width as usize * c.height as usize);
    for y in 0..c.height {
        for x in 0..c.width {
            data.push(c.pixel(x, y).unwrap_or([0; 3]));
        }
    }
    Plane::from_vec(c.width, c.height, data).expect("canvas")
}

/// The luma of a canvas, in [0, 1].
pub fn to_luma(c: &Canvas) -> PlaneF32 {
    scancraft_raster::colour::luma_plane(&to_rgb(c)).expect("luma")
}

/// The luma pyramid of a canvas, as A10 builds it.
pub fn pyramid(c: &Canvas) -> LumaPyramid {
    scancraft_detect::refine::build_pyramid(&to_luma(c)).expect("pyramid")
}

/// The analysis copy of a head-on page: the luma reduced to the copy's size, and the copy's
/// density in pixels per inch.
pub fn copy_of(c: &Canvas) -> (PlaneF32, f64) {
    let luma = to_luma(c);
    let (w, h) = if c.width as f64 / c.height as f64 >= A4.0 / A4.1 {
        (COPY_LONG_SIDE, (f64::from(COPY_LONG_SIDE) * A4.0 / A4.1).round() as u32)
    } else {
        ((f64::from(COPY_LONG_SIDE) * A4.0 / A4.1).round() as u32, COPY_LONG_SIDE)
    };
    let copy = area_downscale(&luma, w, h).expect("copy");
    // The copy's density: its long side over the page's long side in inches.
    let dpi = f64::from(copy.width().max(copy.height())) / (A4.1.max(A4.0) / 72.0);
    (copy, dpi)
}

/// The scale from the canvas's rows to the copy's rows.
pub fn row_scale(c: &Canvas) -> f64 {
    let (copy, _) = copy_of(c);
    f64::from(copy.height()) / f64::from(c.height)
}

/// The corners of a whole picture, in photo pixels.
pub fn whole_quad(width: u32, height: u32) -> Quad {
    Quad([
        Point::new(0.0, 0.0),
        Point::new(f64::from(width), 0.0),
        Point::new(f64::from(width), f64::from(height)),
        Point::new(0.0, f64::from(height)),
    ])
}

/// The sheet's corners in photo pixels, with a margin of `margin` px.
pub fn sheet_quad(c: &Canvas, margin: f64) -> Quad {
    let (w, h) = (f64::from(c.width), f64::from(c.height));
    Quad([Point::new(margin, margin), Point::new(w - margin, margin), Point::new(w - margin, h - margin), Point::new(margin, h - margin)])
}

/// The page's size in millimetres, from the canvas's points.
pub fn size_mm(c: &Canvas) -> (f64, f64) {
    (c.width as f64 / PX_PER_PT * 25.4 / 72.0, c.height as f64 / PX_PER_PT * 25.4 / 72.0)
}

/// The centre of mass of a line's ink, in the canvas's rows: where the line's true middle is.
pub fn centre_of_mass(c: &Canvas, baseline: f64, leading: f64) -> f64 {
    let (y0, y1) = ((baseline - leading / 2.0) * PX_PER_PT, (baseline + leading / 2.0) * PX_PER_PT);
    let (y0, y1) = (y0.max(0.0) as u32, (y1.min(f64::from(c.height)) as u32).max(y0 as u32 + 1));
    let mut sum = 0f64;
    let mut moment = 0f64;
    for y in y0..y1 {
        let mut count = 0f64;
        for x in 0..c.width {
            let p = c.pixel(x, y).unwrap_or([255; 3]);
            let luma = 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
            if luma < 200.0 {
                count += 1.0;
            }
        }
        sum += count;
        moment += count * (f64::from(y) + 0.5);
    }
    if sum <= 0.0 {
        return f64::from(y0);
    }
    moment / sum
}

/// The plane sheared, so that its horizontal lines rise by `slope` for every pixel of x.
pub fn shear(src: &PlaneF32, slope: f32) -> PlaneF32 {
    let (w, h) = (src.width(), src.height());
    let mut data = vec![0f32; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            let sy = (f64::from(y) - f64::from(x) * f64::from(slope)).round();
            let sy = sy.clamp(0.0, f64::from(h) - 1.0) as u32;
            data[(y as usize) * (w as usize) + (x as usize)] = src.get(x, sy).unwrap_or(0.0);
        }
    }
    Plane::from_vec(w, h, data).expect("plane")
}

/// The plane bent: the content moves down by `amplitude` at the middle of the page and back to
/// where it was at the edges, the way a curled sheet carries its text.
///
/// The steepest slope is `amplitude · 2π / width`.
pub fn bend(src: &PlaneF32, amplitude: f64) -> PlaneF32 {
    let (w, h) = (src.width(), src.height());
    let mut data = vec![0f32; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            let shift = amplitude * (2.0 * std::f64::consts::PI * f64::from(x) / f64::from(w)).sin();
            let sy = (f64::from(y) - shift).round();
            let sy = sy.clamp(0.0, f64::from(h) - 1.0) as u32;
            data[(y as usize) * (w as usize) + (x as usize)] = src.get(x, sy).unwrap_or(0.0);
        }
    }
    Plane::from_vec(w, h, data).expect("plane")
}

/// The plane turned a quarter turn clockwise.
pub fn turn(src: &PlaneF32) -> PlaneF32 {
    let (w, h) = (src.width(), src.height());
    let mut data = vec![0f32; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            // A quarter turn clockwise sends (x, y) to the column h − 1 − y and the row x.
            data[(x as usize) * (h as usize) + (h as usize - 1 - y as usize)] = src.get(x, y).unwrap_or(0.0);
        }
    }
    Plane::from_vec(h, w, data).expect("plane")
}

/// The centre of mass of a line's ink over its band, in the columns of one strip, in the copy's
/// rows. This is the line's true middle, measured from the page rather than from the tracker.
pub fn middle_in_strip(copy: &PlaneF32, x0: u32, x1: u32, baseline: f64, x_height: f64) -> f64 {
    // The band of the line: the body of the letters, from below their feet to the top of their
    // x-height.
    let y0 = (baseline - 1.5 * x_height).max(0.0) as u32;
    let y1 = ((baseline + 0.4 * x_height).max(1.0) as u32).min(copy.height());
    let mut sum = 0f64;
    let mut moment = 0f64;
    for y in y0..y1 {
        let mut count = 0f64;
        for x in x0..x1.min(copy.width()) {
            if copy.get(x, y).unwrap_or(1.0) < 0.75 {
                count += 1.0;
            }
        }
        sum += count;
        moment += count * (f64::from(y) + 0.5);
    }
    if sum <= 0.0 {
        return baseline - x_height / 2.0;
    }
    moment / sum
}
