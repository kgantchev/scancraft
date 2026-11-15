//! A4: Gaussian blur, direct for σ ≤ 8 px and three boxes above (specification A4). Owner: WP2.
//!
//! Both paths are separable and both mirror the border without repeating the edge pixel (§7.5).
//! Every pass writes whole 64-row bands, so the result does not depend on the thread count.

use crate::RasterError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, Plane, PlaneF32};

/// The smallest σ A4 accepts.
const SIGMA_MIN: f32 = 0.3;
/// The largest σ A4 accepts.
const SIGMA_MAX: f32 = 256.0;
/// Above this σ the blur runs three box filters instead of a direct convolution.
const DIRECT_LIMIT: f32 = 8.0;
/// Rows per band in every parallel pass (§7.6).
const BAND: usize = BAND_ROWS as usize;

/// Mirrors an index into a signal of `n` samples without repeating the edge pixel (§7.5):
/// … c b | a b c d | c b …
fn mirror(i: i64, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let last = n as i64 - 1;
    let period = 2 * last;
    let mut j = i.rem_euclid(period);
    if j > last {
        j = period - j;
    }
    j as usize
}

/// The direct convolution's weights: g(t) = exp(−t² / 2σ²) for |t| ≤ ⌈3σ⌉, normalised to sum
/// to one.
fn direct_weights(sigma: f32) -> Vec<f32> {
    let radius = (3.0f32 * sigma).ceil().max(1.0) as i64;
    let mut weights = Vec::with_capacity((2 * radius + 1) as usize);
    let mut sum = 0f32;
    for t in -radius..=radius {
        let x = t as f32;
        let v = (-(x * x) / (2.0 * sigma * sigma)).exp();
        weights.push(v);
        sum += v;
    }
    for v in &mut weights {
        *v /= sum;
    }
    weights
}

/// The three box widths A4 uses above σ = 8 (Kovesi 2010): `m` boxes of width `w_l`, then
/// `n - m` boxes of width `w_u`.
fn box_widths(sigma: f32) -> (usize, usize, usize) {
    let n = 3f64;
    let s2 = 12.0 * f64::from(sigma) * f64::from(sigma);
    let w_ideal = (s2 / n + 1.0).sqrt();
    let floor = w_ideal.floor() as i64;
    let w_l = if floor % 2 == 0 { floor - 1 } else { floor }.max(1);
    let w_u = w_l + 2;
    let m = ((s2 - n * (w_l * w_l) as f64 - 4.0 * n * w_l as f64 - 3.0 * n) / (-4.0 * w_l as f64 - 4.0)).round();
    let m = (m as i64).clamp(0, n as i64) as usize;
    (w_l as usize, w_u as usize, m)
}

/// The horizontal pass of a direct convolution: every row blurred along x.
fn convolve_rows(src: &[f32], w: usize, h: usize, weights: &[f32], radius: i64) -> Vec<f32> {
    let mut mid = vec![0f32; w * h];
    mid.par_chunks_mut(w * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(w).enumerate() {
            let y = band * BAND + row_index;
            let row_in = src.get(y * w..(y + 1) * w).unwrap_or(&[]);
            for (i, slot) in row_out.iter_mut().enumerate() {
                let mut acc = 0f32;
                for (k, wt) in weights.iter().enumerate() {
                    let t = k as i64 - radius;
                    let sx = mirror(i as i64 + t, w);
                    acc += wt * row_in.get(sx).copied().unwrap_or(0.0);
                }
                *slot = acc;
            }
        }
    });
    mid
}

/// The vertical pass of a direct convolution: every column blurred along y.
fn convolve_columns(src: &[f32], w: usize, h: usize, weights: &[f32], radius: i64) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    out.par_chunks_mut(w * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(w).enumerate() {
            let y = band * BAND + row_index;
            for (i, slot) in row_out.iter_mut().enumerate() {
                let mut acc = 0f32;
                for (k, wt) in weights.iter().enumerate() {
                    let t = k as i64 - radius;
                    let sy = mirror(y as i64 + t, h);
                    acc += wt * src.get(sy * w + i).copied().unwrap_or(0.0);
                }
                *slot = acc;
            }
        }
    });
    out
}

/// The horizontal pass of a box filter: a running sum of `width` samples, mirrored at the
/// border, divided by the width so that the filter keeps the signal's level.
fn box_rows(src: &[f32], w: usize, width: usize, out: &mut [f32]) {
    let half = (width as i64 - 1) / 2;
    out.par_chunks_mut(w * BAND).enumerate().for_each(|(band, chunk)| {
        // Prefix sums over the mirrored extension of one row.
        let mut pre = vec![0f32; w + width];
        for (row_index, row_out) in chunk.chunks_mut(w).enumerate() {
            let y = band * BAND + row_index;
            let row_in = src.get(y * w..(y + 1) * w).unwrap_or(&[]);
            pre[0] = 0.0;
            for t in 0..(w + width - 1) {
                let sx = mirror(t as i64 - half, w);
                let v = row_in.get(sx).copied().unwrap_or(0.0);
                let prev = pre.get(t).copied().unwrap_or(0.0);
                if let Some(slot) = pre.get_mut(t + 1) {
                    *slot = prev + v;
                }
            }
            for (i, slot) in row_out.iter_mut().enumerate() {
                let hi = pre.get(i + width).copied().unwrap_or(0.0);
                let lo = pre.get(i).copied().unwrap_or(0.0);
                *slot = (hi - lo) / width as f32;
            }
        }
    });
}

/// The vertical pass of a box filter: the same running sum down the columns.
///
/// The running sum needs the sum over a window of rows, so the prefix sums are built over the
/// mirrored extension of the whole column first; every output row then costs one subtraction.
fn box_columns(src: &[f32], w: usize, h: usize, width: usize, out: &mut [f32]) {
    let half = (width as i64 - 1) / 2;
    let rows = h + width;
    let mut pre = vec![0f32; rows * w];
    for t in 0..(rows - 1) {
        let sy = mirror(t as i64 - half, h);
        let src_row = src.get(sy * w..(sy + 1) * w).unwrap_or(&[]);
        let (head, tail) = pre.split_at_mut((t + 1) * w);
        let prev = head.get(t * w..(t + 1) * w).unwrap_or(&[]);
        let cur = tail.get_mut(0..w).unwrap_or(&mut []);
        for x in 0..w {
            let p = prev.get(x).copied().unwrap_or(0.0);
            let v = src_row.get(x).copied().unwrap_or(0.0);
            if let Some(slot) = cur.get_mut(x) {
                *slot = p + v;
            }
        }
    }
    out.par_chunks_mut(w * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(w).enumerate() {
            let j = band * BAND + row_index;
            let lo_row = pre.get(j * w..(j + 1) * w).unwrap_or(&[]);
            let hi_row = pre.get((j + width) * w..(j + width + 1) * w).unwrap_or(&[]);
            for (x, slot) in row_out.iter_mut().enumerate() {
                let hi = hi_row.get(x).copied().unwrap_or(0.0);
                let lo = lo_row.get(x).copied().unwrap_or(0.0);
                *slot = (hi - lo) / width as f32;
            }
        }
    });
}

/// The direct convolution: σ ≤ 8.
fn gaussian_direct(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let weights = direct_weights(sigma);
    let radius = (weights.len() as i64 - 1) / 2;
    let mid = convolve_rows(src, w, h, &weights, radius);
    convolve_columns(&mid, w, h, &weights, radius)
}

/// Three box filters in a row: σ > 8.
fn gaussian_boxes(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let (w_l, w_u, m) = box_widths(sigma);
    let mut a = src.to_vec();
    let mut b = vec![0f32; w * h];
    for pass in 0..3usize {
        let width = if pass < m { w_l } else { w_u };
        box_rows(&a, w, width, &mut b);
        box_columns(&b, w, h, width, &mut a);
    }
    a
}

/// Blurs one channel; σ must lie in [0.3, 256].
pub fn gaussian(src: &PlaneF32, sigma: f32) -> Result<PlaneF32, RasterError> {
    if !sigma.is_finite() || sigma < SIGMA_MIN || sigma > SIGMA_MAX {
        return Err(RasterError::BadParameter("sigma must be in [0.3, 256]"));
    }
    let (w, h) = (src.width() as usize, src.height() as usize);
    let data = if sigma <= DIRECT_LIMIT { gaussian_direct(src.data(), w, h, sigma) } else { gaussian_boxes(src.data(), w, h, sigma) };
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

#[cfg(test)]
mod tests {
    use super::{DIRECT_LIMIT, SIGMA_MAX, SIGMA_MIN, gaussian};
    use scancraft_core::image::Plane;

    /// A zero plane with one bright pixel in the middle.
    fn impulse(w: u32, h: u32) -> Plane<f32> {
        let mut p = Plane::new(w, h, 0f32).unwrap();
        p.set(w / 2, h / 2, 1.0);
        p
    }

    /// The sum and the variance of the response around the impulse, in pixel units. The variance
    /// is the one of a single axis: the response is separable, so both axes have it.
    fn response_moments(out: &Plane<f32>, w: u32, h: u32) -> (f64, f64) {
        let cx = f64::from(w) / 2.0;
        let cy = f64::from(h) / 2.0;
        let mut sum = 0f64;
        let mut moment = 0f64;
        for y in 0..h {
            for x in 0..w {
                let v = f64::from(out.get(x, y).unwrap());
                sum += v;
                let dx = f64::from(x) + 0.5 - cx;
                let dy = f64::from(y) + 0.5 - cy;
                moment += v * (dx * dx + dy * dy);
            }
        }
        (sum, moment / sum / 2.0)
    }

    #[test]
    fn direct_response_sums_to_one() {
        for sigma in [0.3f32, 1.5, 4.0] {
            let out = gaussian(&impulse(65, 65), sigma).unwrap();
            let (sum, _) = response_moments(&out, 65, 65);
            assert!((sum - 1.0).abs() < 1e-4, "sigma {sigma}: sum {sum}");
        }
    }

    #[test]
    fn direct_response_variance_is_sigma_squared() {
        // The direct kernel is truncated at ⌈3σ⌉, so the variance matches σ² closely for the
        // middle of the range; at σ = 0.3 the radius is one pixel and no continuous kernel fits.
        for sigma in [1.5f32, 3.0] {
            let out = gaussian(&impulse(65, 65), sigma).unwrap();
            let (_, variance) = response_moments(&out, 65, 65);
            let want = f64::from(sigma) * f64::from(sigma);
            assert!((variance - want).abs() / want < 0.02, "sigma {sigma}: variance {variance}, want {want}");
        }
    }

    #[test]
    fn box_response_sums_to_one() {
        let out = gaussian(&impulse(201, 201), 20.0).unwrap();
        let (sum, _) = response_moments(&out, 201, 201);
        assert!((sum - 1.0).abs() < 1e-3, "sum {sum}");
    }

    #[test]
    fn box_response_variance_is_sigma_squared() {
        for sigma in [9.0f32, 20.0, 64.0] {
            let out = gaussian(&impulse(401, 401), sigma).unwrap();
            let (_, variance) = response_moments(&out, 401, 401);
            let want = f64::from(sigma) * f64::from(sigma);
            assert!((variance - want).abs() / want < 0.05, "sigma {sigma}: variance {variance}, want {want}");
        }
    }

    #[test]
    fn sigma_outside_the_range_is_an_error() {
        let src = Plane::new(8, 8, 0.5f32).unwrap();
        for sigma in [0.29f32, 256.01, f32::NAN, f32::INFINITY, -1.0, 0.0] {
            assert!(gaussian(&src, sigma).is_err(), "sigma {sigma}");
        }
        assert!(gaussian(&src, SIGMA_MIN).is_ok());
        assert!(gaussian(&src, SIGMA_MAX).is_ok());
        assert!(gaussian(&src, DIRECT_LIMIT).is_ok());
    }

    #[test]
    fn constant_image_stays_constant() {
        let src = Plane::new(37, 23, 0.42f32).unwrap();
        for sigma in [0.3f32, 1.5, 8.0, 20.0, 100.0] {
            let out = gaussian(&src, sigma).unwrap();
            for v in out.data() {
                assert!((v - 0.42).abs() < 1e-5, "sigma {sigma}: {v}");
            }
        }
    }

    #[test]
    fn separable_filters_mirror_without_repeating_the_edge_pixel() {
        // A single row: the vertical pass then leaves the row alone, so the border rule of the
        // horizontal pass is visible on its own.
        let src = Plane::from_vec(8, 1, (0..8).map(|i| i as f32).collect()).unwrap();
        let out = gaussian(&src, 1.5).unwrap();
        // The mirrored extension of [0, 1, 2, …] at index −1 is 1, not 0.
        let radius = 5i64;
        let mut expect = 0f64;
        let mut expect_replicated = 0f64;
        let mut sum = 0f64;
        for t in -radius..=radius {
            let x = t as f64;
            let w = (-(x * x) / (2.0 * 1.5 * 1.5)).exp();
            sum += w;
            let mirrored = if t < 0 { -t } else { t };
            expect += w * f64::from(mirrored as u32);
            expect_replicated += w * f64::from(t.max(0) as u32);
        }
        let got = f64::from(out.get(0, 0).unwrap());
        assert!((got - expect / sum).abs() < 1e-5, "got {got}, want {}", expect / sum);
        assert!((got - expect_replicated / sum).abs() > 1e-3, "the border was replicated, not mirrored");
    }

    #[test]
    fn one_thread_and_eight_threads_give_the_same_bytes() {
        let src = Plane::from_vec(131, 97, (0..131u32 * 97).map(|i| ((i * 37) % 251) as f32 / 251.0).collect()).unwrap();
        for sigma in [1.5f32, 20.0] {
            let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| gaussian(&src, sigma).unwrap());
            let many = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(|| gaussian(&src, sigma).unwrap());
            assert_eq!(one.data(), many.data());
        }
    }
}
