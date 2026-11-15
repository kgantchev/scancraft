//! A3: area downscale (specification A3), written by WP0.
//!
//! Each output pixel is the average of the source area it covers, with partial weights for source
//! pixels cut by its boundary. For a scale k = out / in ≤ 1 along an axis, output column i covers
//! source x ∈ [i/k, (i + 1)/k). Weights are computed once per column (and per row) and applied
//! horizontally, then vertically, with f32 sums. Output rows are made in fixed 64-row bands, each
//! from the same inputs in the same order, so the result doesn't depend on the thread count.

use crate::RasterError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, Plane, PlaneF32, Rgb8, RgbF32};

/// For each output index, the first source index it touches and the weights from there on.
struct Taps {
    start: Vec<usize>,
    weights: Vec<Vec<f32>>,
}

fn taps(n_in: u32, n_out: u32) -> Taps {
    let scale = f64::from(n_in) / f64::from(n_out);
    let mut start = Vec::with_capacity(n_out as usize);
    let mut weights = Vec::with_capacity(n_out as usize);
    for i in 0..n_out {
        let lo = f64::from(i) * scale;
        let hi = (f64::from(i) + 1.0) * scale;
        let first = lo.floor() as usize;
        let last = (hi.ceil() as usize).min(n_in as usize).max(first + 1);
        let mut w = Vec::with_capacity(last - first);
        for s in first..last {
            let overlap = (hi.min(s as f64 + 1.0) - lo.max(s as f64)).max(0.0);
            w.push((overlap / scale) as f32);
        }
        start.push(first);
        weights.push(w);
    }
    Taps { start, weights }
}

/// Downscales `ch` interleaved f32 channels.
fn downscale_interleaved(src: &[f32], w: u32, h: u32, ch: usize, out_w: u32, out_h: u32) -> Vec<f32> {
    let tx = taps(w, out_w);
    let ty = taps(h, out_h);
    let (w, ow) = (w as usize, out_w as usize);
    // Horizontal pass: every source row to out_w columns.
    let mut mid = vec![0f32; ow * h as usize * ch];
    mid.par_chunks_mut(ow * ch * BAND_ROWS as usize).enumerate().for_each(|(band, chunk)| {
        for (r, row_out) in chunk.chunks_mut(ow * ch).enumerate() {
            let y = band * BAND_ROWS as usize + r;
            let row_in = src.get(y * w * ch..(y + 1) * w * ch).unwrap_or(&[]);
            for (i, (s0, wts)) in tx.start.iter().zip(&tx.weights).enumerate() {
                for c in 0..ch {
                    let mut acc = 0f32;
                    for (k, wt) in wts.iter().enumerate() {
                        acc += wt * row_in.get((s0 + k) * ch + c).copied().unwrap_or(0.0);
                    }
                    if let Some(slot) = row_out.get_mut(i * ch + c) {
                        *slot = acc;
                    }
                }
            }
        }
    });
    // Vertical pass: out_h rows from the intermediate.
    let mut out = vec![0f32; ow * out_h as usize * ch];
    out.par_chunks_mut(ow * ch * BAND_ROWS as usize).enumerate().for_each(|(band, chunk)| {
        for (r, row_out) in chunk.chunks_mut(ow * ch).enumerate() {
            let j = band * BAND_ROWS as usize + r;
            let (Some(s0), Some(wts)) = (ty.start.get(j), ty.weights.get(j)) else { continue };
            for (k, wt) in wts.iter().enumerate() {
                let Some(row_in) = mid.get((s0 + k) * ow * ch..(s0 + k + 1) * ow * ch) else { continue };
                for (o, v) in row_out.iter_mut().zip(row_in) {
                    *o += wt * v;
                }
            }
        }
    });
    out
}

fn check(w: u32, h: u32, out_w: u32, out_h: u32) -> Result<(), RasterError> {
    if out_w == 0 || out_h == 0 || out_w > w || out_h > h {
        return Err(RasterError::BadParameter("area downscale needs 0 < output size ≤ input size"));
    }
    Ok(())
}

/// A3 on one f32 channel.
pub fn area_downscale(src: &PlaneF32, out_w: u32, out_h: u32) -> Result<PlaneF32, RasterError> {
    check(src.width(), src.height(), out_w, out_h)?;
    let data = downscale_interleaved(src.data(), src.width(), src.height(), 1, out_w, out_h);
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

/// A3 on three f32 channels.
pub fn area_downscale_rgb(src: &RgbF32, out_w: u32, out_h: u32) -> Result<RgbF32, RasterError> {
    check(src.width(), src.height(), out_w, out_h)?;
    let flat: Vec<f32> = src.data().iter().flatten().copied().collect();
    let data = downscale_interleaved(&flat, src.width(), src.height(), 3, out_w, out_h);
    let px = data.as_chunks::<3>().0.to_vec();
    Ok(Plane::from_vec(out_w, out_h, px)?)
}

/// A3 on 8-bit sRGB, averaging the encoded values as the specification does, rounded back.
pub fn area_downscale_rgb8(src: &Rgb8, out_w: u32, out_h: u32) -> Result<Rgb8, RasterError> {
    check(src.width(), src.height(), out_w, out_h)?;
    let flat: Vec<f32> = src.data().iter().flatten().map(|&v| f32::from(v)).collect();
    let data = downscale_interleaved(&flat, src.width(), src.height(), 3, out_w, out_h);
    let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    let px = data.as_chunks::<3>().0.iter().map(|c| c.map(q)).collect();
    Ok(Plane::from_vec(out_w, out_h, px)?)
}

/// The output size whose long side is `long_side`, keeping the aspect ratio, never larger than
/// the input (A3 makes the work image, display copy and previews this way).
pub fn fit_long_side(w: u32, h: u32, long_side: u32) -> (u32, u32) {
    let l = w.max(h);
    if l <= long_side || l == 0 {
        return (w, h);
    }
    let k = f64::from(long_side) / f64::from(l);
    (((f64::from(w) * k).round() as u32).max(1), ((f64::from(h) * k).round() as u32).max(1))
}

#[cfg(test)]
mod tests {
    use super::{area_downscale, area_downscale_rgb8, fit_long_side};
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::image::Plane;

    fn random_plane(w: u32, h: u32, seed: u64) -> Plane<f32> {
        let mut r = Xorshift64Star::new(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| r.next_f64() as f32).collect()).unwrap()
    }

    #[test]
    fn integer_factor_gives_exact_block_means() {
        let src = random_plane(12, 8, 1);
        let out = area_downscale(&src, 4, 2).unwrap();
        for j in 0..2 {
            for i in 0..4 {
                let mut sum = 0f32;
                for y in 0..4 {
                    for x in 0..3 {
                        sum += src.get(i * 3 + x, j * 4 + y).unwrap();
                    }
                }
                assert!((out.get(i, j).unwrap() - sum / 12.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn constant_stays_constant_and_mean_is_preserved() {
        let c = Plane::new(1000, 750, 0.42f32).unwrap();
        let out = area_downscale(&c, 333, 251).unwrap();
        assert!(out.data().iter().all(|v| (v - 0.42).abs() < 1e-5));
        let src = random_plane(1000, 750, 2);
        let out = area_downscale(&src, 333, 251).unwrap();
        let mean = |p: &Plane<f32>| p.data().iter().map(|&v| f64::from(v)).sum::<f64>() / p.data().len() as f64;
        assert!((mean(&src) - mean(&out)).abs() < 1e-5);
    }

    #[test]
    fn eight_bit_rgb_rounds_and_rejects_upscaling() {
        let src = Plane::new(4, 4, [10u8, 200, 255]).unwrap();
        assert_eq!(area_downscale_rgb8(&src, 2, 2).unwrap().get(1, 1), Some([10, 200, 255]));
        assert!(area_downscale_rgb8(&src, 5, 2).is_err());
        assert!(area_downscale_rgb8(&src, 0, 2).is_err());
    }

    #[test]
    fn same_result_with_one_or_many_threads() {
        let src = random_plane(1023, 517, 3);
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| area_downscale(&src, 400, 202).unwrap());
        let many = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(|| area_downscale(&src, 400, 202).unwrap());
        assert_eq!(one.data(), many.data());
    }

    #[test]
    fn long_side_fit() {
        assert_eq!(fit_long_side(4032, 3024, 1024), (1024, 768));
        assert_eq!(fit_long_side(800, 600, 1024), (800, 600));
    }
}
