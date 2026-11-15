//! A14: separable Lanczos-3 resampling (specification A14). Owner: WP2.
//!
//! For a scale k = out/in, output sample i has its centre at source position (i + ½)/k and takes
//! the source samples within 3/k′ of it, with k′ = min(k, 1), each weighted by L(k′·t), where t is
//! the distance from the centre to the sample's own centre. Weights are normalised to sum to 1.
//! The horizontal pass runs first, then the vertical one, in f32; the result is clamped to [0, 1].
//! Samples beyond the border mirror the image without repeating the edge pixel (… c b | a b c d |
//! c b …), as §7.5 says of separable filters. Output rows are made in fixed 64-row bands,
//! so the result doesn't depend on the thread count (§7.6).

use crate::RasterError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, MAX_PAGE_PIXELS, Plane, PlaneF32, RgbF32};

/// The weights of one output index: the first source index (which may be negative) and the
/// weights from there on.
struct Taps {
    first: Vec<i64>,
    weights: Vec<Vec<f32>>,
}

/// The index inside 0..n that mirrors `i` without repeating the edge pixel.
fn mirror(i: i64, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n as i64 - 1);
    let m = i.rem_euclid(period);
    (if m < n as i64 { m } else { period - m }) as usize
}

/// sinc(t) = sin(πt)/(πt), with sinc(0) = 1.
fn sinc(t: f64) -> f64 {
    if t.abs() < 1e-12 {
        1.0
    } else {
        let x = std::f64::consts::PI * t;
        x.sin() / x
    }
}

/// The Lanczos-3 kernel: sinc(t)·sinc(t/3) for |t| < 3, else 0.
fn lanczos(t: f64) -> f64 {
    if t.abs() < 3.0 { sinc(t) * sinc(t / 3.0) } else { 0.0 }
}

fn taps(n_in: u32, n_out: u32) -> Taps {
    let k = f64::from(n_out) / f64::from(n_in);
    // A source sample is "within 3/k of the centre" when shrinking; when growing, the kernel is
    // the plain one, so interpolation passes through the samples.
    let k_eff = k.min(1.0);
    let radius = 3.0 / k_eff;
    let mut first = Vec::with_capacity(n_out as usize);
    let mut weights = Vec::with_capacity(n_out as usize);
    for i in 0..n_out {
        let centre = (f64::from(i) + 0.5) / k;
        let lo = (centre - radius - 0.5).floor() as i64;
        let hi = (centre + radius - 0.5).ceil() as i64;
        let mut w: Vec<f64> = (lo..=hi).map(|j| lanczos(k_eff * ((j as f64 + 0.5) - centre))).collect();
        let sum: f64 = w.iter().sum();
        if sum.abs() > 1e-12 {
            for v in &mut w {
                *v /= sum;
            }
        } else if let Some(middle) = w.len().checked_div(2).and_then(|m| w.get_mut(m)) {
            *middle = 1.0;
        }
        first.push(lo);
        weights.push(w.into_iter().map(|v| v as f32).collect());
    }
    Taps { first, weights }
}

/// Resamples `ch` interleaved f32 channels.
fn resample_interleaved(src: &[f32], w: u32, h: u32, ch: usize, out_w: u32, out_h: u32) -> Vec<f32> {
    let tx = taps(w, out_w);
    let ty = taps(h, out_h);
    let (w, hh, ow) = (w as usize, h as usize, out_w as usize);
    let band = BAND_ROWS as usize;
    // Horizontal pass: every source row to out_w columns.
    let mut mid = vec![0f32; ow * hh * ch];
    mid.par_chunks_mut(ow * ch * band).enumerate().for_each(|(b, chunk)| {
        for (r, row_out) in chunk.chunks_mut(ow * ch).enumerate() {
            let y = b * band + r;
            let row_in = src.get(y * w * ch..(y + 1) * w * ch).unwrap_or(&[]);
            for (i, (first, wts)) in tx.first.iter().zip(&tx.weights).enumerate() {
                for c in 0..ch {
                    let mut acc = 0f32;
                    for (k, wt) in wts.iter().enumerate() {
                        let x = mirror(first + k as i64, w);
                        acc += wt * row_in.get(x * ch + c).copied().unwrap_or(0.0);
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
    out.par_chunks_mut(ow * ch * band).enumerate().for_each(|(b, chunk)| {
        for (r, row_out) in chunk.chunks_mut(ow * ch).enumerate() {
            let j = b * band + r;
            let (Some(first), Some(wts)) = (ty.first.get(j), ty.weights.get(j)) else { continue };
            for (k, wt) in wts.iter().enumerate() {
                let y = mirror(first + k as i64, hh);
                let Some(row_in) = mid.get(y * ow * ch..(y + 1) * ow * ch) else { continue };
                for (o, v) in row_out.iter_mut().zip(row_in) {
                    *o += wt * v;
                }
            }
            for o in row_out.iter_mut() {
                *o = o.clamp(0.0, 1.0);
            }
        }
    });
    out
}

fn check(w: u32, h: u32, out_w: u32, out_h: u32) -> Result<(), RasterError> {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return Err(RasterError::BadParameter("resampling needs non-empty images"));
    }
    if u64::from(out_w) * u64::from(out_h) > MAX_PAGE_PIXELS {
        return Err(RasterError::BadParameter("resampled page is too large"));
    }
    Ok(())
}

/// A14 on three channels.
pub fn lanczos3_rgb(src: &RgbF32, out_w: u32, out_h: u32) -> Result<RgbF32, RasterError> {
    check(src.width(), src.height(), out_w, out_h)?;
    let flat: Vec<f32> = src.data().iter().flatten().copied().collect();
    let data = resample_interleaved(&flat, src.width(), src.height(), 3, out_w, out_h);
    let px = data.as_chunks::<3>().0.to_vec();
    Ok(Plane::from_vec(out_w, out_h, px)?)
}

/// A14 on one channel.
pub fn lanczos3(src: &PlaneF32, out_w: u32, out_h: u32) -> Result<PlaneF32, RasterError> {
    check(src.width(), src.height(), out_w, out_h)?;
    let data = resample_interleaved(src.data(), src.width(), src.height(), 1, out_w, out_h);
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

#[cfg(test)]
mod tests {
    use super::{lanczos3, lanczos3_rgb};
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::image::Plane;

    fn random_plane(w: u32, h: u32, seed: u64) -> Plane<f32> {
        let mut r = Xorshift64Star::new(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| r.next_f64() as f32).collect()).unwrap()
    }

    #[test]
    fn a_constant_image_stays_constant() {
        for (w, h, ow, oh) in [(1000, 700, 333, 251), (300, 200, 450, 120), (97, 61, 97, 61)] {
            let c = Plane::new(w, h, 0.37f32).unwrap();
            let out = lanczos3(&c, ow, oh).unwrap();
            assert!(out.data().iter().all(|v| (v - 0.37).abs() < 1e-5), "{w}x{h} -> {ow}x{oh}");
            let rgb = Plane::new(w, h, [0.2f32, 0.5, 0.9]).unwrap();
            let out = lanczos3_rgb(&rgb, ow, oh).unwrap();
            assert!(out.data().iter().all(|p| (p[0] - 0.2).abs() < 1e-5 && (p[1] - 0.5).abs() < 1e-5 && (p[2] - 0.9).abs() < 1e-5));
        }
    }

    #[test]
    fn the_same_size_is_the_identity() {
        let src = random_plane(70, 50, 4);
        let out = lanczos3(&src, 70, 50).unwrap();
        assert!(src.data().iter().zip(out.data()).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    /// A grating keeps its contrast: Lanczos-3 passes 0.99 of a grating at 0.3 cycles per output
    /// pixel and 0.82 of one at 0.4 (the kernel's own response, from its Fourier transform). The
    /// samples of a grating at 0.4 cycles miss its peaks by up to 5%, hence 0.7 and not 0.82.
    #[test]
    fn a_grating_keeps_its_contrast() {
        for (cycles, least) in [(0.3f64, 0.9f32), (0.4, 0.7)] {
            for scale in [0.5f64, 0.3, 0.75] {
                let (ow, oh) = (400u32, 8u32);
                let (w, h) = ((f64::from(ow) / scale).round() as u32, 16u32);
                // `cycles` per output pixel is cycles·k per source pixel.
                let freq = cycles * f64::from(ow) / f64::from(w);
                let data: Vec<f32> = (0..w * h).map(|i| (0.5 + 0.5 * (2.0 * std::f64::consts::PI * freq * f64::from(i % w)).sin()) as f32).collect();
                let src = Plane::from_vec(w, h, data).unwrap();
                let out = lanczos3(&src, ow, oh).unwrap();
                let row = out.row(oh / 2).unwrap();
                let middle = &row[ow as usize / 4..3 * ow as usize / 4];
                let (lo, hi) = middle.iter().fold((1f32, 0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                assert!((hi - lo) >= least, "{cycles} cycles, scale {scale}: contrast {}", hi - lo);
            }
        }
    }

    /// A zone plate whose frequency rises past the output's Nyquist limit comes out flat there,
    /// not as moiré.
    #[test]
    fn a_zone_plate_shows_no_aliasing() {
        let (w, h) = (2000u32, 8u32);
        // Frequency a·x cycles per pixel, reaching 0.5 at the far edge.
        let a = 0.5 / f64::from(w);
        let data: Vec<f32> = (0..w * h)
            .map(|i| {
                let x = f64::from(i % w);
                (0.5 + 0.5 * (std::f64::consts::PI * a * x * x).sin()) as f32
            })
            .collect();
        let src = Plane::from_vec(w, h, data).unwrap();
        let out = lanczos3(&src, w / 2, h / 2).unwrap();
        let row = out.row(2).unwrap();
        // Source frequency above 0.375 (1.5 times the output's Nyquist limit of 0.25) is x > 1500,
        // which is output index 750 and beyond; stay clear of the border.
        let stop = &row[760..990];
        let swing = stop.iter().fold(0f32, |m, &v| m.max((v - 0.5).abs()));
        assert!(swing < 0.12, "aliased swing {swing}");
    }

    #[test]
    fn upsampling_passes_through_the_samples_at_integer_ratios() {
        let src = random_plane(20, 20, 5);
        let out = lanczos3(&src, 40, 40).unwrap();
        // Output sample 2i + 1 and 2i sit at ¼ and ¾ of the source pixel: the mean of the pair is
        // close to the source pixel away from the border.
        for y in 4..16 {
            for x in 4..16 {
                let mean = (out.get(2 * x, 2 * y).unwrap() + out.get(2 * x + 1, 2 * y + 1).unwrap()) / 2.0;
                assert!((mean - src.get(x, y).unwrap()).abs() < 0.5, "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_border_mirrors_without_repeating_the_edge_pixel() {
        use super::mirror;
        let seen: Vec<usize> = (-3..9).map(|i| mirror(i, 5)).collect();
        assert_eq!(seen, [3, 2, 1, 0, 1, 2, 3, 4, 3, 2, 1, 0]);
        assert_eq!(mirror(-7, 1), 0);
    }

    #[test]
    fn bad_sizes_are_errors() {
        let src = random_plane(10, 10, 1);
        assert!(lanczos3(&src, 0, 5).is_err());
        assert!(lanczos3(&src, 100_000, 100_000).is_err());
    }

    #[test]
    fn same_result_with_one_or_many_threads() {
        let src = random_plane(1023, 517, 3);
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| lanczos3(&src, 400, 202).unwrap());
        assert_eq!(run(1).data(), run(8).data());
    }
}
