//! Pages for the tests of A15–A18, drawn with the scene generator's reference-page canvas: its
//! stroke alphabet, its exact anti-aliasing and its ink truth. The pages are small (a quarter of an
//! A4 sheet at 300 DPI) so that the tests run in a debug build.
#![allow(dead_code)]

use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::{BitPlane, Gray8, Plane, PlaneF32, Rgb8, RgbF32};
use scancraft_raster::colour::{to_f32, to_u8_plane};
use scancraft_raster::downscale::{area_downscale, area_downscale_rgb};
use scancraft_raster::helpers::max_filter;
use scancraft_synth::font::{UNITS_PER_EM, glyph, text_units};
use scancraft_synth::page::{Canvas, PAPER, PX_PER_PT};

/// The size of the test sheet in points: half an A5 sheet each way, 1100 × 1550 px at 300 DPI.
pub const SHEET: (f64, f64) = (264.0, 372.0);

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

/// Paragraphs of `size_pt` text from `top` to `bottom` between `left` and `right`, one line every
/// `leading` points, drawn in ink.
pub fn paragraphs(c: &mut Canvas, rng: &mut Xorshift64Star, area: (f64, f64, f64, f64), size_pt: f64, leading: f64) {
    let (left, right, top, bottom) = area;
    let unit = size_pt / UNITS_PER_EM;
    let space = glyph(' ').advance * unit;
    let mut baseline = top;
    while baseline <= bottom {
        let mut x = left;
        loop {
            let word = WORDS[rng.below(WORDS.len() as u64) as usize];
            let width = text_units(word) * unit;
            if x + width > right {
                break;
            }
            x = c.text(x, baseline, size_pt, word, scancraft_synth::page::INK) + space;
        }
        baseline += leading;
    }
}

/// A sheet of 10 pt text with margins of 20 pt.
pub fn text_sheet(seed: u64) -> Canvas {
    sized_sheet(seed, 10.0)
}

/// A sheet of `size_pt` text with margins of 20 pt and the leading of the reference page (1.4 times
/// the size).
pub fn sized_sheet(seed: u64, size_pt: f64) -> Canvas {
    let mut c = Canvas::new(SHEET, PAPER).expect("sheet size");
    let mut rng = Xorshift64Star::new(seed);
    c.text(20.0, 40.0, 20.0, "Quarterly report", scancraft_synth::page::INK);
    paragraphs(&mut c, &mut rng, (20.0, SHEET.0 - 20.0, 64.0, SHEET.1 - 24.0), size_pt, size_pt * 1.4);
    c
}

/// The canvas as a colour plane in [0, 1].
pub fn to_rgb(c: &Canvas) -> RgbF32 {
    let mut data = Vec::with_capacity(c.width as usize * c.height as usize);
    for y in 0..c.height {
        for x in 0..c.width {
            let p = c.pixel(x, y).unwrap_or([0; 3]);
            data.push(p.map(|v| f32::from(v) / 255.0));
        }
    }
    Plane::from_vec(c.width, c.height, data).expect("canvas size")
}

/// The canvas as 8-bit sRGB.
pub fn to_rgb8(c: &Canvas) -> Rgb8 {
    scancraft_raster::colour::to_u8(&to_rgb(c)).expect("8 bits")
}

/// The page lit by `light(x, y)`: the reflectance times the light, per channel.
pub fn lit(page: &RgbF32, light: impl Fn(u32, u32) -> [f32; 3]) -> RgbF32 {
    let mut data = Vec::with_capacity(page.data().len());
    for y in 0..page.height() {
        for x in 0..page.width() {
            let (p, l) = (page.get(x, y).unwrap_or([0.0; 3]), light(x, y));
            data.push([p[0] * l[0], p[1] * l[1], p[2] * l[2]]);
        }
    }
    Plane::from_vec(page.width(), page.height(), data).expect("page size")
}

/// Sensor noise: zero-mean Gaussian of `sigma` on every channel, from a seeded generator.
pub fn noisy(page: &RgbF32, sigma: f32, seed: u64) -> RgbF32 {
    let mut rng = Xorshift64Star::new(seed);
    let mut gauss = move || {
        let (a, b) = (rng.next_f64().max(1e-12), rng.next_f64());
        ((-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()) as f32
    };
    let data = page.data().iter().map(|p| p.map(|v| (v + sigma * gauss()).clamp(0.0, 1.0))).collect();
    Plane::from_vec(page.width(), page.height(), data).expect("page size")
}

/// Where the paper is: every pixel at least `grow` px from the ink and outside the `except`
/// rectangles (in points), as a flat list of flags.
pub fn paper_pixels(ink: &BitPlane, grow: u32, except: &[(f64, f64, f64, f64)]) -> Vec<bool> {
    let (w, h) = (ink.width(), ink.height());
    let mut plane = PlaneF32::new(w, h, 0.0).expect("size");
    for y in 0..h {
        for x in 0..w {
            if ink.get(x, y) == Some(true) {
                plane.set(x, y, 1.0);
            }
        }
    }
    let near = max_filter(&plane, 2 * grow + 1).expect("filter");
    let mut out = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (f64::from(x) / PX_PER_PT, f64::from(y) / PX_PER_PT);
            let excluded = except.iter().any(|&(x0, y0, x1, y1)| px >= x0 && px < x1 && py >= y0 && py < y1);
            out.push(near.get(x, y).unwrap_or(1.0) < 0.5 && !excluded);
        }
    }
    out
}

/// The 8-bit levels of an RGB plane's luma.
pub fn levels(page: &RgbF32) -> Vec<u8> {
    let luma = scancraft_raster::colour::luma_plane(page).expect("luma");
    to_u8_plane(&luma).expect("8 bits").into_vec()
}

/// The values at the flagged positions.
pub fn pick<T: Copy>(values: &[T], flags: &[bool]) -> Vec<T> {
    values.iter().zip(flags).filter(|&(_, &f)| f).map(|(&v, _)| v).collect()
}

/// The `q`-quantile (0 to 1) of the levels, by sorting.
pub fn quantile(values: &[u8], q: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    if v.is_empty() {
        return 0.0;
    }
    f64::from(v[((v.len() - 1) as f64 * q).round() as usize])
}

/// The share of the flagged positions whose value satisfies `ok`.
pub fn share(values: &[u8], flags: &[bool], ok: impl Fn(u8) -> bool) -> f64 {
    let picked = pick(values, flags);
    if picked.is_empty() {
        return 0.0;
    }
    picked.iter().filter(|&&v| ok(v)).count() as f64 / picked.len() as f64
}

/// The sheet at `dpi` instead of 300: the colour page and the ink truth, both reduced by area.
pub fn at_density(c: &Canvas, dpi: f64) -> (RgbF32, BitPlane) {
    let (rgb, truth) = (to_rgb(c), c.ink_mask().expect("ink"));
    if (dpi - 300.0).abs() < 1e-9 {
        return (rgb, truth);
    }
    let (w, h) = (((f64::from(c.width) * dpi / 300.0).round() as u32).max(1), ((f64::from(c.height) * dpi / 300.0).round() as u32).max(1));
    let small = area_downscale_rgb(&rgb, w, h).expect("downscale");
    let mut coverage = PlaneF32::new(c.width, c.height, 0.0).expect("size");
    for y in 0..c.height {
        for x in 0..c.width {
            if truth.get(x, y) == Some(true) {
                coverage.set(x, y, 1.0);
            }
        }
    }
    let reduced = area_downscale(&coverage, w, h).expect("downscale");
    let mut ink = BitPlane::new(w, h).expect("size");
    for y in 0..h {
        for x in 0..w {
            if reduced.get(x, y).unwrap_or(0.0) > 0.5 {
                ink.set(x, y, true);
            }
        }
    }
    (small, ink)
}

/// A camera's view of a page: a little blur and sensor noise, as `ink.rs` of the raster crate does.
pub fn photographed(page: &RgbF32, dpi: f64, noise: f32, seed: u64) -> RgbF32 {
    let blurred = scancraft_enhance::helpers::blur_rgb(page, (0.8 * dpi / 300.0).max(0.3) as f32).expect("blur");
    noisy(&blurred, noise, seed)
}

/// A 1024 px Grayscale page for A15: the canvas reduced to a long side of 1024 px.
pub fn upright_input(c: &Canvas) -> Gray8 {
    let rgb = to_rgb(c);
    let (w, h) = scancraft_raster::downscale::fit_long_side(rgb.width(), rgb.height(), 1024);
    let small = area_downscale_rgb(&rgb, w, h).expect("downscale");
    to_u8_plane(&scancraft_raster::colour::luma_plane(&small).expect("luma")).expect("8 bits")
}

/// Turns a grey page clockwise by `quarters` quarter turns, exactly.
pub fn rotate(page: &Gray8, quarters: u8) -> Gray8 {
    let (w, h) = (page.width(), page.height());
    let (ow, oh) = if quarters.is_multiple_of(2) { (w, h) } else { (h, w) };
    let mut data = Vec::with_capacity(page.data().len());
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match quarters % 4 {
                0 => (x, y),
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            data.push(page.get(sx, sy).expect("inside"));
        }
    }
    Plane::from_vec(ow, oh, data).expect("size")
}

/// 8-bit sRGB to f32, for pages that arrive as bytes.
pub fn rgb_f32(page: &Rgb8) -> RgbF32 {
    to_f32(page).expect("f32")
}
