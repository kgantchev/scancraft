//! A19: shared helpers (specification A19). Owner: WP2.
//!
//! The minimum and maximum filters repeat the edge pixel at the border (§7.5), unlike the
//! separable filters of A4 and A14, which mirror.

use crate::RasterError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, BitPlane, Plane, PlaneF32};

/// Rows per band in every parallel pass (§7.6).
const BAND: usize = BAND_ROWS as usize;

/// The two extremes the minimum and maximum filters need.
trait Extreme: Copy {
    /// The better of two samples.
    fn pick(a: f32, b: f32) -> f32;
}

/// The maximum.
#[derive(Copy, Clone)]
struct Max;

impl Extreme for Max {
    fn pick(a: f32, b: f32) -> f32 {
        a.max(b)
    }
}

/// The minimum.
#[derive(Copy, Clone)]
struct Min;

impl Extreme for Min {
    fn pick(a: f32, b: f32) -> f32 {
        a.min(b)
    }
}

/// The van Herk filter (van Herk 1992; Gil and Werman 1993): the extreme over a window of `w`
/// samples in three comparisons per sample.
///
/// The signal is padded with `(w − 1) / 2` repeated edge samples on each side and split into
/// blocks of `w` samples. `g` is the running extreme from each block's start and `h` the
/// running extreme to each block's end, so the window starting at padded index `i` holds
/// `pick(h[i], g[i + w − 1])`.
struct VanHerk {
    width: usize,
    len: usize,
    pad: Vec<f32>,
    g: Vec<f32>,
    h: Vec<f32>,
}

impl VanHerk {
    /// A filter for a signal of `n` samples and a window of `width` samples.
    fn new(n: usize, width: usize) -> Self {
        let p = (width - 1) / 2;
        let len = n.saturating_add(2 * p).div_ceil(width) * width;
        Self { width, len, pad: vec![0.0; len], g: vec![0.0; len], h: vec![0.0; len] }
    }

    /// The extreme over a window of `width` samples of `signal`, the edge repeated at the
    /// borders. `signal` and `out` have the same length.
    fn run<E: Extreme>(&mut self, signal: &[f32], out: &mut [f32]) {
        let n = signal.len();
        if n == 0 {
            return;
        }
        let p = (self.width - 1) / 2;
        let last = n as i64 - 1;
        for (j, slot) in self.pad.iter_mut().enumerate() {
            let idx = (j as i64 - p as i64).clamp(0, last) as usize;
            *slot = signal.get(idx).copied().unwrap_or(0.0);
        }
        for j in 0..self.len {
            let v = self.pad[j];
            self.g[j] = if j % self.width == 0 { v } else { E::pick(self.g[j - 1], v) };
        }
        for j in (0..self.len).rev() {
            let v = self.pad[j];
            self.h[j] = if j % self.width == self.width - 1 { v } else { E::pick(v, self.h[j + 1]) };
        }
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = E::pick(self.h[i], self.g[i + self.width - 1]);
        }
    }
}

/// Copies a plane of `w` columns and `h` rows into one of `h` columns and `w` rows, so that
/// the samples of a column become contiguous.
fn transpose(src: &[f32], w: usize, h: usize, dst: &mut [f32]) {
    const TILE: usize = 32;
    for x0 in (0..w).step_by(TILE) {
        for y0 in (0..h).step_by(TILE) {
            for x in x0..(x0 + TILE).min(w) {
                for y in y0..(y0 + TILE).min(h) {
                    let v = src.get(y * w + x).copied().unwrap_or(0.0);
                    if let Some(slot) = dst.get_mut(x * h + y) {
                        *slot = v;
                    }
                }
            }
        }
    }
}

/// The minimum or the maximum over a square window of odd side `w`, with the edge repeated at
/// the borders (§7.5): a horizontal pass, then a vertical one.
fn square_filter<E: Extreme>(src: &PlaneF32, width: u32) -> Result<PlaneF32, RasterError> {
    let w = width as usize;
    if width == 0 || width.is_multiple_of(2) {
        return Err(RasterError::BadParameter("filter window must be odd and at least 1"));
    }
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    // The horizontal pass.
    let mut mid = vec![0f32; pw * ph];
    mid.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        let mut filter = VanHerk::new(pw, w);
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = band * BAND + row_index;
            let row_in = src.data().get(y * pw..(y + 1) * pw).unwrap_or(&[]);
            filter.run::<E>(row_in, row_out);
        }
    });
    // The vertical pass, run on the transposed plane so that a column's samples are contiguous.
    let mut t = vec![0f32; pw * ph];
    transpose(&mid, pw, ph, &mut t);
    let mut v = vec![0f32; pw * ph];
    v.par_chunks_mut(ph * BAND).enumerate().for_each(|(band, chunk)| {
        let mut filter = VanHerk::new(ph, w);
        for (row_index, row_out) in chunk.chunks_mut(ph).enumerate() {
            let x = band * BAND + row_index;
            let row_in = t.get(x * ph..(x + 1) * ph).unwrap_or(&[]);
            filter.run::<E>(row_in, row_out);
        }
    });
    let mut out = vec![0f32; pw * ph];
    transpose(&v, ph, pw, &mut out);
    Ok(Plane::from_vec(src.width(), src.height(), out)?)
}

/// Minimum filter over a square of odd side `w` (van Herk; Gil and Werman).
pub fn min_filter(src: &PlaneF32, w: u32) -> Result<PlaneF32, RasterError> {
    square_filter::<Min>(src, w)
}

/// Maximum filter over a square of odd side `w`.
pub fn max_filter(src: &PlaneF32, w: u32) -> Result<PlaneF32, RasterError> {
    square_filter::<Max>(src, w)
}

/// The two diagonal directions of the octagon.
enum Diagonal {
    /// 45°: down and to the right.
    DownRight,
    /// 135°: down and to the left.
    DownLeft,
}

/// The octagon's half-lengths: `a = round(0.4142 · r)` along 0° and 90°, `b = round(0.2929 · r)`
/// along 45° and 135°.
fn octagon_half_lengths(r: u32) -> (i64, i64) {
    let a = (0.4142 * f64::from(r)).round() as i64;
    let b = (0.2929 * f64::from(r)).round() as i64;
    (a, b)
}

/// The extreme over a window of `width` samples along every row.
fn horizontal_filter<E: Extreme>(src: &[f32], pw: usize, width: usize, out: &mut [f32]) {
    out.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        let mut filter = VanHerk::new(pw, width);
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = band * BAND + row_index;
            let row_in = src.get(y * pw..(y + 1) * pw).unwrap_or(&[]);
            filter.run::<E>(row_in, row_out);
        }
    });
}

/// The extreme over a window of `width` samples along every column, run on the transposed plane
/// so that a column's samples are contiguous.
fn vertical_filter<E: Extreme>(src: &[f32], pw: usize, ph: usize, width: usize, out: &mut [f32]) {
    let mut t = vec![0f32; pw * ph];
    transpose(src, pw, ph, &mut t);
    let mut v = vec![0f32; pw * ph];
    v.par_chunks_mut(ph * BAND).enumerate().for_each(|(band, chunk)| {
        let mut filter = VanHerk::new(ph, width);
        for (row_index, row_out) in chunk.chunks_mut(ph).enumerate() {
            let x = band * BAND + row_index;
            let row_in = t.get(x * ph..(x + 1) * ph).unwrap_or(&[]);
            filter.run::<E>(row_in, row_out);
        }
    });
    transpose(&v, ph, pw, out);
}

/// The extreme over the `2 · half + 1` samples along the diagonal through each pixel. The ends
/// of the diagonal are repeated, so the window follows the line rather than the image border.
fn diagonal_filter<E: Extreme>(src: &[f32], pw: usize, ph: usize, direction: Diagonal, half: i64, out: &mut [f32]) {
    let (w, h) = (pw as i64, ph as i64);
    out.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = (band * BAND + row_index) as i64;
            for (x, slot) in row_out.iter_mut().enumerate() {
                let x = x as i64;
                // The line through (x, y): where it starts, how long it is and the pixel's place
                // on it.
                let (x0, y0, len, k) = match direction {
                    Diagonal::DownRight => {
                        let d = x - y;
                        let x0 = d.max(0);
                        let y0 = (-d).max(0);
                        (x0, y0, (w - x0).min(h - y0), x - x0)
                    }
                    Diagonal::DownLeft => {
                        let s = x + y;
                        let x0 = (s - (h - 1)).max(0);
                        let y0 = s - x0;
                        (x0, y0, (w - x0).min(y0 + 1), x - x0)
                    }
                };
                let mut best = 0f32;
                let mut first = true;
                for t in -half..=half {
                    let j = (k + t).clamp(0, len - 1);
                    let sx = x0 + j;
                    let sy = match direction {
                        Diagonal::DownRight => y0 + j,
                        Diagonal::DownLeft => y0 - j,
                    };
                    let v = src.get((sy * w + sx) as usize).copied().unwrap_or(0.0);
                    best = if first { v } else { E::pick(best, v) };
                    first = false;
                }
                *slot = best;
            }
        }
    });
}

/// The four passes of the octagon: along 0° and 90° with half-length `a`, then along 45° and
/// 135° with half-length `b`.
fn octagon_passes<E: Extreme>(src: &[f32], pw: usize, ph: usize, a: i64, b: i64, out: &mut [f32]) {
    let n = pw * ph;
    let mut cur = vec![0f32; n];
    let mut tmp = vec![0f32; n];
    if a > 0 {
        let width = (2 * a + 1) as usize;
        horizontal_filter::<E>(src, pw, width, &mut cur);
        vertical_filter::<E>(&cur, pw, ph, width, &mut tmp);
    } else {
        tmp.copy_from_slice(src);
    }
    if b > 0 {
        diagonal_filter::<E>(&tmp, pw, ph, Diagonal::DownRight, b, &mut cur);
        diagonal_filter::<E>(&cur, pw, ph, Diagonal::DownLeft, b, &mut tmp);
    } else {
        cur.copy_from_slice(&tmp);
    }
    out.copy_from_slice(&tmp);
}

/// Closing with an octagon of radius `r`, standing in for a disk.
///
/// The closing is the four maximum passes followed by the four minimum passes, so a dark mark
/// thinner than the element disappears while a wide one stays.
pub fn octagon_close(src: &PlaneF32, r: u32) -> Result<PlaneF32, RasterError> {
    let (a, b) = octagon_half_lengths(r);
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    let mut maxed = vec![0f32; pw * ph];
    octagon_passes::<Max>(src.data(), pw, ph, a, b, &mut maxed);
    let mut out = vec![0f32; pw * ph];
    octagon_passes::<Min>(&maxed, pw, ph, a, b, &mut out);
    Ok(Plane::from_vec(src.width(), src.height(), out)?)
}

/// The median of nine samples: the fifth smallest.
fn median_of_nine(values: [f32; 9]) -> f32 {
    // The five smallest seen so far, in order. A new sample smaller than the largest of them
    // goes in and pushes the largest out; the largest of the five is the median.
    let mut kept = [f32::INFINITY; 5];
    for v in values {
        if v < kept[4] {
            let mut i = 4;
            while i > 0 && kept[i - 1] > v {
                kept[i] = kept[i - 1];
                i -= 1;
            }
            kept[i] = v;
        }
    }
    kept[4]
}

/// 3 × 3 median (Paeth's sorting network).
pub fn median3(src: &PlaneF32) -> Result<PlaneF32, RasterError> {
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    let mut out = vec![0f32; pw * ph];
    out.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = band * BAND + row_index;
            for (x, slot) in row_out.iter_mut().enumerate() {
                let mut values = [0f32; 9];
                for (k, value) in values.iter_mut().enumerate() {
                    // The edge is repeated at the borders: a median is a rank filter, not a
                    // separable one, so it does not follow the mirror rule.
                    let sx = (x + k % 3).saturating_sub(1).min(pw - 1) as u32;
                    let sy = (y + k / 3).saturating_sub(1).min(ph - 1) as u32;
                    *value = src.get(sx, sy).unwrap_or(0.0);
                }
                *slot = median_of_nine(values);
            }
        }
    });
    Ok(Plane::from_vec(src.width(), src.height(), out)?)
}

/// Unsharp mask: Y + amount · (Y − G_σ ∗ Y) where the difference exceeds `threshold`.
pub fn unsharp(src: &PlaneF32, sigma: f32, amount: f32, threshold: f32) -> Result<PlaneF32, RasterError> {
    if !amount.is_finite() || !threshold.is_finite() {
        return Err(RasterError::BadParameter("amount and threshold must be finite"));
    }
    // A4 checks σ and rejects a value outside [0.3, 256].
    let blurred = crate::blur::gaussian(src, sigma)?;
    let mut data = Vec::with_capacity(src.data().len());
    for (y, b) in src.data().iter().zip(blurred.data()) {
        let difference = y - b;
        let value = if difference.abs() > threshold { y + amount * difference } else { *y };
        data.push(value);
    }
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

/// 8-connected components of the set bits: a label per pixel (0 for unset) and the count.
pub fn connected_components(bits: &BitPlane) -> Result<(Plane<u32>, u32), RasterError> {
    let (w, h) = (bits.width(), bits.height());
    let pixels = u64::from(w) * u64::from(h);
    if pixels >= u64::from(u32::MAX) {
        return Err(RasterError::BadParameter("the image has too many pixels to label"));
    }
    // Ink is 255, paper is 0: the labelling needs two distinct colours.
    let mask = image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(w, h, vec![0u8; pixels as usize]);
    let Some(mut mask) = mask else {
        return Err(RasterError::BadParameter("the image has too many pixels to label"));
    };
    for y in 0..h {
        for x in 0..w {
            if bits.get(x, y) == Some(true) {
                mask.get_pixel_mut(x, y)[0] = 255;
            }
        }
    }
    let labels = imageproc::region_labelling::connected_components(&mask, imageproc::region_labelling::Connectivity::Eight, image::Luma([0u8]));
    let mut data = Vec::with_capacity(pixels as usize);
    let mut count = 0u32;
    for pixel in labels.pixels() {
        let label = pixel[0];
        count = count.max(label);
        data.push(label);
    }
    Ok((Plane::from_vec(w, h, data)?, count))
}

/// Normalized convolution (Knutsson and Westin): (G_σ ∗ (c · I)) / (G_σ ∗ c), with the local
/// confidence G_σ ∗ c returned too.
pub fn normalized_convolution(values: &PlaneF32, confidence: &BitPlane, sigma: f32) -> Result<(PlaneF32, PlaneF32), RasterError> {
    let (w, h) = (values.width(), values.height());
    if confidence.width() != w || confidence.height() != h {
        return Err(RasterError::BadParameter("the confidence mask must have the same size as the values"));
    }
    // c is the mask as 0 or 1; both convolutions are A4's Gaussian, so the mirror rule applies.
    let mut mask = PlaneF32::new(w, h, 0f32)?;
    let mut weighted = PlaneF32::new(w, h, 0f32)?;
    for y in 0..h {
        for x in 0..w {
            let c = if confidence.get(x, y) == Some(true) { 1f32 } else { 0f32 };
            let v = values.get(x, y).unwrap_or(0.0);
            mask.set(x, y, c);
            weighted.set(x, y, c * v);
        }
    }
    let numerator = crate::blur::gaussian(&weighted, sigma)?;
    let denominator = crate::blur::gaussian(&mask, sigma)?;
    let mut out = Vec::with_capacity(numerator.data().len());
    for (n, d) in numerator.data().iter().zip(denominator.data()) {
        out.push(if *d > 0f32 { n / d } else { 0f32 });
    }
    Ok((Plane::from_vec(w, h, out)?, denominator))
}

#[cfg(test)]
mod tests {
    use super::{connected_components, max_filter, median3, min_filter, normalized_convolution, unsharp};
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::image::{BitPlane, Plane};

    fn random_plane(w: u32, h: u32, seed: u64) -> Plane<f32> {
        let mut r = Xorshift64Star::new(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| r.next_f64() as f32).collect()).unwrap()
    }

    /// The window extreme of every pixel, computed the slow way, with the edge repeated.
    fn naive_extreme(src: &Plane<f32>, w: u32, want_max: bool) -> Vec<f32> {
        let (pw, ph) = (src.width() as usize, src.height() as usize);
        let p = (w as usize - 1) / 2;
        let mut out = vec![0f32; pw * ph];
        for y in 0..ph {
            for x in 0..pw {
                let mut best = if want_max { f32::NEG_INFINITY } else { f32::INFINITY };
                for dy in 0..w as usize {
                    for dx in 0..w as usize {
                        let sx = (x + dx).saturating_sub(p).min(pw - 1);
                        let sy = (y + dy).saturating_sub(p).min(ph - 1);
                        let v = src.get(sx as u32, sy as u32).unwrap();
                        best = if want_max { best.max(v) } else { best.min(v) };
                    }
                }
                out[y * pw + x] = best;
            }
        }
        out
    }

    #[test]
    fn min_and_max_filters_agree_with_a_naive_window() {
        let src = random_plane(37, 23, 7);
        for w in [1u32, 3, 5, 9, 15] {
            let min = min_filter(&src, w).unwrap();
            let max = max_filter(&src, w).unwrap();
            assert_eq!(min.data(), naive_extreme(&src, w, false).as_slice());
            assert_eq!(max.data(), naive_extreme(&src, w, true).as_slice());
        }
    }

    #[test]
    fn min_and_max_filters_repeat_the_edge_pixel() {
        // The window at the border holds the edge sample again, not its neighbour: a signal
        // that falls away from the edge gives a different answer under the two rules.
        let src = Plane::from_vec(5, 1, vec![9.0f32, 4.0, 1.0, 0.0, 0.0]).unwrap();
        let min = min_filter(&src, 3).unwrap();
        // The window at x = 0 is {9, 9, 4} under the repeated edge and {4, 9, 4} under the
        // mirror; the minimum is 4 either way, but the maximum tells them apart at x = 0.
        assert_eq!(min.get(0, 0), Some(4.0));
        assert_eq!(min.get(4, 0), Some(0.0));
        let max = max_filter(&src, 3).unwrap();
        assert_eq!(max.get(0, 0), Some(9.0));
        assert_eq!(max.get(4, 0), Some(0.0));
    }

    #[test]
    fn an_even_window_is_an_error() {
        let src = random_plane(8, 8, 1);
        assert!(min_filter(&src, 0).is_err());
        assert!(min_filter(&src, 4).is_err());
        assert!(max_filter(&src, 0).is_err());
        assert!(max_filter(&src, 4).is_err());
        assert!(min_filter(&src, 1).is_ok());
    }

    #[test]
    fn octagon_half_lengths_are_the_specified_constants() {
        assert_eq!(super::octagon_half_lengths(0), (0, 0));
        assert_eq!(super::octagon_half_lengths(1), (0, 0));
        assert_eq!(super::octagon_half_lengths(2), (1, 1));
        assert_eq!(super::octagon_half_lengths(4), (2, 1));
        assert_eq!(super::octagon_half_lengths(10), (4, 3));
        assert_eq!(super::octagon_half_lengths(24), (10, 7));
        assert_eq!(super::octagon_half_lengths(120), (50, 35));
    }

    #[test]
    fn octagon_closing_removes_thin_dark_marks() {
        // A page with a one-pixel dark line and a wide dark box.
        let mut src = Plane::new(64, 64, 1.0f32).unwrap();
        for y in 0..64u32 {
            src.set(32, y, 0.0);
        }
        for y in 20..44u32 {
            for x in 8..24u32 {
                src.set(x, y, 0.0);
            }
        }
        let out = super::octagon_close(&src, 3).unwrap();
        // The thin line is gone.
        for y in 0..64u32 {
            assert!(out.get(32, y).unwrap() > 0.9, "line at {y}: {}", out.get(32, y).unwrap());
        }
        // The wide box stays.
        for y in 26..38u32 {
            for x in 12..20u32 {
                assert!(out.get(x, y).unwrap() < 0.1, "box at ({x},{y}): {}", out.get(x, y).unwrap());
            }
        }
        // A radius of zero changes nothing.
        let same = super::octagon_close(&src, 0).unwrap();
        assert_eq!(same.data(), src.data());
    }

    #[test]
    fn one_thread_and_eight_threads_give_the_same_bytes() {
        let src = random_plane(131, 97, 11);
        let mut confidence = BitPlane::new(131, 97).unwrap();
        for y in 0..97u32 {
            for x in 0..131u32 {
                confidence.set(x, y, (x + y) % 3 == 0);
            }
        }
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| {
            (
                min_filter(&src, 9).unwrap(),
                max_filter(&src, 9).unwrap(),
                super::octagon_close(&src, 4).unwrap(),
                median3(&src).unwrap(),
                unsharp(&src, 1.5, 0.5, 0.01).unwrap(),
                normalized_convolution(&src, &confidence, 2.0).unwrap(),
            )
        });
        let many = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(|| {
            (
                min_filter(&src, 9).unwrap(),
                max_filter(&src, 9).unwrap(),
                super::octagon_close(&src, 4).unwrap(),
                median3(&src).unwrap(),
                unsharp(&src, 1.5, 0.5, 0.01).unwrap(),
                normalized_convolution(&src, &confidence, 2.0).unwrap(),
            )
        });
        assert_eq!(one.0.data(), many.0.data());
        assert_eq!(one.1.data(), many.1.data());
        assert_eq!(one.2.data(), many.2.data());
        assert_eq!(one.3.data(), many.3.data());
        assert_eq!(one.4.data(), many.4.data());
        assert_eq!(one.5.0.data(), many.5.0.data());
        assert_eq!(one.5.1.data(), many.5.1.data());
    }

    #[test]
    fn median3_equals_the_median_of_the_nine_samples() {
        let cases: [[f32; 9]; 6] = [
            [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
            [9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0],
            [0.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0],
            [5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 9.0],
            [5.0, 1.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0],
        ];
        for case in cases {
            let mut want = case;
            want.sort_by(|a, b| a.total_cmp(b));
            assert_eq!(super::median_of_nine(case), want[4]);
        }
        // The same on a plane, against a sorted reference.
        let src = random_plane(19, 13, 3);
        let out = median3(&src).unwrap();
        for y in 0..13u32 {
            for x in 0..19u32 {
                let mut values = Vec::new();
                for dy in 0..3u32 {
                    for dx in 0..3u32 {
                        let sx = (x + dx).saturating_sub(1).min(18);
                        let sy = (y + dy).saturating_sub(1).min(12);
                        values.push(src.get(sx, sy).unwrap());
                    }
                }
                values.sort_by(|a, b| a.total_cmp(b));
                assert_eq!(out.get(x, y), Some(values[4]));
            }
        }
    }

    #[test]
    fn unsharp_leaves_values_below_the_threshold_alone() {
        let src = Plane::new(32, 32, 0.5f32).unwrap();
        let out = unsharp(&src, 1.0, 0.5, 0.01).unwrap();
        for v in out.data() {
            assert!((v - 0.5).abs() < 1e-6);
        }
        assert!(unsharp(&src, 0.0, 0.5, 0.01).is_err());
        assert!(unsharp(&src, f32::NAN, 0.5, 0.01).is_err());
        assert!(unsharp(&src, 1.0, f32::NAN, 0.01).is_err());
    }

    #[test]
    fn unsharp_sharpens_a_step_edge() {
        // A step from 0 to 1 across the middle of a grey-free plane.
        let mut src = Plane::new(64, 8, 0.0f32).unwrap();
        for y in 0..8u32 {
            for x in 32..64u32 {
                src.set(x, y, 1.0);
            }
        }
        let out = unsharp(&src, 0.8, 0.5, 0.01).unwrap();
        // The blurred step is smooth, so the sharp side gains and the flat side loses nothing.
        assert!(out.get(31, 4).unwrap() < 0.05, "{}", out.get(31, 4).unwrap());
        assert!(out.get(32, 4).unwrap() > 0.5, "{}", out.get(32, 4).unwrap());
        // The overshoot at the edge grows with the amount.
        let strong = unsharp(&src, 0.8, 1.0, 0.01).unwrap();
        assert!(strong.get(33, 4).unwrap() > out.get(33, 4).unwrap());
    }

    #[test]
    fn connected_components_labels_the_set_bits_in_8_connectivity() {
        let mut bits = BitPlane::new(5, 5).unwrap();
        // A diagonal pair is one component with 8-connectivity.
        bits.set(1, 1, true);
        bits.set(2, 2, true);
        // A separate pair.
        bits.set(4, 0, true);
        bits.set(4, 1, true);
        let (labels, count) = connected_components(&bits).unwrap();
        assert_eq!(count, 2);
        // The labels follow the raster-scan order of the components' first pixel.
        assert_eq!(labels.get(4, 0), Some(1));
        assert_eq!(labels.get(4, 1), Some(1));
        assert_eq!(labels.get(1, 1), Some(2));
        assert_eq!(labels.get(2, 2), Some(2));
        assert_eq!(labels.get(0, 0), Some(0));
    }

    #[test]
    fn connected_components_counts_the_components() {
        let mut bits = BitPlane::new(9, 9).unwrap();
        assert_eq!(connected_components(&bits).unwrap().1, 0);
        for (i, j) in [(0, 0), (4, 4), (8, 8), (8, 0)] {
            bits.set(i, j, true);
        }
        assert_eq!(connected_components(&bits).unwrap().1, 4);
    }

    #[test]
    fn normalized_convolution_divides_by_the_local_confidence() {
        let values = random_plane(24, 24, 5);
        let mut confidence = BitPlane::new(24, 24).unwrap();
        for y in 0..24u32 {
            for x in 0..24u32 {
                confidence.set(x, y, x < 12);
            }
        }
        let (out, local) = normalized_convolution(&values, &confidence, 2.0).unwrap();
        // Where the whole neighbourhood is confident, the result is the blurred input.
        let blurred = crate::blur::gaussian(&values, 2.0).unwrap();
        assert!((out.get(0, 0).unwrap() - blurred.get(0, 0).unwrap()).abs() < 1e-5);
        // The local confidence is 1 where the mask is full.
        assert!((local.get(0, 0).unwrap() - 1.0).abs() < 1e-5);
        assert!(local.get(23, 0).unwrap() < 0.5);
    }

    #[test]
    fn normalized_convolution_guards_against_zero_confidence() {
        let values = random_plane(16, 16, 6);
        let confidence = BitPlane::new(16, 16).unwrap();
        let (out, local) = normalized_convolution(&values, &confidence, 1.0).unwrap();
        for v in out.data() {
            assert!(v.is_finite());
            assert_eq!(*v, 0.0);
        }
        for v in local.data() {
            assert_eq!(*v, 0.0);
        }
    }

    #[test]
    fn normalized_convolution_needs_a_mask_of_the_same_size() {
        let values = random_plane(8, 8, 9);
        let confidence = BitPlane::new(8, 9).unwrap();
        assert!(normalized_convolution(&values, &confidence, 1.0).is_err());
    }
}
