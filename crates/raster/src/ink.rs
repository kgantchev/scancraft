//! A18 steps 1–5, the lightest/darkest midpoint rule, shared by A18 and A26. Owner: WP2.
//!
//! Each pixel is compared with the midpoint between the lightest value nearby (the paper) and the
//! darkest value very nearby (the stroke). Where those two are close, there is no ink edge to judge
//! and the pixel is paper, unless it is very dark. The photo regions (step 6) and the despeckling
//! (step 7) belong to A18 itself and are not done here.

use crate::RasterError;
use crate::blur::gaussian;
use crate::helpers::{max_filter, min_filter};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, BitPlane, PlaneF32};

/// The densities the rule is defined for; outside them the window sizes stop making sense.
const DPI_MIN: f64 = 50.0;
const DPI_MAX: f64 = 1200.0;
/// A pixel darker than this is ink whatever its surroundings (step 5).
const SOLID: f32 = 0.25;

/// The smallest odd window at least `n` wide (and at least 1).
fn odd(n: f64) -> u32 {
    let n = n.round().max(1.0) as u32;
    if n % 2 == 1 { n } else { n + 1 }
}

/// The window sides of A18 at `dpi`: the paper window w_p and the stroke window w_i.
pub fn windows(dpi: f64) -> (u32, u32) {
    (odd(31.0 * dpi / 300.0), odd(9.0 * dpi / 300.0))
}

/// The gate g of A18 step 4 for a strength of 0..=100.
pub fn gate(strength: u8) -> f32 {
    let k = f32::from(strength.min(100)) / 100.0;
    0.04 + 0.16 * (1.0 - k)
}

/// Ink where Y < 0.25, or where M − m ≥ g and Y ≤ (M + m)/2 (A18 step 5), at `dpi` pixels per
/// inch and `strength` 0..=100. `luma` is the page's luma at that density, from A16.
pub fn midpoint_ink(luma: &PlaneF32, dpi: f64, strength: u8) -> Result<BitPlane, RasterError> {
    if !dpi.is_finite() || !(DPI_MIN..=DPI_MAX).contains(&dpi) {
        return Err(RasterError::BadParameter("dpi must be in [50, 1200]"));
    }
    if strength > 100 {
        return Err(RasterError::BadParameter("strength must be in 0..=100"));
    }
    let (w, h) = (luma.width(), luma.height());
    // Step 1: smooth, σ = 0.7 px at 300 DPI.
    let sigma = (0.7 * dpi / 300.0).max(0.3) as f32;
    let smooth = gaussian(luma, sigma)?;
    // Steps 2 and 3: the paper level and the stroke level.
    let (wp, wi) = windows(dpi);
    let paper = max_filter(&smooth, wp)?;
    let stroke = min_filter(&smooth, wi)?;
    // Steps 4 and 5.
    let g = gate(strength);
    let width = w as usize;
    let stride = width.div_ceil(8);
    let mut bits = vec![0u8; stride * h as usize];
    bits.par_chunks_mut(stride * BAND_ROWS as usize).enumerate().for_each(|(band, chunk)| {
        for (r, row) in chunk.chunks_mut(stride).enumerate() {
            let y = band * BAND_ROWS as usize + r;
            let (Some(ys), Some(ms), Some(ns)) = (
                smooth.data().get(y * width..(y + 1) * width),
                paper.data().get(y * width..(y + 1) * width),
                stroke.data().get(y * width..(y + 1) * width),
            ) else {
                continue;
            };
            for (x, ((&v, &hi), &lo)) in ys.iter().zip(ms).zip(ns).enumerate() {
                let ink = v < SOLID || (hi - lo >= g && v <= 0.5 * (hi + lo));
                if ink && let Some(byte) = row.get_mut(x / 8) {
                    *byte |= 0x80u8 >> (x % 8);
                }
            }
        }
    });
    let mut out = BitPlane::new(w, h)?;
    for y in 0..h {
        for x in 0..w {
            if bits.get(y as usize * stride + x as usize / 8).is_some_and(|b| b & (0x80u8 >> (x % 8)) != 0) {
                out.set(x, y, true);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{gate, midpoint_ink, windows};
    use crate::blur::gaussian;
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::image::{BitPlane, Plane, PlaneF32};

    const PAPER: f32 = 0.92;
    const INK: f32 = 0.10;

    /// A page drawn in millimetres at 8 times the output density, then reduced by area, so strokes
    /// have anti-aliased edges. `truth` is the ink the page was drawn with, where more than half
    /// of a pixel is covered.
    struct Page {
        dpi: f64,
        width: u32,
        height: u32,
        /// Ink coverage in 0..=1 per output pixel.
        coverage: Vec<f32>,
    }

    const SUPER: u32 = 8;

    impl Page {
        fn new(dpi: f64, mm_w: f64, mm_h: f64) -> Page {
            let (width, height) = ((mm_w * dpi / 25.4).round() as u32, (mm_h * dpi / 25.4).round() as u32);
            Page { dpi, width, height, coverage: vec![0.0; (width * height) as usize] }
        }

        /// Paints a rectangle given in millimetres, with exact area coverage.
        fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
            let px = self.dpi / 25.4;
            let (x0, y0, x1, y1) = (x0 * px, y0 * px, x1 * px, y1 * px);
            for y in (y0.floor().max(0.0) as u32)..(y1.ceil().max(0.0) as u32).min(self.height) {
                for x in (x0.floor().max(0.0) as u32)..(x1.ceil().max(0.0) as u32).min(self.width) {
                    let ox = (f64::from(x + 1).min(x1) - f64::from(x).max(x0)).max(0.0);
                    let oy = (f64::from(y + 1).min(y1) - f64::from(y).max(y0)).max(0.0);
                    let slot = &mut self.coverage[(y * self.width + x) as usize];
                    *slot = (*slot + (ox * oy) as f32).min(1.0);
                }
            }
        }

        /// Paints a disk with centre and diameter in millimetres, by supersampling.
        fn disk(&mut self, cx: f64, cy: f64, diameter: f64) {
            let px = self.dpi / 25.4;
            let (cx, cy, r) = (cx * px, cy * px, diameter * px / 2.0);
            for y in ((cy - r - 1.0).floor().max(0.0) as u32)..((cy + r + 1.0).ceil().max(0.0) as u32).min(self.height) {
                for x in ((cx - r - 1.0).floor().max(0.0) as u32)..((cx + r + 1.0).ceil().max(0.0) as u32).min(self.width) {
                    let mut inside = 0;
                    for sy in 0..SUPER {
                        for sx in 0..SUPER {
                            let (fx, fy) =
                                (f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SUPER), f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SUPER));
                            if (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r {
                                inside += 1;
                            }
                        }
                    }
                    let slot = &mut self.coverage[(y * self.width + x) as usize];
                    *slot = (*slot + inside as f32 / (SUPER * SUPER) as f32).min(1.0);
                }
            }
        }

        /// The photographed page: paper and ink by coverage, the optics' blur, and sensor noise.
        fn luma(&self, ink_level: f32, noise: f32, seed: u64) -> PlaneF32 {
            let flat: Vec<f32> = self.coverage.iter().map(|&c| PAPER + c * (ink_level - PAPER)).collect();
            let plane = Plane::from_vec(self.width, self.height, flat).unwrap();
            let blurred = gaussian(&plane, (0.8 * self.dpi / 300.0).max(0.3) as f32).unwrap();
            let mut rng = Xorshift64Star::new(seed);
            let data = blurred
                .data()
                .iter()
                .map(|&v| {
                    let (a, b) = (rng.next_f64().max(1e-12), rng.next_f64());
                    let g = (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos();
                    (v + noise * g as f32).clamp(0.0, 1.0)
                })
                .collect();
            Plane::from_vec(self.width, self.height, data).unwrap()
        }

        fn truth(&self, x: u32, y: u32) -> bool {
            self.coverage[(y * self.width + x) as usize] > 0.5
        }
    }

    /// Lines of letter-like shapes: stems, bars and bowls, 0.30 mm strokes, 2.4 mm tall.
    fn text_page(dpi: f64, seed: u64) -> Page {
        let mut page = Page::new(dpi, 60.0, 30.0);
        let mut rng = Xorshift64Star::new(seed);
        let stroke = 0.30;
        let height = 2.4;
        let mut y = 4.0;
        while y + height < 28.0 {
            let mut x = 3.0;
            while x < 56.0 {
                let glyphs = 3 + rng.below(4);
                for _ in 0..glyphs {
                    let width = 1.5;
                    let parts = rng.below(32);
                    // Left stem, right stem, top bar, middle bar, bottom bar; at least two.
                    let parts = if parts.count_ones() < 2 { parts | 0b00101 } else { parts };
                    if parts & 1 != 0 {
                        page.rect(x, y, x + stroke, y + height);
                    }
                    if parts & 2 != 0 {
                        page.rect(x + width - stroke, y, x + width, y + height);
                    }
                    if parts & 4 != 0 {
                        page.rect(x, y, x + width, y + stroke);
                    }
                    if parts & 8 != 0 {
                        page.rect(x, y + (height - stroke) / 2.0, x + width, y + (height + stroke) / 2.0);
                    }
                    if parts & 16 != 0 {
                        page.rect(x, y + height - stroke, x + width, y + height);
                    }
                    x += width + 0.5;
                }
                x += 2.0;
            }
            y += 5.0;
        }
        page
    }

    /// Precision, recall and F-measure of `ink` against the page's truth, over the whole page.
    fn scores(page: &Page, ink: &BitPlane) -> (f64, f64, f64) {
        let (mut tp, mut fp, mut fneg) = (0u64, 0u64, 0u64);
        for y in 0..page.height {
            for x in 0..page.width {
                match (page.truth(x, y), ink.get(x, y).unwrap()) {
                    (true, true) => tp += 1,
                    (false, true) => fp += 1,
                    (true, false) => fneg += 1,
                    _ => {}
                }
            }
        }
        let precision = tp as f64 / (tp + fp).max(1) as f64;
        let recall = tp as f64 / (tp + fneg).max(1) as f64;
        (precision, recall, 2.0 * precision * recall / (precision + recall).max(1e-12))
    }

    #[test]
    fn the_windows_and_the_gate_follow_the_specification() {
        assert_eq!(windows(300.0), (31, 9));
        assert_eq!(windows(200.0), (21, 7));
        assert_eq!(windows(150.0), (17, 5));
        assert!((gate(55) - 0.112).abs() < 1e-6);
        assert!((gate(0) - 0.2).abs() < 1e-6 && (gate(100) - 0.04).abs() < 1e-6);
    }

    #[test]
    fn printed_text_is_found_at_300_and_200_dpi() {
        for (dpi, least) in [(300.0, 0.96), (200.0, 0.90)] {
            let mut results = Vec::new();
            for seed in 1..=4 {
                let page = text_page(dpi, seed);
                let ink = midpoint_ink(&page.luma(INK, 3.0 / 255.0, seed), dpi, 55).unwrap();
                results.push(scores(&page, &ink).2);
            }
            let worst = results.iter().cloned().fold(1.0, f64::min);
            assert!(worst >= least, "{dpi} DPI: F-measures {results:?}");
        }
    }

    #[test]
    fn a_pale_pencil_line_and_a_bold_heading_are_both_found() {
        let dpi = 300.0;
        let mut page = text_page(dpi, 9);
        // A pale pencil line away from the print, and a bold heading.
        let mut pencil = Page::new(dpi, 60.0, 30.0);
        pencil.rect(8.0, 28.4, 52.0, 28.7);
        let mut heading = Page::new(dpi, 60.0, 30.0);
        heading.rect(5.0, 0.5, 40.0, 2.5);
        let base = page.luma(INK, 3.0 / 255.0, 9);
        let pale = pencil.luma(PAPER - 0.2, 0.0, 1);
        let bold = heading.luma(0.05, 0.0, 1);
        // Combine: the darkest of the three at each pixel.
        let data: Vec<f32> = base.data().iter().zip(pale.data()).zip(bold.data()).map(|((&a, &b), &c)| a.min(b).min(c)).collect();
        let luma = Plane::from_vec(page.width, page.height, data).unwrap();
        for (i, c) in pencil.coverage.iter().zip(&heading.coverage).map(|(a, b)| a + b).enumerate() {
            page.coverage[i] = (page.coverage[i] + c).min(1.0);
        }
        let ink = midpoint_ink(&luma, dpi, 55).unwrap();
        let recall = |part: &Page| {
            let (mut hit, mut total) = (0u64, 0u64);
            for y in 0..part.height {
                for x in 0..part.width {
                    if part.truth(x, y) {
                        total += 1;
                        hit += u64::from(ink.get(x, y).unwrap());
                    }
                }
            }
            hit as f64 / total.max(1) as f64
        };
        assert!(recall(&pencil) >= 0.85, "pencil recall {}", recall(&pencil));
        assert!(recall(&heading) >= 0.85, "heading recall {}", recall(&heading));
    }

    #[test]
    fn blank_noisy_paper_has_almost_no_ink() {
        let blank = Page::new(300.0, 60.0, 30.0);
        let luma = blank.luma(INK, 6.0 / 255.0, 5);
        let ink = midpoint_ink(&luma, 300.0, 55).unwrap();
        let count = (0..blank.height).flat_map(|y| (0..blank.width).map(move |x| (x, y))).filter(|&(x, y)| ink.get(x, y).unwrap()).count();
        let share = count as f64 / f64::from(blank.width * blank.height);
        assert!(share < 1e-4, "{share}");
    }

    #[test]
    fn a_solid_black_box_stays_black() {
        let mut page = Page::new(300.0, 60.0, 30.0);
        page.rect(10.0, 5.0, 50.0, 25.0);
        let luma = page.luma(0.05, 3.0 / 255.0, 2);
        let ink = midpoint_ink(&luma, 300.0, 55).unwrap();
        let (mut inside, mut total) = (0u64, 0u64);
        for y in 0..page.height {
            for x in 0..page.width {
                if page.truth(x, y) {
                    total += 1;
                    inside += u64::from(ink.get(x, y).unwrap());
                }
            }
        }
        assert!(inside as f64 / total as f64 >= 0.97, "{inside} of {total}");
    }

    #[test]
    fn full_stops_and_the_dots_on_i_survive_at_low_resolution() {
        for dpi in [150.0, 200.0, 300.0] {
            let mut page = Page::new(dpi, 60.0, 30.0);
            let centres: Vec<(f64, f64)> = (0..10).map(|i| (6.0 + 5.0 * f64::from(i), 8.0 + f64::from(i % 3) * 5.0)).collect();
            // Dots 0.4 mm across: a full stop at 10 pt, a little under the dot of an i.
            for &(cx, cy) in &centres {
                page.disk(cx, cy, 0.4);
            }
            let luma = page.luma(INK, 3.0 / 255.0, 3);
            let ink = midpoint_ink(&luma, dpi, 55).unwrap();
            let px = dpi / 25.4;
            for &(cx, cy) in &centres {
                let (x, y) = ((cx * px) as u32, (cy * px) as u32);
                let found = (y.saturating_sub(1)..=y + 1)
                    .flat_map(|yy| (x.saturating_sub(1)..=x + 1).map(move |xx| (xx, yy)))
                    .any(|(xx, yy)| ink.get(xx, yy) == Some(true));
                assert!(found, "{dpi} DPI: the dot at ({cx}, {cy}) mm was lost");
            }
        }
    }

    #[test]
    fn bad_parameters_are_errors() {
        let luma = Plane::new(40, 40, 0.9f32).unwrap();
        assert!(midpoint_ink(&luma, 10.0, 55).is_err());
        assert!(midpoint_ink(&luma, f64::NAN, 55).is_err());
        assert!(midpoint_ink(&luma, 300.0, 101).is_err());
        // A window larger than the image still works.
        assert!(midpoint_ink(&luma, 300.0, 55).is_ok());
    }

    #[test]
    fn same_result_with_one_or_many_threads() {
        let page = text_page(300.0, 7);
        let luma = page.luma(INK, 3.0 / 255.0, 7);
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| midpoint_ink(&luma, 300.0, 55).unwrap());
        assert_eq!(run(1), run(8));
    }
}
