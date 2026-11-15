//! A separable Gaussian blur in fixed 64-row bands (so threads can't change the result), used for
//! the blur damage and the page's contact shadow. Weights are exp(−t²/2σ²) for |t| ≤ ⌈3σ⌉,
//! normalised to sum to 1 (A4's definition), and the edge is mirrored without repeating the edge
//! pixel (§7.5).

use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;
use scancraft_core::image::BAND_ROWS;

fn kernel(sigma: f64) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil().max(1.0) as i64;
    let mut w: Vec<f64> = (-radius..=radius).map(|t| (-((t * t) as f64) / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f64 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    w.into_iter().map(|v| v as f32).collect()
}

/// The index inside 0..n that mirrors `i` without repeating the edge pixel (… c b | a b c d | c b …).
fn mirror(i: i64, n: i64) -> i64 {
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let m = i.rem_euclid(period);
    if m < n { m } else { period - m }
}

/// Blurs `src` (`w × h` pixels of `N` channels) by `sigma`, reading each output pixel from
/// `src` shifted by `shift` pixels; samples beyond the edge are `outside` when `outside` is given,
/// and mirrored otherwise.
pub fn blur<const N: usize>(src: &[[f32; N]], w: usize, h: usize, sigma: f64, shift: (i64, i64), outside: Option<[f32; N]>) -> Vec<[f32; N]> {
    if w == 0 || h == 0 || src.len() != w * h {
        return src.to_vec();
    }
    let k = kernel(sigma.max(0.05));
    let radius = (k.len() / 2) as i64;
    let at = |buf: &[[f32; N]], x: i64, y: i64| -> [f32; N] {
        match outside {
            Some(o) if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 => o,
            _ => buf[mirror(y, h as i64) as usize * w + mirror(x, w as i64) as usize],
        }
    };
    let band = BAND_ROWS as usize * w;
    // Horizontal pass, with the shift's x part.
    let mut mid = vec![[0f32; N]; w * h];
    mid.par_chunks_mut(band).enumerate().for_each(|(b, chunk)| {
        for (r, row) in chunk.chunks_mut(w).enumerate() {
            let y = (b * BAND_ROWS as usize + r) as i64;
            for (x, out) in row.iter_mut().enumerate() {
                let mut acc = [0f32; N];
                for (t, weight) in k.iter().enumerate() {
                    let p = at(src, x as i64 + shift.0 + t as i64 - radius, y);
                    for c in 0..N {
                        acc[c] += weight * p[c];
                    }
                }
                *out = acc;
            }
        }
    });
    // Vertical pass, with the shift's y part.
    let mut out = vec![[0f32; N]; w * h];
    out.par_chunks_mut(band).enumerate().for_each(|(b, chunk)| {
        for (r, row) in chunk.chunks_mut(w).enumerate() {
            let y = (b * BAND_ROWS as usize + r) as i64;
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = [0f32; N];
                for (t, weight) in k.iter().enumerate() {
                    let yy = y + shift.1 + t as i64 - radius;
                    let p = match outside {
                        Some(v) if yy < 0 || yy >= h as i64 => v,
                        _ => mid[mirror(yy, h as i64) as usize * w + x],
                    };
                    for c in 0..N {
                        acc[c] += weight * p[c];
                    }
                }
                *o = acc;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_response_to_a_bright_pixel_sums_to_one_with_variance_sigma_squared() {
        for sigma in [0.6, 1.0, 1.5] {
            let (w, h) = (41, 41);
            let mut img = vec![[0f32; 1]; w * h];
            img[20 * w + 20] = [1.0];
            let out = blur(&img, w, h, sigma, (0, 0), None);
            let sum: f32 = out.iter().map(|p| p[0]).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{sigma}: {sum}");
            // The row through the pixel is the response times its own weight there; normalise by its sum.
            let row: f32 = (0..w).map(|x| out[20 * w + x][0]).sum();
            let var: f32 = (0..w).map(|x| (x as f32 - 20.0).powi(2) * out[20 * w + x][0]).sum::<f32>() / row;
            assert!((f64::from(var) - sigma * sigma).abs() < 0.03 * sigma * sigma + 0.02, "{sigma}: {var}");
        }
    }

    #[test]
    fn a_constant_image_stays_constant_and_a_shift_moves_the_blur() {
        let img = vec![[0.4f32, 0.9]; 30 * 20];
        assert!(blur(&img, 30, 20, 1.2, (0, 0), None).iter().all(|p| (p[0] - 0.4).abs() < 1e-5 && (p[1] - 0.9).abs() < 1e-5));
        // Reading from (+3, 0): the bright pixel at x = 10 shows up at x = 7.
        let mut one = vec![[0f32; 1]; 30 * 20];
        one[10 * 30 + 10] = [1.0];
        let out = blur(&one, 30, 20, 0.5, (3, 0), None);
        let peak = (0..30).max_by(|&a, &b| out[10 * 30 + a][0].total_cmp(&out[10 * 30 + b][0])).unwrap();
        assert_eq!(peak, 7);
        // Outside the photo is `outside`, not a mirror.
        let edge = blur(&[[1f32; 1]; 25], 5, 5, 1.0, (0, 0), Some([0.0]));
        assert!(edge[0][0] < 0.9 && edge[12][0] > 0.95);
    }

    #[test]
    fn one_and_eight_threads_agree() {
        let img: Vec<[f32; 3]> = (0..130 * 90).map(|i| [(i % 7) as f32, (i % 11) as f32, (i % 5) as f32]).collect();
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| blur(&img, 130, 90, 1.3, (0, 0), None));
        assert_eq!(run(1), run(8));
    }

    #[test]
    fn mirroring_does_not_repeat_the_edge() {
        let seen: Vec<i64> = (-3..9).map(|i| mirror(i, 5)).collect();
        assert_eq!(seen, [3, 2, 1, 0, 1, 2, 3, 4, 3, 2, 1, 0]);
    }
}
