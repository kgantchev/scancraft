//! A5: colour edges (specification A5). Owner: WP8.
//!
//! Canny's detector on the work image: it marks thin lines where brightness changes fastest and
//! then keeps the weaker edge pixels only where they connect to strong ones. The rule OpenCV uses
//! for multi-channel input is followed: each pixel takes its gradient from the channel that
//! changes most, so a page that differs from the desk mostly in blue still has an edge.

use crate::DetectError;
use rayon::prelude::*;
use scancraft_core::CoreError;
use scancraft_core::image::{BAND_ROWS, BitPlane, Plane, PlaneF32, RgbF32};
use scancraft_raster::blur::gaussian;

/// The blur A5 puts before the gradients: σ = 1.5 px (A5 step 1).
pub const BLUR_SIGMA: f32 = 1.5;
/// The lowest magnitude that counts towards the thresholds (A5 step 5).
const MIN_MAGNITUDE: f32 = 0.005;
/// The strict map's high threshold's bounds (A5 step 5).
const STRICT_RANGE: (f32, f32) = (0.040, 0.200);
/// The sensitive map's high threshold's bounds (A5 step 5).
const SENSITIVE_RANGE: (f32, f32) = (0.015, 0.100);
/// The low threshold's share of the high one (A5 step 5).
const LOW_SHARE: f32 = 0.4;

/// A5's output: both edge maps, plus magnitude G and direction θ for A9 and A10.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeMaps {
    pub strict: BitPlane,
    pub sensitive: BitPlane,
    pub magnitude: PlaneF32,
    /// Radians.
    pub direction: PlaneF32,
    /// The work image blurred with A5's σ = 1.5, which A9's contrast is measured on.
    pub blurred: RgbF32,
}

/// A5 on the work image (RGB, long side 1024 px).
pub fn colour_edges(work: &RgbF32) -> Result<EdgeMaps, DetectError> {
    let (w, h) = (work.width(), work.height());
    if w < 3 || h < 3 {
        return Err(CoreError::OutOfRange { name: "work image", value: f64::from(w.min(h)) }.into());
    }
    // Step 1: blur each channel, so wood grain and fabric don't read as edges.
    let channels = split_channels(work)?;
    let mut blurred: Vec<PlaneF32> = Vec::with_capacity(3);
    for channel in &channels {
        blurred.push(gaussian(channel, BLUR_SIGMA)?);
    }
    let blurred_rgb = join_channels(&blurred)?;
    // Steps 2 and 3: the Sobel gradients of every channel, and the channel that changes most at
    // each pixel.
    let mut magnitude = PlaneF32::new(w, h, 0.0)?;
    let mut direction = PlaneF32::new(w, h, 0.0)?;
    let mut best = vec![0f32; w as usize * h as usize];
    for channel in &blurred {
        let (gx, gy) = sobel(channel)?;
        keep_stronger(&gx, &gy, &mut best, &mut magnitude, &mut direction);
    }
    // Step 4: thin the edges.
    let alive = non_maximum(&magnitude, &direction);
    // Step 5: the thresholds, from the pixels that are left.
    let mut values: Vec<f32> = Vec::new();
    for (i, &g) in magnitude.data().iter().enumerate() {
        if alive[i] && g > MIN_MAGNITUDE && g.is_finite() {
            values.push(g);
        }
    }
    let (strict_high, sensitive_high) = if values.is_empty() {
        (0.0, 0.0)
    } else {
        let p80 = percentile(&mut values, 0.80);
        let p60 = percentile(&mut values, 0.60);
        (p80.clamp(STRICT_RANGE.0, STRICT_RANGE.1), p60.clamp(SENSITIVE_RANGE.0, SENSITIVE_RANGE.1))
    };
    // Step 6: hysteresis.
    let strict = hysteresis(&alive, &magnitude, strict_high, strict_high * LOW_SHARE)?;
    let sensitive = hysteresis(&alive, &magnitude, sensitive_high, sensitive_high * LOW_SHARE)?;
    Ok(EdgeMaps { strict, sensitive, magnitude, direction, blurred: blurred_rgb })
}

/// The three channels of an RGB plane, each on its own.
fn split_channels(work: &RgbF32) -> Result<Vec<PlaneF32>, DetectError> {
    let (w, h) = (work.width(), work.height());
    (0..3)
        .map(|c| {
            let data: Vec<f32> = work.data().iter().map(|px| px.get(c).copied().unwrap_or(0.0)).collect();
            Plane::from_vec(w, h, data).map_err(DetectError::from)
        })
        .collect()
}

/// The three channels of a colour plane, each on its own.
fn join_channels(channels: &[PlaneF32]) -> Result<RgbF32, DetectError> {
    let (w, h) = (channels.first().map(|c| c.width()).unwrap_or(0), channels.first().map(|c| c.height()).unwrap_or(0));
    let mut px: Vec<[f32; 3]> = Vec::with_capacity(w as usize * h as usize);
    for i in 0..(w as usize * h as usize) {
        px.push([
            channels.first().and_then(|c| c.data().get(i)).copied().unwrap_or(0.0),
            channels.get(1).and_then(|c| c.data().get(i)).copied().unwrap_or(0.0),
            channels.get(2).and_then(|c| c.data().get(i)).copied().unwrap_or(0.0),
        ]);
    }
    Ok(Plane::from_vec(w, h, px)?)
}

/// The Sobel gradients of one channel, scaled so that a ramp rising by 1 per pixel gives 1
/// (A5 step 2).
///
/// The pixels outside the image are taken as the nearest pixel inside it, the same convention
/// the rest of the pipeline uses (§7.5).
fn sobel(plane: &PlaneF32) -> Result<(PlaneF32, PlaneF32), DetectError> {
    let (w, h) = (plane.width() as i64, plane.height() as i64);
    let data = plane.data();
    let at = |x: i64, y: i64| -> f32 {
        let cx = x.clamp(0, w - 1) as usize;
        let cy = y.clamp(0, h - 1) as usize;
        data.get(cy * w as usize + cx).copied().unwrap_or(0.0)
    };
    let mut gx = Plane::new(plane.width(), plane.height(), 0.0f32)?;
    let mut gy = Plane::new(plane.width(), plane.height(), 0.0f32)?;
    let width = w as usize;
    let (gx_data, gy_data) = (gx.data_mut(), gy.data_mut());
    gx_data.par_chunks_mut(width * BAND_ROWS as usize).zip(gy_data.par_chunks_mut(width * BAND_ROWS as usize)).enumerate().for_each(
        |(band, (gx_rows, gy_rows))| {
            let y0 = band * BAND_ROWS as usize;
            for (r, (gx_row, gy_row)) in gx_rows.chunks_mut(width).zip(gy_rows.chunks_mut(width)).enumerate() {
                let y = (y0 + r) as i64;
                for (x, (gx_slot, gy_slot)) in gx_row.iter_mut().zip(gy_row.iter_mut()).enumerate() {
                    let x = x as i64;
                    let (xm, xp) = (x - 1, x + 1);
                    let (ym, yp) = (y - 1, y + 1);
                    let gxv = (at(xp, ym) + 2.0 * at(xp, y) + at(xp, yp) - at(xm, ym) - 2.0 * at(xm, y) - at(xm, yp)) / 8.0;
                    let gyv = (at(xm, yp) + 2.0 * at(x, yp) + at(xp, yp) - at(xm, ym) - 2.0 * at(x, ym) - at(xp, ym)) / 8.0;
                    *gx_slot = gxv;
                    *gy_slot = gyv;
                }
            }
        },
    );
    Ok((gx, gy))
}

/// Keeps the channel that changes most at each pixel (A5 step 3).
///
/// The magnitude is √(gx² + gy²) and the direction is atan2(gy, gx), both in radians.
fn keep_stronger(gx: &PlaneF32, gy: &PlaneF32, best: &mut [f32], magnitude: &mut PlaneF32, direction: &mut PlaneF32) {
    let width = gx.width() as usize;
    let (gx_data, gy_data) = (gx.data(), gy.data());
    let (mag_data, dir_data) = (magnitude.data_mut(), direction.data_mut());
    mag_data
        .par_chunks_mut(width * BAND_ROWS as usize)
        .zip(dir_data.par_chunks_mut(width * BAND_ROWS as usize))
        .zip(best.par_chunks_mut(width * BAND_ROWS as usize))
        .enumerate()
        .for_each(|(band, ((mag_rows, dir_rows), best_rows))| {
            let y0 = band * BAND_ROWS as usize;
            for (r, ((mag_row, dir_row), best_row)) in
                mag_rows.chunks_mut(width).zip(dir_rows.chunks_mut(width)).zip(best_rows.chunks_mut(width)).enumerate()
            {
                let y = y0 + r;
                for (x, ((mag_slot, dir_slot), best_slot)) in mag_row.iter_mut().zip(dir_row.iter_mut()).zip(best_row.iter_mut()).enumerate() {
                    let i = y * width + x;
                    let (a, b) = (gx_data[i], gy_data[i]);
                    let m = a * a + b * b;
                    if m > *best_slot {
                        *best_slot = m;
                        *mag_slot = m.sqrt();
                        *dir_slot = b.atan2(a);
                    }
                }
            }
        });
}

/// The pixels that survive non-maximum suppression (A5 step 4).
///
/// A pixel is kept when its magnitude is greater than the neighbour's in the gradient direction
/// and at least the neighbour's in the opposite direction, so that edges come out one pixel wide
/// and a flat stretch of edge is kept whole. The outermost pixels of the image are cleared.
fn non_maximum(magnitude: &PlaneF32, direction: &PlaneF32) -> Vec<bool> {
    let (w, h) = (magnitude.width() as usize, magnitude.height() as usize);
    let mut alive = vec![false; w * h];
    let (mag, dirs) = (magnitude.data(), direction.data());
    alive.par_chunks_mut(w * BAND_ROWS as usize).enumerate().for_each(|(band, chunk)| {
        let y0 = band * BAND_ROWS as usize;
        for (r, slot) in chunk.iter_mut().enumerate() {
            let y = y0 + r / w;
            if y == 0 || y + 1 == h {
                continue;
            }
            let x = r % w;
            if x == 0 || x + 1 == w {
                continue;
            }
            let i = y * w + x;
            let g = mag[i];
            if g.is_nan() || g <= 0.0 {
                continue;
            }
            let (dx, dy) = neighbour_offsets(dirs[i]);
            let forward = mag[(y as isize + dy) as usize * w + (x as isize + dx) as usize];
            let backward = mag[(y as isize - dy) as usize * w + (x as isize - dx) as usize];
            *slot = g > forward && g >= backward;
        }
    });
    alive
}

/// The neighbour to compare with, in the gradient's direction, for a direction in radians.
///
/// The direction is rounded, modulo 180°, to 0°, 45°, 90° or 135°; a half-way angle rounds up.
/// The rounding gives the axis, and the gradient itself says which way along it the neighbour
/// lies.
fn neighbour_offsets(theta: f32) -> (isize, isize) {
    let degrees = f64::from(theta).to_degrees().rem_euclid(180.0);
    let axis = ((degrees + 22.5) / 45.0).floor() as i64 % 4;
    let (sin, cos) = theta.sin_cos();
    let sx = if cos >= 0.0 { 1 } else { -1 };
    let sy = if sin >= 0.0 { 1 } else { -1 };
    match axis {
        0 => (sx, 0),
        1 => (sx, sy),
        2 => (0, sy),
        _ => (sx, sy),
    }
}

/// The percentage of the values below the given one, by the nearest rank.
///
/// The values are left in no particular order; the one at the rank is found in place.
fn percentile(values: &mut [f32], p: f64) -> f32 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let rank = ((p * n as f64).ceil() as usize).clamp(1, n) - 1;
    values.select_nth_unstable_by(rank, |a, b| f64::from(*a).total_cmp(&f64::from(*b)));
    values.get(rank).copied().unwrap_or(0.0)
}

/// The edge map of one threshold pair: the strong pixels, grown through the weaker ones that
/// touch them (A5 step 6).
///
/// The growth is a flood fill with an explicit stack, so a long edge never recurses.
fn hysteresis(alive: &[bool], magnitude: &PlaneF32, high: f32, low: f32) -> Result<BitPlane, DetectError> {
    let (w, h) = (magnitude.width() as usize, magnitude.height() as usize);
    let mut out = BitPlane::new(magnitude.width(), magnitude.height())?;
    if high <= 0.0 && low <= 0.0 {
        return Ok(out);
    }
    let mag = magnitude.data();
    let mut marked = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..w * h {
        if alive[i] && mag[i] >= high {
            marked[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if marked[j] || !alive[j] || mag[j] < low {
                    continue;
                }
                marked[j] = true;
                stack.push(j);
            }
        }
    }
    for (i, _) in marked.iter().enumerate().filter(|(_, on)| **on) {
        out.set((i % w) as u32, (i / w) as u32, true);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{colour_edges, neighbour_offsets, percentile};
    use scancraft_core::image::{BitPlane, Plane, RgbF32};

    fn work(w: u32, h: u32, f: impl Fn(f64, f64) -> [f32; 3]) -> RgbF32 {
        let mut px: Vec<[f32; 3]> = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(f(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
        Plane::from_vec(w, h, px).unwrap()
    }

    /// The pixels of a map that are set.
    fn set_pixels(map: &BitPlane) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for y in 0..map.height() {
            for x in 0..map.width() {
                if map.get(x, y) == Some(true) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn a_vertical_step_is_one_pixel_wide() {
        let img = work(60, 60, |x, _| if x < 30.0 { [0.1; 3] } else { [0.9; 3] });
        let maps = colour_edges(&img).unwrap();
        let pixels = set_pixels(&maps.sensitive);
        assert!(!pixels.is_empty());
        // Every edge pixel sits on the step, and no two of them are neighbours in x.
        for (x, _) in &pixels {
            assert!((29..=31).contains(x), "edge at x = {x}");
        }
        let mut xs: Vec<u32> = pixels.iter().map(|(x, _)| *x).collect();
        xs.sort_unstable();
        xs.dedup();
        assert!(xs.len() <= 3, "{xs:?}");
    }

    #[test]
    fn a_pale_yellow_desk_under_white_paper_has_an_edge() {
        // White paper (0.95, 0.95, 0.95) on a pale yellow desk (0.97, 0.95, 0.80): the luma
        // differs by 0.011, the blue by 0.15 (A5's "why colour").
        let img = work(60, 60, |x, _| if x < 30.0 { [0.97, 0.95, 0.80] } else { [0.95, 0.95, 0.95] });
        let maps = colour_edges(&img).unwrap();
        let pixels = set_pixels(&maps.sensitive);
        assert!(pixels.len() > 30, "the pale edge was not found: {} pixels", pixels.len());
    }

    #[test]
    fn a_faint_step_is_found_by_the_sensitive_map_only() {
        // A 0.12 step: the blurred edge peaks at about 0.032, above the sensitive map's floor of
        // 0.015 and below the strict map's floor of 0.040.
        let img = work(60, 60, |x, _| if x < 30.0 { [0.5; 3] } else { [0.62; 3] });
        let maps = colour_edges(&img).unwrap();
        assert!(!set_pixels(&maps.sensitive).is_empty());
        assert!(set_pixels(&maps.strict).is_empty());
    }

    #[test]
    fn a_flat_image_has_no_edges() {
        let img = work(40, 40, |_, _| [0.5; 3]);
        let maps = colour_edges(&img).unwrap();
        assert!(set_pixels(&maps.strict).is_empty());
        assert!(set_pixels(&maps.sensitive).is_empty());
    }

    #[test]
    fn the_edges_follow_a_diagonal_at_every_angle() {
        // A step with a contrast of 0.1, at 15° steps: the edge must be found along at least
        // 95% of its length, within 1 px (A5's test).
        for step in 0..24u32 {
            let angle = f64::from(step) * 15.0;
            let (sin, cos) = angle.to_radians().sin_cos();
            let img = work(120, 120, move |x, y| {
                let d = (x - 60.0) * cos + (y - 60.0) * sin;
                if d < 0.0 { [0.45; 3] } else { [0.55; 3] }
            });
            let maps = colour_edges(&img).unwrap();
            let pixels = set_pixels(&maps.sensitive);
            // The step runs along the line through the middle, at right angles to the gradient;
            // count how much of that line has an edge within 1 px.
            let mut along = 0usize;
            let mut found = 0usize;
            for t in 0..100 {
                let px = 60.0 - sin * (t as f64 - 50.0);
                let py = 60.0 + cos * (t as f64 - 50.0);
                if !(0.0..120.0).contains(&px) || !(0.0..120.0).contains(&py) {
                    continue;
                }
                along += 1;
                let near = pixels.iter().any(|(x, y)| {
                    let dx = f64::from(*x) + 0.5 - px;
                    let dy = f64::from(*y) + 0.5 - py;
                    dx * dx + dy * dy <= 1.0
                });
                if near {
                    found += 1;
                }
            }
            assert!(along > 0);
            let ratio = found as f64 / along as f64;
            assert!(ratio >= 0.95, "{angle}°: only {ratio:.3} of the edge was found");
        }
    }

    #[test]
    fn the_neighbour_of_a_direction_is_the_one_the_angle_names() {
        assert_eq!(neighbour_offsets(0.0), (1, 0));
        assert_eq!(neighbour_offsets(45.0_f32.to_radians()), (1, 1));
        assert_eq!(neighbour_offsets(90.0_f32.to_radians()), (0, 1));
        assert_eq!(neighbour_offsets(135.0_f32.to_radians()), (-1, 1));
        // The axis comes from the angle, and the gradient says which way along it.
        assert_eq!(neighbour_offsets(180.0_f32.to_radians()), (-1, 0));
        assert_eq!(neighbour_offsets(-45.0_f32.to_radians()), (1, -1));
        assert_eq!(neighbour_offsets(225.0_f32.to_radians()), (-1, -1));
    }

    #[test]
    fn a_percentile_is_the_value_at_its_rank() {
        let mut values: Vec<f32> = (1..=100).map(|v| v as f32).collect();
        assert_eq!(percentile(&mut values, 0.80), 80.0);
        assert_eq!(percentile(&mut values, 0.60), 60.0);
        let mut one = vec![7.0f32];
        assert_eq!(percentile(&mut one, 0.5), 7.0);
    }
}
