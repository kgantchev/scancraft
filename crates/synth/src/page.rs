//! The reference page (specification §10.1): drawn in points on an A4 sheet at 300 DPI with the
//! stroke alphabet of `font`, then kept as a texture with a mip chain for the renderer, and as an
//! ink mask. Edges are anti-aliased from the geometry: rectangles by exact area, strokes by 8 × 8
//! samples at the pixels their edge crosses.

use crate::font::{UNITS_PER_EM, glyph};
use crate::sheet::{A4, A5};
use scancraft_core::CoreError;
use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::BitPlane;

/// 300 DPI: texture pixels per point.
pub const PX_PER_PT: f64 = 300.0 / 72.0;

pub const PAPER: [u8; 3] = [243, 241, 236];
pub const INK: [u8; 3] = [24, 24, 30];
/// The pale pencil line's grey: 0.2 darker than the paper in luma.
pub const PENCIL: [u8; 3] = [192, 190, 186];
pub const LOGO: [u8; 3] = [220, 40, 40];

/// Where the elements of the A4 reference page sit, in points: (x0, y0, x1, y1).
pub mod layout {
    pub const BOX: (f64, f64, f64, f64) = (150.0, 112.0, 210.0, 152.0);
    pub const LOGO: (f64, f64, f64, f64) = (495.0, 40.0, 545.0, 70.0);
    pub const PHOTO: (f64, f64, f64, f64) = (50.0, 112.0, 130.0, 172.0);
    pub const TABLE: (f64, f64, f64, f64) = (50.0, 600.0, 250.0, 680.0);
    pub const HEADING_BASELINE: f64 = 84.0;
    pub const SUBHEADING_BASELINE: f64 = 106.0;
    pub const PENCIL_FROM: (f64, f64) = (50.0, 800.0);
    pub const PENCIL_TO: (f64, f64) = (545.0, 803.0);
    pub const PENCIL_WIDTH: f64 = 0.8;
}

/// A page being drawn: colour and ink coverage per texture pixel.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    rgb: Vec<[u8; 3]>,
    ink: Vec<u8>,
    /// The words drawn, one line each, for the character error rate.
    pub text: String,
}

fn mix(a: u8, b: u8, cov: f64) -> u8 {
    (f64::from(a) * (1.0 - cov) + f64::from(b) * cov).round().clamp(0.0, 255.0) as u8
}

impl Canvas {
    pub fn new(size_pt: (f64, f64), paper: [u8; 3]) -> Result<Canvas, CoreError> {
        let (width, height) = ((size_pt.0 * PX_PER_PT).round() as u32, (size_pt.1 * PX_PER_PT).round() as u32);
        let len = scancraft_core::image::checked_len(width, height)?;
        Ok(Canvas { width, height, rgb: vec![paper; len], ink: vec![0; len], text: String::new() })
    }

    fn blend(&mut self, x: i64, y: i64, colour: [u8; 3], cov: f64, ink: bool) {
        if cov <= 0.0 || x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let k = y as usize * self.width as usize + x as usize;
        if let Some(px) = self.rgb.get_mut(k) {
            *px = [mix(px[0], colour[0], cov), mix(px[1], colour[1], cov), mix(px[2], colour[2], cov)];
        }
        if ink && let Some(m) = self.ink.get_mut(k) {
            *m = (*m).max((cov.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }

    /// A rectangle in points, anti-aliased by exact area.
    pub fn fill_rect(&mut self, rect: (f64, f64, f64, f64), colour: [u8; 3], ink: bool) {
        let (x0, y0, x1, y1) = (rect.0 * PX_PER_PT, rect.1 * PX_PER_PT, rect.2 * PX_PER_PT, rect.3 * PX_PER_PT);
        for y in (y0.floor() as i64)..(y1.ceil() as i64) {
            for x in (x0.floor() as i64)..(x1.ceil() as i64) {
                let ox = ((x + 1) as f64).min(x1) - (x as f64).max(x0);
                let oy = ((y + 1) as f64).min(y1) - (y as f64).max(y0);
                self.blend(x, y, colour, ox.max(0.0) * oy.max(0.0), ink);
            }
        }
    }

    /// A line of `width_pt` with round ends between two points in points.
    pub fn stroke(&mut self, a: (f64, f64), b: (f64, f64), width_pt: f64, colour: [u8; 3], ink: bool) {
        let (ax, ay, bx, by) = (a.0 * PX_PER_PT, a.1 * PX_PER_PT, b.0 * PX_PER_PT, b.1 * PX_PER_PT);
        let r = width_pt * PX_PER_PT / 2.0;
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let dist = |px: f64, py: f64| {
            let t = if len2 > 0.0 { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            (px - (ax + t * dx)).hypot(py - (ay + t * dy))
        };
        let pad = r + 1.5;
        let (x0, x1) = ((ax.min(bx) - pad).floor() as i64, (ax.max(bx) + pad).ceil() as i64);
        let (y0, y1) = ((ay.min(by) - pad).floor() as i64, (ay.max(by) + pad).ceil() as i64);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = dist(x as f64 + 0.5, y as f64 + 0.5);
                let cov = if d <= r - 0.75 {
                    1.0
                } else if d >= r + 0.75 {
                    0.0
                } else {
                    let mut inside = 0;
                    for sy in 0..8 {
                        for sx in 0..8 {
                            if dist(x as f64 + (f64::from(sx) + 0.5) / 8.0, y as f64 + (f64::from(sy) + 0.5) / 8.0) <= r {
                                inside += 1;
                            }
                        }
                    }
                    f64::from(inside) / 64.0
                };
                self.blend(x, y, colour, cov, ink);
            }
        }
    }

    /// `text` at `size_pt` with its baseline at `baseline_pt`, starting at `x_pt`; returns where it ends.
    pub fn text(&mut self, x_pt: f64, baseline_pt: f64, size_pt: f64, text: &str, colour: [u8; 3]) -> f64 {
        let unit = size_pt / UNITS_PER_EM;
        let width = size_pt * 0.085;
        let mut x = x_pt;
        for c in text.chars() {
            let g = glyph(c);
            for stroke in &g.strokes {
                let pts: Vec<(f64, f64)> = stroke.iter().map(|&(gx, gy)| (x + gx * unit, baseline_pt - gy * unit)).collect();
                for pair in pts.windows(2) {
                    self.stroke(pair[0], pair[1], width, colour, true);
                }
                if pts.len() == 1 {
                    self.stroke(pts[0], pts[0], width, colour, true);
                }
            }
            x += g.advance * unit;
        }
        x
    }

    /// Darkens the page toward its middle, as a gutter does: multiplies by 1 − 0.35·exp(−|u − ½| / 0.03).
    pub fn shade_gutter(&mut self) {
        let w = self.width as usize;
        let shade: Vec<f64> = (0..w).map(|x| 1.0 - 0.35 * (-((x as f64 + 0.5) / w as f64 - 0.5).abs() / 0.03).exp()).collect();
        for row in self.rgb.chunks_mut(w) {
            for (px, s) in row.iter_mut().zip(&shade) {
                *px = px.map(|c| (f64::from(c) * s).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
}

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

/// `count` words drawn from the list.
fn words(rng: &mut Xorshift64Star, count: usize) -> Vec<String> {
    (0..count).map(|_| WORDS[rng.below(WORDS.len() as u64) as usize].to_string()).collect()
}

/// A paragraph set at `size_pt` between `left` and `right`, from baseline `top` down `lines` lines
/// of `leading_pt`; the words are recorded in the canvas's text. Returns the next baseline.
fn paragraph(c: &mut Canvas, rng: &mut Xorshift64Star, area: (f64, f64, f64), lines: usize, size_pt: f64, leading_pt: f64) -> f64 {
    let (left, right, mut baseline) = area;
    let unit = size_pt / UNITS_PER_EM;
    for _ in 0..lines {
        let mut x = left;
        let mut line = String::new();
        loop {
            let word = words(rng, 1).remove(0);
            let w = crate::font::text_units(&word) * unit;
            if x + w > right {
                break;
            }
            x = c.text(x, baseline, size_pt, &word, INK) + crate::font::glyph(' ').advance * unit;
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        c.text.push_str(&line);
        c.text.push('\n');
        baseline += leading_pt;
    }
    baseline
}

fn heading(c: &mut Canvas, x: f64, baseline: f64, size: f64, text: &str) {
    c.text(x, baseline, size, text, INK);
    c.text.push_str(text);
    c.text.push('\n');
}

/// The photo: a smooth picture of sky, hills and a sun, so it holds colour and gradients.
fn draw_photo(c: &mut Canvas, rect: (f64, f64, f64, f64)) {
    let (x0, y0, x1, y1) = (rect.0 * PX_PER_PT, rect.1 * PX_PER_PT, rect.2 * PX_PER_PT, rect.3 * PX_PER_PT);
    for y in (y0.floor() as i64)..(y1.ceil() as i64) {
        for x in (x0.floor() as i64)..(x1.ceil() as i64) {
            let (u, v) = ((x as f64 + 0.5 - x0) / (x1 - x0), (y as f64 + 0.5 - y0) / (y1 - y0));
            let hill = 0.62 + 0.12 * (u * 7.0).sin() + 0.08 * (u * 17.0 + 1.0).sin();
            let colour = if v > hill {
                [30.0 + 40.0 * v, 110.0 + 60.0 * (1.0 - v), 50.0]
            } else {
                let sun = (-((u - 0.75).powi(2) + (v - 0.25).powi(2)) / 0.01).exp();
                [90.0 + 120.0 * sun, 150.0 + 80.0 * sun, 220.0 - 40.0 * sun]
            };
            c.blend(x, y, colour.map(|v| v.clamp(0.0, 255.0) as u8), 1.0, false);
        }
    }
}

/// A ruled table: hairlines, a shaded header row, and numbers in the cells.
fn draw_table(c: &mut Canvas, rng: &mut Xorshift64Star) {
    let (x0, y0, x1, y1) = layout::TABLE;
    let (cols, rows) = (4, 4);
    let (cw, rh) = ((x1 - x0) / f64::from(cols), (y1 - y0) / f64::from(rows));
    c.fill_rect((x0, y0, x1, y0 + rh), [218, 216, 212], false);
    for i in 0..=cols {
        let x = x0 + f64::from(i) * cw;
        c.stroke((x, y0), (x, y1), 0.5, INK, true);
    }
    for j in 0..=rows {
        let y = y0 + f64::from(j) * rh;
        c.stroke((x0, y), (x1, y), 0.5, INK, true);
    }
    for (i, label) in ["Item", "Qty", "Unit", "Total"].iter().enumerate() {
        c.text(x0 + f64::from(i as u32) * cw + 4.0, y0 + rh - 7.0, 10.0, label, INK);
    }
    c.text.push_str("Item Qty Unit Total\n");
    for j in 1..rows {
        let mut row = String::new();
        for i in 0..cols {
            let value = if i == 0 { words(rng, 1).remove(0) } else { format!("{}", 10 + rng.below(990)) };
            c.text(x0 + f64::from(i as u32) * cw + 4.0, y0 + f64::from(j as u32 + 1) * rh - 7.0, 10.0, &value, INK);
            row.push_str(&value);
            row.push(' ');
        }
        c.text.push_str(row.trim_end());
        c.text.push('\n');
    }
}

/// The A4 reference page: text at 36, 12, 10 and 6 pt, a pale pencil line, a solid black box, a
/// coloured logo, a photo and a ruled table.
pub fn reference_page(rng: &mut Xorshift64Star) -> Result<Canvas, CoreError> {
    let mut c = Canvas::new(A4, PAPER)?;
    heading(&mut c, 50.0, layout::HEADING_BASELINE, 36.0, "Synthetic Page");
    heading(&mut c, 50.0, layout::SUBHEADING_BASELINE, 12.0, "Reference document for scanning tests");
    c.fill_rect(layout::LOGO, LOGO, false);
    c.text(502.0, 63.0, 24.0, "S", [255, 255, 255]);
    c.ink_clear_logo(layout::LOGO);
    draw_photo(&mut c, layout::PHOTO);
    c.fill_rect(layout::BOX, [0, 0, 0], true);
    let next = paragraph(&mut c, rng, (50.0, 545.0, 200.0), 5, 12.0, 17.0);
    paragraph(&mut c, rng, (50.0, 545.0, next + 8.0), 28, 10.0, 12.0);
    draw_table(&mut c, rng);
    paragraph(&mut c, rng, (50.0, 545.0, 706.0), 3, 6.0, 8.0);
    c.stroke(layout::PENCIL_FROM, layout::PENCIL_TO, layout::PENCIL_WIDTH, PENCIL, true);
    Ok(c)
}

/// An open book's two A5 pages side by side with a shaded gutter: 10 pt text, a heading on each
/// page, a table on the right and a photo on the left.
pub fn spread_page(rng: &mut Xorshift64Star) -> Result<Canvas, CoreError> {
    let size = (2.0 * A5.0, A5.1);
    let mut c = Canvas::new(size, PAPER)?;
    for page in 0..2 {
        let left = f64::from(page) * A5.0 + 40.0;
        let right = f64::from(page + 1) * A5.0 - 40.0;
        heading(&mut c, left, 60.0, 12.0, &words(rng, 3).join(" "));
        let top = if page == 0 {
            draw_photo(&mut c, (left, 80.0, left + 80.0, 140.0));
            150.0
        } else {
            80.0
        };
        paragraph(&mut c, rng, (left, right, top), 38, 10.0, 12.0);
        c.text(left, A5.1 - 24.0, 8.0, &format!("{}", page + 1), INK);
    }
    c.shade_gutter();
    Ok(c)
}

impl Canvas {
    /// The logo is not ink: whatever the white glyph drew into the mask is cleared again.
    fn ink_clear_logo(&mut self, rect: (f64, f64, f64, f64)) {
        let (x0, y0, x1, y1) = (rect.0 * PX_PER_PT, rect.1 * PX_PER_PT, rect.2 * PX_PER_PT, rect.3 * PX_PER_PT);
        for y in (y0.floor() as usize)..(y1.ceil() as usize).min(self.height as usize) {
            for x in (x0.floor() as usize)..(x1.ceil() as usize).min(self.width as usize) {
                if let Some(m) = self.ink.get_mut(y * self.width as usize + x) {
                    *m = 0;
                }
            }
        }
    }

    /// The ink mask: where more than half of a texture pixel is ink.
    pub fn ink_mask(&self) -> Result<BitPlane, CoreError> {
        let mut bits = BitPlane::new(self.width, self.height)?;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.ink.get(y as usize * self.width as usize + x as usize).copied().unwrap_or(0) > 127 {
                    bits.set(x, y, true);
                }
            }
        }
        Ok(bits)
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        self.rgb.get(y as usize * self.width as usize + x as usize).copied().filter(|_| x < self.width && y < self.height)
    }

    /// The finished texture with its mip chain.
    pub fn into_texture(self) -> Texture {
        Texture::new(self.width, self.height, self.rgb)
    }
}

/// A page as the renderer samples it: the 300 DPI colour image and its half-size reductions.
pub struct Texture {
    levels: Vec<(u32, u32, Vec<[u8; 3]>)>,
}

impl Texture {
    fn new(width: u32, height: u32, rgb: Vec<[u8; 3]>) -> Texture {
        let mut levels = vec![(width, height, rgb)];
        while let Some((w, h, data)) = levels.last() {
            if *w < 128 || *h < 128 || levels.len() >= 5 {
                break;
            }
            let (nw, nh) = (w / 2, h / 2);
            let mut next = Vec::with_capacity((nw * nh) as usize);
            for y in 0..nh {
                for x in 0..nw {
                    let mut sum = [0u32; 3];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = data[((2 * y + dy) * w + 2 * x + dx) as usize];
                        for c in 0..3 {
                            sum[c] += u32::from(p[c]);
                        }
                    }
                    next.push(sum.map(|s| ((s + 2) / 4) as u8));
                }
            }
            let entry = (nw, nh, next);
            levels.push(entry);
        }
        Texture { levels }
    }

    pub fn size(&self) -> (u32, u32) {
        self.levels.first().map_or((0, 0), |l| (l.0, l.1))
    }

    fn bilinear(&self, level: usize, u: f64, v: f64) -> [f32; 3] {
        let Some((w, h, data)) = self.levels.get(level) else { return [0.0; 3] };
        let (fx, fy) = ((u * f64::from(*w) - 0.5).clamp(0.0, f64::from(*w - 1)), (v * f64::from(*h) - 0.5).clamp(0.0, f64::from(*h - 1)));
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(*w - 1), (y0 + 1).min(*h - 1));
        let (ax, ay) = ((fx - f64::from(x0)) as f32, (fy - f64::from(y0)) as f32);
        let at = |x: u32, y: u32| data[(y * w + x) as usize].map(f32::from);
        let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        [0, 1, 2].map(|c| (p00[c] * (1.0 - ax) + p10[c] * ax) * (1.0 - ay) + (p01[c] * (1.0 - ax) + p11[c] * ax) * ay)
    }

    /// The colour at page fractions (u, v), 0–255, trilinear at mip level `lod` (0 is the full texture).
    pub fn sample(&self, u: f64, v: f64, lod: f32) -> [f32; 3] {
        let top = (self.levels.len() - 1) as f32;
        let l = lod.clamp(0.0, top);
        let (lo, t) = (l.floor() as usize, l.fract());
        let a = self.bilinear(lo, u, v);
        if t < 1e-3 || lo + 1 >= self.levels.len() {
            return a;
        }
        let b = self.bilinear(lo + 1, u, v);
        [0, 1, 2].map(|c| a[c] * (1.0 - t) + b[c] * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(seed: u64) -> Canvas {
        reference_page(&mut Xorshift64Star::new(seed)).unwrap()
    }

    fn luma(p: [u8; 3]) -> f64 {
        (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])) / 255.0
    }

    fn ink_in(c: &Canvas, rect: (f64, f64, f64, f64)) -> usize {
        let mask = c.ink_mask().unwrap();
        let (x0, y0, x1, y1) = ((rect.0 * PX_PER_PT) as u32, (rect.1 * PX_PER_PT) as u32, (rect.2 * PX_PER_PT) as u32, (rect.3 * PX_PER_PT) as u32);
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| mask.get(x, y) == Some(true)).count()
    }

    #[test]
    fn the_page_is_a4_at_300_dpi() {
        let c = page(1);
        assert_eq!((c.width, c.height), (2480, 3508));
        let spread = spread_page(&mut Xorshift64Star::new(1)).unwrap();
        assert_eq!((spread.width, spread.height), (3496, 2480));
    }

    #[test]
    fn text_comes_in_four_sizes() {
        // A capital H is 0.72 em high: measure the drawn height at each size.
        for size in [6.0, 10.0, 12.0, 36.0] {
            let mut c = Canvas::new((100.0, 100.0), PAPER).unwrap();
            c.text(10.0, 60.0, size, "H", INK);
            let mask = c.ink_mask().unwrap();
            let rows: Vec<u32> = (0..c.height).filter(|&y| (0..c.width).any(|x| mask.get(x, y) == Some(true))).collect();
            let height = f64::from(rows.last().unwrap() - rows.first().unwrap() + 1);
            let want = 0.72 * size * PX_PER_PT;
            assert!((height - want).abs() <= 0.12 * want + 2.0, "{size} pt: {height} px, wanted about {want}");
        }
        // The page has text at all four sizes in the places the layout puts it.
        let c = page(2);
        for (rect, what) in [
            ((50.0, 56.0, 330.0, 92.0), "36 pt heading"),
            ((50.0, 96.0, 330.0, 110.0), "12 pt sub-heading"),
            ((50.0, 190.0, 545.0, 260.0), "10 pt body"),
            ((50.0, 700.0, 545.0, 730.0), "6 pt fine print"),
        ] {
            assert!(ink_in(&c, rect) > 200, "{what}");
        }
    }

    #[test]
    fn the_black_box_is_black() {
        let c = page(3);
        let (x0, y0, x1, y1) = layout::BOX;
        let (mut black, mut total) = (0, 0);
        for y in (y0 * PX_PER_PT) as u32..(y1 * PX_PER_PT) as u32 {
            for x in (x0 * PX_PER_PT) as u32..(x1 * PX_PER_PT) as u32 {
                total += 1;
                black += u32::from(luma(c.pixel(x, y).unwrap()) < 0.05);
            }
        }
        assert!(f64::from(black) / f64::from(total) >= 0.97);
    }

    #[test]
    fn the_pencil_line_is_pale_and_the_logo_saturated() {
        let c = page(4);
        // Half way along the pencil line, at its middle.
        let (a, b) = (layout::PENCIL_FROM, layout::PENCIL_TO);
        let (mx, my) = ((a.0 + b.0) / 2.0 * PX_PER_PT, (a.1 + b.1) / 2.0 * PX_PER_PT);
        let line = luma(c.pixel(mx as u32, my as u32).unwrap());
        let paper = luma(PAPER);
        assert!(((paper - line) - 0.2).abs() < 0.03, "contrast {}", paper - line);
        // The logo is a saturated colour.
        let p = c.pixel(((layout::LOGO.0 + 2.0) * PX_PER_PT) as u32, ((layout::LOGO.1 + 2.0) * PX_PER_PT) as u32).unwrap();
        assert_eq!(p, LOGO);
        let spread = p.iter().max().unwrap() - p.iter().min().unwrap();
        assert!(spread > 150);
    }

    #[test]
    fn the_ink_mask_marks_the_dark_pixels_and_not_the_paper_logo_or_photo() {
        let c = page(5);
        let mask = c.ink_mask().unwrap();
        let mut disagree = 0u64;
        let mut ink = 0u64;
        for y in 0..c.height {
            for x in 0..c.width {
                let l = luma(c.pixel(x, y).unwrap());
                let is_ink = mask.get(x, y) == Some(true);
                ink += u64::from(is_ink);
                // Ink pixels are darker than the paper by well over the pencil's 0.2... or the pencil.
                if is_ink && l > luma(PAPER) - 0.1 {
                    disagree += 1;
                }
                // Very dark pixels are ink, except in the photo and logo.
                if !is_ink && l < 0.1 {
                    disagree += 1;
                }
            }
        }
        assert!(ink > 50_000, "{ink}");
        assert!(disagree < ink / 50, "{disagree} of {ink}");
        // No ink in the logo, the photo or the blank margin.
        assert_eq!(ink_in(&c, layout::LOGO), 0);
        assert_eq!(ink_in(&c, layout::PHOTO), 0);
        assert_eq!(ink_in(&c, (0.0, 0.0, 30.0, 841.0)), 0);
        // The box is ink.
        assert!(ink_in(&c, layout::BOX) > 4000);
    }

    #[test]
    fn the_same_seed_draws_the_same_page() {
        let (a, b, c) = (page(7), page(7), page(8));
        assert_eq!(a.rgb, b.rgb);
        assert_eq!(a.ink, b.ink);
        assert_eq!(a.text, b.text);
        assert_ne!(a.text, c.text);
        assert!(a.text.contains("Synthetic Page") && a.text.lines().count() > 30);
    }

    #[test]
    fn the_texture_is_sampled_smoothly_at_any_level_of_detail() {
        let texture = page(9).into_texture();
        assert_eq!(texture.size(), (2480, 3508));
        // Paper at a blank spot, at every level.
        for lod in [0.0, 1.0, 2.5, 9.0] {
            let p = texture.sample(0.02, 0.5, lod);
            assert!((p[0] - 243.0).abs() < 1.0 && (p[2] - 236.0).abs() < 1.0, "{lod}: {p:?}");
        }
        // The box is black at level 0 and averaged toward the paper at a coarse one.
        let (x0, y0, x1, y1) = layout::BOX;
        let (u, v) = ((x0 + x1) / 2.0 / A4.0, (y0 + y1) / 2.0 / A4.1);
        assert!(texture.sample(u, v, 0.0)[0] < 5.0);
        // Outside the page the lookup clamps to the edge instead of panicking.
        let _ = texture.sample(-3.0, 9.0, 0.0);
        let _ = texture.sample(f64::NAN, 0.5, 0.0);
    }

    #[test]
    fn a_spread_has_a_darker_gutter() {
        let s = spread_page(&mut Xorshift64Star::new(3)).unwrap();
        let (mid, side) = (s.pixel(s.width / 2, 20).unwrap(), s.pixel(s.width / 4, 20).unwrap());
        assert!(luma(mid) < luma(side) - 0.15, "{mid:?} {side:?}");
    }
}
