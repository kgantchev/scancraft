//! The helpers A16, A17 and A18 need that `scancraft-raster` does not export.
//!
//! `scancraft-raster` owns A19 and A18's steps 1–5. Three of the things this package needs are
//! not among them: the box mean of EL2, the bilinear rescale of EL4 and EL9, and the octagon's
//! **dilation** (the raster crate exports the closing, which is a dilation followed by an
//! erosion). They live here, in the package that needs them, rather than in another package's
//! crate.

use crate::EnhanceError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, BitPlane, Plane, PlaneF32, RgbF32};

/// Rows per band in every parallel pass (§7.6).
const BAND: usize = BAND_ROWS as usize;
/// The bins of the histogram the percentiles are read from.
const BINS: usize = 4096;

/// The q-th percentile of the values, `q` from 0 to 1.
///
/// The values are read from a histogram of 4096 bins over their own range, so the answer is within
/// (max − min) / 4096 of the true percentile and the cost is one pass over the values.
pub fn percentile(values: &[f32], q: f64) -> Option<f32> {
    if values.is_empty() || !q.is_finite() {
        return None;
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    if max <= min {
        return Some(min);
    }
    let scale = (BINS - 1) as f32 / (max - min);
    let mut bins = vec![0u32; BINS];
    let mut count = 0u32;
    for &v in values {
        if !v.is_finite() {
            continue;
        }
        let b = (((v - min) * scale) as usize).min(BINS - 1);
        if let Some(slot) = bins.get_mut(b) {
            *slot += 1;
        }
        count += 1;
    }
    if count == 0 {
        return None;
    }
    let target = (q.clamp(0.0, 1.0) * f64::from(count)).ceil().max(1.0) as u32;
    let mut seen = 0u32;
    for (b, &c) in bins.iter().enumerate() {
        seen += c;
        if seen >= target {
            return Some((min + (b as f32 + 0.5) / scale).clamp(min, max));
        }
    }
    Some(max)
}

/// The median of the values.
pub fn median(values: &[f32]) -> Option<f32> {
    percentile(values, 0.5)
}

/// The median of each channel of a colour list.
pub fn median_colour(colours: &[[f32; 3]]) -> Option<[f32; 3]> {
    let mut out = [0f32; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        let values: Vec<f32> = colours.iter().map(|p| p[c]).collect();
        *slot = median(&values)?;
    }
    Some(out)
}

/// Copies one channel of an RGB plane into a plane of its own.
pub fn channel(src: &RgbF32, c: usize) -> Result<PlaneF32, EnhanceError> {
    let mut data = Vec::with_capacity(src.data().len());
    for px in src.data() {
        data.push(px.get(c).copied().unwrap_or(0.0));
    }
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

/// Puts three channels back together.
pub fn channels(a: &PlaneF32, b: &PlaneF32, c: &PlaneF32) -> Result<RgbF32, EnhanceError> {
    if a.width() != b.width() || a.width() != c.width() || a.height() != b.height() || a.height() != c.height() {
        return Err(scancraft_raster::RasterError::BadParameter("the channels must have the same size").into());
    }
    let mut data = Vec::with_capacity(a.data().len());
    for ((x, y), z) in a.data().iter().zip(b.data()).zip(c.data()) {
        data.push([*x, *y, *z]);
    }
    Ok(Plane::from_vec(a.width(), a.height(), data)?)
}

/// Blurs each channel of an RGB plane with the same σ.
pub fn blur_rgb(src: &RgbF32, sigma: f32) -> Result<RgbF32, EnhanceError> {
    let [r, g, b] =
        map3(&[0usize, 1, 2], |&c| -> Result<PlaneF32, EnhanceError> { Ok(scancraft_raster::blur::gaussian(&channel(src, c)?, sigma)?) })?;
    channels(&r, &g, &b)
}

/// The mean over a square window of odd side `w`, with the edge repeated at the borders (§7.5).
///
/// A horizontal pass and then a vertical one, each a running sum over the window.
pub fn box_mean(src: &PlaneF32, w: u32) -> Result<PlaneF32, EnhanceError> {
    if w == 0 || w.is_multiple_of(2) {
        return Err(scancraft_raster::RasterError::BadParameter("the window must be odd and at least 1").into());
    }
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    if pw == 0 || ph == 0 {
        return Err(scancraft_raster::RasterError::BadParameter("the plane is empty").into());
    }
    let half = (w as usize - 1) / 2;
    // The horizontal pass.
    let mut mid = vec![0f32; pw * ph];
    mid.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = band * BAND + row_index;
            let row_in = src.data().get(y * pw..(y + 1) * pw).unwrap_or(&[]);
            window_means(row_in, half, row_out);
        }
    });
    // The vertical pass, over the transposed plane so that a column's samples are contiguous.
    let t = transpose(&mid, pw, ph);
    let mut v = vec![0f32; pw * ph];
    v.par_chunks_mut(ph * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(ph).enumerate() {
            let x = band * BAND + row_index;
            let row_in = t.get(x * ph..(x + 1) * ph).unwrap_or(&[]);
            window_means(row_in, half, row_out);
        }
    });
    let out = transpose(&v, ph, pw);
    Ok(Plane::from_vec(src.width(), src.height(), out)?)
}

/// The mean over a window of `2 · half + 1` samples, the edges repeated at the borders.
fn window_means(row: &[f32], half: usize, out: &mut [f32]) {
    let n = row.len();
    if n == 0 {
        return;
    }
    let last = n - 1;
    // The window around x = 0 holds the first sample `half` more times, as the edge is repeated.
    let mut sum = 0f32;
    for j in 0..=2 * half {
        sum += row.get(j.saturating_sub(half).min(last)).copied().unwrap_or(0.0);
    }
    for x in 0..n {
        if let Some(slot) = out.get_mut(x) {
            *slot = sum / (2 * half + 1) as f32;
        }
        let leaving = x.saturating_sub(half).min(last);
        let entering = (x + half + 1).min(last);
        sum += row.get(entering).copied().unwrap_or(0.0) - row.get(leaving).copied().unwrap_or(0.0);
    }
}

/// Turns a plane of `w` columns and `h` rows into one of `h` columns and `w` rows.
fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            if let (Some(v), Some(slot)) = (src.get(y * w + x), out.get_mut(x * h + y)) {
                *slot = *v;
            }
        }
    }
    out
}

/// The source coordinate of a destination pixel, with the pixel centres aligned: the centre of
/// the destination pixel at `x` falls on the source's `((x + 0.5) · w / out_w − 0.5)`.
fn source_coordinate(x: u32, out_len: u32, src_len: u32) -> f32 {
    if out_len == 0 || src_len == 0 {
        return 0.0;
    }
    let f = (f64::from(x) + 0.5) * f64::from(src_len) / f64::from(out_len) - 0.5;
    (f as f32).clamp(0.0, (src_len - 1) as f32)
}

/// Scales a plane to `out_w × out_h` bilinearly, with the pixel centres aligned.
pub fn bilinear(src: &PlaneF32, out_w: u32, out_h: u32) -> Result<PlaneF32, EnhanceError> {
    if out_w == 0 || out_h == 0 {
        return Err(scancraft_raster::RasterError::BadParameter("the output size is empty").into());
    }
    let (sw, sh) = (src.width(), src.height());
    let mut data = vec![0f32; out_w as usize * out_h as usize];
    data.par_chunks_mut(out_w as usize).enumerate().for_each(|(y, row)| {
        let fy = source_coordinate(y as u32, out_h, sh);
        let y0 = fy.floor() as u32;
        let y1 = (y0 + 1).min(sh.saturating_sub(1));
        let ay = fy - y0 as f32;
        for (x, slot) in row.iter_mut().enumerate() {
            let fx = source_coordinate(x as u32, out_w, sw);
            let x0 = fx.floor() as u32;
            let x1 = (x0 + 1).min(sw.saturating_sub(1));
            let ax = fx - x0 as f32;
            let p00 = src.get(x0, y0).unwrap_or(0.0);
            let p10 = src.get(x1, y0).unwrap_or(0.0);
            let p01 = src.get(x0, y1).unwrap_or(0.0);
            let p11 = src.get(x1, y1).unwrap_or(0.0);
            *slot = (p00 * (1.0 - ax) + p10 * ax) * (1.0 - ay) + (p01 * (1.0 - ax) + p11 * ax) * ay;
        }
    });
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

/// Scales an RGB plane to `out_w × out_h` bilinearly, with the pixel centres aligned.
pub fn bilinear_rgb(src: &RgbF32, out_w: u32, out_h: u32) -> Result<RgbF32, EnhanceError> {
    if out_w == 0 || out_h == 0 {
        return Err(scancraft_raster::RasterError::BadParameter("the output size is empty").into());
    }
    let (sw, sh) = (src.width(), src.height());
    let mut data = vec![[0f32; 3]; out_w as usize * out_h as usize];
    data.par_chunks_mut(out_w as usize).enumerate().for_each(|(y, row)| {
        let fy = source_coordinate(y as u32, out_h, sh);
        let y0 = fy.floor() as u32;
        let y1 = (y0 + 1).min(sh.saturating_sub(1));
        let ay = fy - y0 as f32;
        for (x, slot) in row.iter_mut().enumerate() {
            let fx = source_coordinate(x as u32, out_w, sw);
            let x0 = fx.floor() as u32;
            let x1 = (x0 + 1).min(sw.saturating_sub(1));
            let ax = fx - x0 as f32;
            let at = |sx: u32, sy: u32| src.get(sx, sy).unwrap_or([0.0; 3]);
            let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            for c in 0..3 {
                slot[c] = (p00[c] * (1.0 - ax) + p10[c] * ax) * (1.0 - ay) + (p01[c] * (1.0 - ax) + p11[c] * ax) * ay;
            }
        }
    });
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

/// The Sobel gradient of a plane: the magnitude of the two first derivatives, divided by 8.
pub fn sobel(src: &PlaneF32) -> Result<PlaneF32, EnhanceError> {
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    let mut data = vec![0f32; pw * ph];
    data.par_chunks_mut(pw * BAND).enumerate().for_each(|(band, chunk)| {
        for (row_index, row_out) in chunk.chunks_mut(pw).enumerate() {
            let y = band * BAND + row_index;
            for (x, slot) in row_out.iter_mut().enumerate() {
                let at = |dx: i64, dy: i64| {
                    let sx = (x as i64 + dx).clamp(0, pw as i64 - 1) as u32;
                    let sy = (y as i64 + dy).clamp(0, ph as i64 - 1) as u32;
                    src.get(sx, sy).unwrap_or(0.0)
                };
                let gx = -at(-1, -1) + at(1, -1) - 2.0 * at(-1, 0) + 2.0 * at(1, 0) - at(-1, 1) + at(1, 1);
                let gy = -at(-1, -1) - 2.0 * at(0, -1) - at(1, -1) + at(-1, 1) + 2.0 * at(0, 1) + at(1, 1);
                *slot = (gx * gx + gy * gy).sqrt() / 8.0;
            }
        }
    });
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

/// The octagon's half-lengths: `a = round(0.4142 · r)` along 0° and 90°, `b = round(0.2929 · r)`
/// along 45° and 135° (A19).
fn octagon_half_lengths(r: u32) -> (i64, i64) {
    ((0.4142 * f64::from(r)).round() as i64, (0.2929 * f64::from(r)).round() as i64)
}

/// The extreme over the `2 · half + 1` samples along the diagonal through each pixel. The ends of
/// the diagonal are repeated, so the window follows the line rather than the image border.
fn diagonal_max(src: &[f32], pw: usize, ph: usize, down_right: bool, half: i64, out: &mut [f32]) {
    let (w, h) = (pw as i64, ph as i64);
    for y in 0..ph {
        for x in 0..pw {
            let (x0, y0, len, k) = if down_right {
                let d = x as i64 - y as i64;
                let x0 = d.max(0);
                let y0 = (-d).max(0);
                (x0, y0, (w - x0).min(h - y0), x as i64 - x0)
            } else {
                let s = x as i64 + y as i64;
                let x0 = (s - (h - 1)).max(0);
                let y0 = s - x0;
                (x0, y0, (w - x0).min(y0 + 1), x as i64 - x0)
            };
            let mut best = f32::NEG_INFINITY;
            for t in -half..=half {
                let j = (k + t).clamp(0, len - 1);
                let sx = x0 + j;
                let sy = if down_right { y0 + j } else { y0 - j };
                let v = src.get((sy * w + sx) as usize).copied().unwrap_or(0.0);
                best = best.max(v);
            }
            if let Some(slot) = out.get_mut(y * pw + x) {
                *slot = best;
            }
        }
    }
}

/// The maximum over the octagon of radius `r`: the four maximum passes of A19, without the
/// minimum passes that turn a closing into a dilation.
///
/// `scancraft-raster` exports the closing only, and EL6 needs the dilation.
pub fn octagon_dilate(src: &PlaneF32, r: u32) -> Result<PlaneF32, EnhanceError> {
    let (a, b) = octagon_half_lengths(r);
    let (pw, ph) = (src.width() as usize, src.height() as usize);
    if pw == 0 || ph == 0 {
        return Err(scancraft_raster::RasterError::BadParameter("the plane is empty").into());
    }
    // The passes along 0° and 90°: a square of half-side a.
    let mut cur = if a > 0 { scancraft_raster::helpers::max_filter(src, (2 * a + 1) as u32)?.data().to_vec() } else { src.data().to_vec() };
    if b > 0 {
        let mut tmp = vec![0f32; pw * ph];
        diagonal_max(&cur, pw, ph, true, b, &mut tmp);
        diagonal_max(&tmp, pw, ph, false, b, &mut cur);
    }
    Ok(Plane::from_vec(src.width(), src.height(), cur)?)
}

/// A 1-bit mask as a plane of 0 and 1.
pub fn bits_to_plane(bits: &BitPlane) -> Result<PlaneF32, EnhanceError> {
    let mut data = Vec::with_capacity(bits.width() as usize * bits.height() as usize);
    for y in 0..bits.height() {
        for x in 0..bits.width() {
            data.push(if bits.get(x, y) == Some(true) { 1.0 } else { 0.0 });
        }
    }
    Ok(Plane::from_vec(bits.width(), bits.height(), data)?)
}

/// A plane of 0 and 1 as a 1-bit mask.
pub fn plane_to_bits(src: &PlaneF32) -> Result<BitPlane, EnhanceError> {
    let mut bits = BitPlane::new(src.width(), src.height())?;
    for y in 0..src.height() {
        for x in 0..src.width() {
            if src.get(x, y).unwrap_or(0.0) >= 0.5 {
                bits.set(x, y, true);
            }
        }
    }
    Ok(bits)
}

/// The 8 × 8 Bayer matrix of A18 step 6, built recursively: B1 = [0], and
/// B2n = [[4·Bn, 4·Bn + 2], [4·Bn + 3, 4·Bn + 1]], each quarter an n × n block.
pub fn bayer8() -> [[f32; 8]; 8] {
    let mut size = 1usize;
    let mut base = vec![0f32];
    while size < 8 {
        let doubled = size * 2;
        let mut next = vec![0f32; doubled * doubled];
        for (i, slot) in next.iter_mut().enumerate() {
            let (y, x) = (i / doubled, i % doubled);
            let inner = base.get((y % size) * size + (x % size)).copied().unwrap_or(0.0);
            let quarter = match (y / size, x / size) {
                (0, 0) => 0.0,
                (0, _) => 2.0,
                (_, 0) => 3.0,
                _ => 1.0,
            };
            *slot = 4.0 * inner + quarter;
        }
        base = next;
        size = doubled;
    }
    let mut m = [[0f32; 8]; 8];
    for (y, row) in m.iter_mut().enumerate() {
        for (x, slot) in row.iter_mut().enumerate() {
            *slot = base.get(y * 8 + x).copied().unwrap_or(0.0);
        }
    }
    m
}

/// Runs `f` on each of three values and keeps the three results, stopping at the first error.
///
/// The colour channels of A16 are three planes; an array of three keeps their number in the type,
/// so nothing indexes past the end.
pub fn map3<T, U, E>(src: &[T; 3], mut f: impl FnMut(&T) -> Result<U, E>) -> Result<[U; 3], E> {
    let [a, b, c] = src;
    Ok([f(a)?, f(b)?, f(c)?])
}

#[cfg(test)]
mod tests {
    use super::{bayer8, bilinear, box_mean, channel, channels, median_colour, percentile, sobel};
    use scancraft_core::image::{Plane, RgbF32};

    fn ramp(w: u32, h: u32) -> Plane<f32> {
        let data = (0..w * h).map(|i| i as f32 / (w * h) as f32).collect();
        Plane::from_vec(w, h, data).unwrap()
    }

    #[test]
    fn the_percentiles_are_read_from_the_values() {
        let values: Vec<f32> = (0..100).map(|i| i as f32).collect();
        assert!((percentile(&values, 0.0).unwrap() - 0.0).abs() < 0.5);
        assert!((percentile(&values, 0.5).unwrap() - 49.5).abs() < 1.0);
        assert!((percentile(&values, 0.75).unwrap() - 74.25).abs() < 1.0);
        assert!((percentile(&values, 1.0).unwrap() - 99.0).abs() < 0.5);
        assert!(percentile(&[], 0.5).is_none());
        // One value, and values that are all the same.
        assert_eq!(percentile(&[3.0], 0.5), Some(3.0));
        assert_eq!(percentile(&[3.0, 3.0, 3.0], 0.5), Some(3.0));
    }

    #[test]
    fn the_median_colour_is_per_channel() {
        let colours = [[0.0, 1.0, 0.5], [1.0, 0.0, 0.5], [0.5, 0.5, 0.5]];
        let m = median_colour(&colours).unwrap();
        assert!((m[0] - 0.5).abs() < 1e-3 && (m[1] - 0.5).abs() < 1e-3 && (m[2] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_box_mean_is_the_mean_of_the_window() {
        // The middle of a ramp: the window's mean is the middle value.
        let src = ramp(20, 10);
        let out = box_mean(&src, 3).unwrap();
        assert!((out.get(10, 5).unwrap() - src.get(10, 5).unwrap()).abs() < 1e-6);
        // A flat plane stays flat all the way to the edge, because the edge is repeated.
        let flat = box_mean(&Plane::new(9, 7, 0.25f32).unwrap(), 5).unwrap();
        assert!(flat.data().iter().all(|&v| (v - 0.25).abs() < 1e-6));
        // A step: the window at the edge holds the edge sample again, and the window across the
        // step holds one sample of the other side.
        let step = Plane::from_vec(6, 1, vec![0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0]).unwrap();
        let out = box_mean(&step, 3).unwrap();
        let expected = [0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0];
        for (x, want) in expected.iter().enumerate() {
            assert!((out.get(x as u32, 0).unwrap() - want).abs() < 1e-6, "x = {x}: {}", out.get(x as u32, 0).unwrap());
        }
        assert!(box_mean(&src, 4).is_err());
        assert!(box_mean(&src, 0).is_err());
    }

    #[test]
    fn the_bilinear_rescale_keeps_the_pixel_centres_aligned() {
        // A source of four pixels: the destination's centres fall between them.
        let src = Plane::from_vec(4, 1, vec![0.0f32, 1.0, 2.0, 3.0]).unwrap();
        let out = bilinear(&src, 4, 1).unwrap();
        assert_eq!(out.data(), src.data());
        // Doubling the size puts the source's centres between the new pixels.
        let out = bilinear(&src, 8, 1).unwrap();
        assert!((out.get(0, 0).unwrap() - 0.0).abs() < 1e-6);
        assert!((out.get(1, 0).unwrap() - 0.25).abs() < 1e-6);
        assert!((out.get(2, 0).unwrap() - 0.75).abs() < 1e-6);
        // Scaling down averages.
        let out = bilinear(&src, 2, 1).unwrap();
        assert!((out.get(0, 0).unwrap() - 0.5).abs() < 1e-6);
        assert!((out.get(1, 0).unwrap() - 2.5).abs() < 1e-6);
    }

    #[test]
    fn the_sobel_gradient_finds_the_step() {
        let mut src = Plane::new(16, 16, 0.0f32).unwrap();
        for y in 0..16 {
            for x in 8..16 {
                src.set(x, y, 1.0);
            }
        }
        let out = sobel(&src).unwrap();
        // The gradient is at the step and nowhere else. The Sobel of a step of 1 is 4, so the
        // magnitude divided by 8 is 0.5.
        assert!((out.get(7, 8).unwrap() - 0.5).abs() < 1e-5, "{}", out.get(7, 8).unwrap());
        assert!((out.get(8, 8).unwrap() - 0.5).abs() < 1e-5, "{}", out.get(8, 8).unwrap());
        assert!(out.get(3, 8).unwrap() < 1e-6);
        assert!(out.get(12, 8).unwrap() < 1e-6);
    }

    #[test]
    fn the_bayer_matrix_is_the_one_the_specification_gives() {
        let m = bayer8();
        // B2n = [[4·Bn, 4·Bn + 2], [4·Bn + 3, 4·Bn + 1]], so the top-left 4 × 4 quarter of the
        // 8 × 8 matrix is 4 · B4, with B4 = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]].
        let b4 = [[0.0f32, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];
        for (y, row) in b4.iter().enumerate() {
            for (x, v) in row.iter().enumerate() {
                assert_eq!(m[y][x], 4.0 * v, "top left at ({x}, {y})");
                assert_eq!(m[y][x + 4], 4.0 * v + 2.0, "top right at ({x}, {y})");
                assert_eq!(m[y + 4][x], 4.0 * v + 3.0, "bottom left at ({x}, {y})");
                assert_eq!(m[y + 4][x + 4], 4.0 * v + 1.0, "bottom right at ({x}, {y})");
            }
        }
        // Every number from 0 to 63 appears once.
        let mut seen = [false; 64];
        for row in m {
            for v in row {
                assert!(!seen[v as usize]);
                seen[v as usize] = true;
            }
        }
    }

    #[test]
    fn the_channels_go_back_together() {
        let rgb = RgbF32::from_vec(2, 2, vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]]).unwrap();
        let a = channel(&rgb, 0).unwrap();
        let b = channel(&rgb, 1).unwrap();
        let c = channel(&rgb, 2).unwrap();
        let back = channels(&a, &b, &c).unwrap();
        assert_eq!(back.data(), rgb.data());
    }
}
