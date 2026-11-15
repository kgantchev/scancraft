//! A13: inverse-mapping warp with Catmull–Rom sampling (specification A13). Owner: WP2.
//! The mapping is A12's homography or A28's unrolled sheet, through `PageMap`.
//!
//! For each output pixel the mapping gives a position in the photo. The pixel is sampled with
//! a cubic convolution (Keys 1981, a = −0.5) at the index position (x − ½, y − ½), so an
//! identity mapping copies the photo exactly. Points outside the photo are white (§7.5).

use crate::RasterError;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BAND_ROWS, MAX_PAGE_PIXELS, Plane, PlaneF32, RgbF32};
use scancraft_core::map::{PageMap, ScanOrder};

/// Rows per band in every parallel pass (§7.6).
const BAND: usize = BAND_ROWS as usize;
/// Columns per chunk in the column-wise pass.
const COLUMNS_PER_CHUNK: usize = 32;

/// The Catmull–Rom weight (Keys 1981, a = −0.5).
fn catmull_rom(t: f32) -> f32 {
    let a = t.abs();
    if a <= 1.0 {
        1.5 * a * a * a - 2.5 * a * a + 1.0
    } else if a < 2.0 {
        -0.5 * a * a * a + 2.5 * a * a - 4.0 * a + 2.0
    } else {
        0.0
    }
}

/// The four weights and the four taps of one axis of the cubic convolution.
fn taps(position: f64, length: usize) -> ([f32; 4], [usize; 4]) {
    let index = position.floor() as i64;
    let fraction = (position - index as f64) as f32;
    let weights = [catmull_rom(1.0 + fraction), catmull_rom(fraction), catmull_rom(1.0 - fraction), catmull_rom(2.0 - fraction)];
    let last = length as i64 - 1;
    let indices = [
        (index - 1).clamp(0, last) as usize,
        index.clamp(0, last) as usize,
        (index + 1).clamp(0, last) as usize,
        (index + 2).clamp(0, last) as usize,
    ];
    (weights, indices)
}

/// The colour of one photo pixel, sampled with the cubic convolution; white outside the photo.
fn sample_rgb(src: &[[f32; 3]], sw: usize, sh: usize, x: f64, y: f64) -> [f32; 3] {
    if !(x >= 0.0 && x <= sw as f64 && y >= 0.0 && y <= sh as f64) {
        return [1.0, 1.0, 1.0];
    }
    let (wx, xi) = taps(x - 0.5, sw);
    let (wy, yi) = taps(y - 0.5, sh);
    let mut acc = [0f32; 3];
    for j in 0..4 {
        let row = src.get(yi[j] * sw..(yi[j] + 1) * sw).unwrap_or(&[]);
        let mut line = [0f32; 3];
        for i in 0..4 {
            let pixel = row.get(xi[i]).copied().unwrap_or([0.0, 0.0, 0.0]);
            for c in 0..3 {
                line[c] += wx[i] * pixel[c];
            }
        }
        for c in 0..3 {
            acc[c] += wy[j] * line[c];
        }
    }
    [acc[0].clamp(0.0, 1.0), acc[1].clamp(0.0, 1.0), acc[2].clamp(0.0, 1.0)]
}

/// The value of one photo pixel, sampled with the cubic convolution; white outside the photo.
fn sample_luma(src: &[f32], sw: usize, sh: usize, x: f64, y: f64) -> f32 {
    if !(x >= 0.0 && x <= sw as f64 && y >= 0.0 && y <= sh as f64) {
        return 1.0;
    }
    let (wx, xi) = taps(x - 0.5, sw);
    let (wy, yi) = taps(y - 0.5, sh);
    let mut acc = 0f32;
    for j in 0..4 {
        let row = src.get(yi[j] * sw..(yi[j] + 1) * sw).unwrap_or(&[]);
        let mut line = 0f32;
        for i in 0..4 {
            line += wx[i] * row.get(xi[i]).copied().unwrap_or(0.0);
        }
        acc += wy[j] * line;
    }
    acc.clamp(0.0, 1.0)
}

/// The output size and the page limit of §9.
fn check_size(out_w: u32, out_h: u32) -> Result<(), RasterError> {
    if out_w == 0 || out_h == 0 {
        return Err(RasterError::BadParameter("output size must be positive"));
    }
    let pixels = u64::from(out_w) * u64::from(out_h);
    if pixels > MAX_PAGE_PIXELS {
        return Err(RasterError::Core(scancraft_core::CoreError::OutOfRange { name: "warped page pixels", value: pixels as f64 }));
    }
    Ok(())
}

/// The warp along the rows: the mapping is evaluated once per row, incrementally.
fn warp_rows_luma(src: &[f32], sw: usize, sh: usize, map: &dyn PageMap, out_w: usize, out_h: usize) -> Vec<f32> {
    let mut out = vec![0f32; out_w * out_h];
    out.par_chunks_mut(out_w * BAND).enumerate().for_each(|(band, chunk)| {
        let mut positions = vec![None; out_w];
        for (row_index, row_out) in chunk.chunks_mut(out_w).enumerate() {
            let j = band * BAND + row_index;
            let v = (j as f64 + 0.5) / out_h as f64;
            map.map_span(ScanOrder::Rows, v, &mut positions);
            for (i, slot) in row_out.iter_mut().enumerate() {
                *slot = match positions.get(i).copied().flatten() {
                    Some(p) => sample_luma(src, sw, sh, p.x, p.y),
                    None => 1.0,
                };
            }
        }
    });
    out
}

/// The warp along the rows for three channels.
fn warp_rows_rgb(src: &[[f32; 3]], sw: usize, sh: usize, map: &dyn PageMap, out_w: usize, out_h: usize) -> Vec<[f32; 3]> {
    let mut out = vec![[0f32; 3]; out_w * out_h];
    out.par_chunks_mut(out_w * BAND).enumerate().for_each(|(band, chunk)| {
        let mut positions = vec![None; out_w];
        for (row_index, row_out) in chunk.chunks_mut(out_w).enumerate() {
            let j = band * BAND + row_index;
            let v = (j as f64 + 0.5) / out_h as f64;
            map.map_span(ScanOrder::Rows, v, &mut positions);
            for (i, slot) in row_out.iter_mut().enumerate() {
                *slot = match positions.get(i).copied().flatten() {
                    Some(p) => sample_rgb(src, sw, sh, p.x, p.y),
                    None => [1.0, 1.0, 1.0],
                };
            }
        }
    });
    out
}

/// The warp along the columns: the mapping is evaluated once per column, into a transposed
/// page, which is then turned back the right way round.
fn warp_columns_luma(src: &[f32], sw: usize, sh: usize, map: &dyn PageMap, out_w: usize, out_h: usize) -> Vec<f32> {
    let mut transposed = vec![0f32; out_w * out_h];
    transposed.par_chunks_mut(out_h * COLUMNS_PER_CHUNK).enumerate().for_each(|(chunk_index, chunk)| {
        let mut positions = vec![None; out_h];
        for (index, column) in chunk.chunks_mut(out_h).enumerate() {
            let x = chunk_index * COLUMNS_PER_CHUNK + index;
            if x >= out_w {
                break;
            }
            let u = (x as f64 + 0.5) / out_w as f64;
            map.map_span(ScanOrder::Columns, u, &mut positions);
            for (y, slot) in column.iter_mut().enumerate() {
                *slot = match positions.get(y).copied().flatten() {
                    Some(p) => sample_luma(src, sw, sh, p.x, p.y),
                    None => 1.0,
                };
            }
        }
    });
    let mut out = vec![0f32; out_w * out_h];
    crate::helpers::transpose(&transposed, out_h, out_w, &mut out);
    out
}

/// The warp along the columns for three channels.
fn warp_columns_rgb(src: &[[f32; 3]], sw: usize, sh: usize, map: &dyn PageMap, out_w: usize, out_h: usize) -> Vec<[f32; 3]> {
    let mut transposed = vec![[0f32; 3]; out_w * out_h];
    transposed.par_chunks_mut(out_h * COLUMNS_PER_CHUNK).enumerate().for_each(|(chunk_index, chunk)| {
        let mut positions = vec![None; out_h];
        for (index, column) in chunk.chunks_mut(out_h).enumerate() {
            let x = chunk_index * COLUMNS_PER_CHUNK + index;
            if x >= out_w {
                break;
            }
            let u = (x as f64 + 0.5) / out_w as f64;
            map.map_span(ScanOrder::Columns, u, &mut positions);
            for (y, slot) in column.iter_mut().enumerate() {
                *slot = match positions.get(y).copied().flatten() {
                    Some(p) => sample_rgb(src, sw, sh, p.x, p.y),
                    None => [1.0, 1.0, 1.0],
                };
            }
        }
    });
    let mut out = vec![[0f32; 3]; out_w * out_h];
    crate::helpers::transpose(&transposed, out_h, out_w, &mut out);
    out
}

/// Renders an `out_w × out_h` page from `src`; points outside the photo are white.
pub fn warp_rgb(src: &RgbF32, map: &dyn PageMap, out_w: u32, out_h: u32) -> Result<RgbF32, RasterError> {
    check_size(out_w, out_h)?;
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (ow, oh) = (out_w as usize, out_h as usize);
    let data = if map.scan_order() == ScanOrder::Columns {
        warp_columns_rgb(src.data(), sw, sh, map, ow, oh)
    } else {
        warp_rows_rgb(src.data(), sw, sh, map, ow, oh)
    };
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

/// The same for one channel (analysis copies, A26, A29, A30).
pub fn warp_luma(src: &PlaneF32, map: &dyn PageMap, out_w: u32, out_h: u32) -> Result<PlaneF32, RasterError> {
    check_size(out_w, out_h)?;
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (ow, oh) = (out_w as usize, out_h as usize);
    let data = if map.scan_order() == ScanOrder::Columns {
        warp_columns_luma(src.data(), sw, sh, map, ow, oh)
    } else {
        warp_rows_luma(src.data(), sw, sh, map, ow, oh)
    };
    Ok(Plane::from_vec(out_w, out_h, data)?)
}

#[cfg(test)]
mod tests {
    use super::{warp_luma, warp_rgb};
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::image::Plane;
    use scancraft_core::map::{PageMap, ScanOrder};
    use scancraft_geom::homography::Homography;

    /// The page (u, v) ∈ [0, 1]² laid straight over a photo of `w × h` pixels.
    struct Straight {
        w: f64,
        h: f64,
    }

    impl PageMap for Straight {
        fn map(&self, u: f64, v: f64) -> Option<Point> {
            Some(Point::new(u * self.w, v * self.h))
        }
    }

    /// Any map, evaluated column by column.
    struct Columns<M: PageMap>(M);

    impl<M: PageMap> PageMap for Columns<M> {
        fn map(&self, u: f64, v: f64) -> Option<Point> {
            self.0.map(u, v)
        }
        fn scan_order(&self) -> ScanOrder {
            ScanOrder::Columns
        }
    }

    fn random_rgb(w: u32, h: u32, seed: u64) -> Plane<[f32; 3]> {
        let mut r = Xorshift64Star::new(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| [r.next_f64() as f32, r.next_f64() as f32, r.next_f64() as f32]).collect()).unwrap()
    }

    fn quad(points: [(f64, f64); 4]) -> Quad {
        Quad(points.map(|(x, y)| Point::new(x, y)))
    }

    #[test]
    fn an_identity_mapping_reproduces_the_image_exactly() {
        let rgb = random_rgb(70, 50, 1);
        let out = warp_rgb(&rgb, &Straight { w: 70.0, h: 50.0 }, 70, 50).unwrap();
        assert!(rgb.data().iter().zip(out.data()).all(|(a, b)| (0..3).all(|c| (a[c] - b[c]).abs() < 1e-6)));
        let luma = Plane::from_vec(70, 50, rgb.data().iter().map(|p| p[1]).collect()).unwrap();
        let out = warp_luma(&luma, &Straight { w: 70.0, h: 50.0 }, 70, 50).unwrap();
        assert!(luma.data().iter().zip(out.data()).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn areas_outside_the_photo_are_white() {
        let rgb = Plane::new(40, 30, [0.1f32, 0.2, 0.3]).unwrap();
        // The page lies entirely to the right of the photo.
        let out = warp_rgb(&rgb, &quad_map([(100.0, 0.0), (140.0, 0.0), (140.0, 30.0), (100.0, 30.0)]), 20, 15).unwrap();
        assert!(out.data().iter().all(|p| *p == [1.0, 1.0, 1.0]));
        // Half of it inside: the left half takes the photo's colour, the right half is white.
        let out = warp_rgb(&rgb, &quad_map([(20.0, 0.0), (60.0, 0.0), (60.0, 30.0), (20.0, 30.0)]), 40, 30).unwrap();
        assert_eq!(out.get(5, 15), Some([0.1, 0.2, 0.3]));
        assert_eq!(out.get(35, 15), Some([1.0, 1.0, 1.0]));
        // A map that is undefined somewhere gives white there, not a panic.
        struct Holes;
        impl PageMap for Holes {
            fn map(&self, u: f64, _v: f64) -> Option<Point> {
                (u < 0.5).then(|| Point::new(u * 40.0, 10.0))
            }
        }
        let out = warp_rgb(&rgb, &Holes, 40, 30).unwrap();
        assert_eq!(out.get(30, 5), Some([1.0, 1.0, 1.0]));
        assert_eq!(out.get(5, 5), Some([0.1, 0.2, 0.3]));
    }

    fn quad_map(points: [(f64, f64); 4]) -> Homography {
        Homography::from_unit_square(&quad(points), 4032.0).unwrap()
    }

    /// A checkerboard of 16-pixel squares with soft edges (σ = 2 px, like a photographed one),
    /// warped to a quadrilateral and back, comes back within 40 dB PSNR away from the photo's
    /// border. The test fails for a mapping that is off by half a pixel.
    #[test]
    fn a_checkerboard_survives_a_round_trip() {
        let (w, h) = (256u32, 192u32);
        let board: Vec<f32> = (0..w * h).map(|i| if ((i % w) / 16 + (i / w) / 16) % 2 == 0 { 0.1 } else { 0.9 }).collect();
        let source = crate::blur::gaussian(&Plane::from_vec(w, h, board).unwrap(), 2.0).unwrap();
        // The photo is `source`; the page is the board seen through a tilted frame, at about 1:1.
        let corners = quad([(6.0, 4.0), (250.0, 10.0), (244.0, 186.0), (10.0, 180.0)]);
        let forward = Homography::from_unit_square(&corners, 256.0).unwrap();
        let (pw, ph) = (240u32, 176u32);
        let page = warp_luma(&source, &forward, pw, ph).unwrap();
        // Back: a page pixel (i, j) came from the photo at forward(u, v); map each photo pixel to
        // the page by the inverse and sample the page there.
        struct Back<'a> {
            forward: &'a Homography,
            w: f64,
            h: f64,
            bias: f64,
        }
        impl PageMap for Back<'_> {
            fn map(&self, u: f64, v: f64) -> Option<Point> {
                let (pu, pv) = self.forward.invert(u * self.w, v * self.h)?;
                Some(Point::new(pu * 240.0 + self.bias, pv * 176.0))
            }
        }
        let measure = |bias: f64| -> (f64, u64) {
            let back = warp_luma(&page, &Back { forward: &forward, w: f64::from(w), h: f64::from(h), bias }, w, h).unwrap();
            // Compare where the photo lies well inside the page.
            let (mut sum, mut n) = (0f64, 0u64);
            for y in 0..h {
                for x in 0..w {
                    let (pu, pv) = forward.invert(f64::from(x) + 0.5, f64::from(y) + 0.5).unwrap_or((-1.0, -1.0));
                    let inside = pu > 0.04 && pu < 0.96 && pv > 0.04 && pv < 0.96;
                    if inside {
                        let d = f64::from(source.get(x, y).unwrap() - back.get(x, y).unwrap());
                        sum += d * d;
                        n += 1;
                    }
                }
            }
            let psnr = 10.0 * (1.0 / (sum / n as f64).max(1e-12)).log10();
            (psnr, n)
        };
        let (psnr, n) = measure(0.0);
        assert!(n > 20000);
        assert!(psnr >= 40.0, "PSNR {psnr:.1} dB over {n} pixels");
        let (shifted, _) = measure(0.5);
        assert!(shifted < 40.0, "a half-pixel error still reaches {shifted:.1} dB");
    }

    #[test]
    fn column_order_gives_the_same_pixels_as_row_order() {
        let rgb = random_rgb(120, 90, 2);
        let map = quad_map([(10.0, 8.0), (110.0, 14.0), (104.0, 80.0), (14.0, 84.0)]);
        // A page whose size is not a multiple of the chunk or band size.
        for (w, h) in [(97u32, 71u32), (33, 130), (200, 5)] {
            let rows = warp_rgb(&rgb, &map, w, h).unwrap();
            let cols = warp_rgb(&rgb, &Columns(quad_map([(10.0, 8.0), (110.0, 14.0), (104.0, 80.0), (14.0, 84.0)])), w, h).unwrap();
            assert_eq!(rows.data(), cols.data(), "{w}x{h}");
            let luma = Plane::from_vec(120, 90, rgb.data().iter().map(|p| p[0]).collect()).unwrap();
            let a = warp_luma(&luma, &map, w, h).unwrap();
            let b = warp_luma(&luma, &Columns(quad_map([(10.0, 8.0), (110.0, 14.0), (104.0, 80.0), (14.0, 84.0)])), w, h).unwrap();
            assert_eq!(a.data(), b.data(), "{w}x{h} luma");
        }
    }

    #[test]
    fn the_result_is_the_same_on_any_number_of_threads() {
        let rgb = random_rgb(300, 220, 3);
        let map = quad_map([(20.0, 10.0), (280.0, 30.0), (260.0, 200.0), (30.0, 190.0)]);
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| warp_rgb(&rgb, &map, 311, 207).unwrap());
        assert_eq!(run(1).data(), run(8).data());
    }

    #[test]
    fn bad_output_sizes_are_errors() {
        let rgb = Plane::new(10, 10, [0.5f32; 3]).unwrap();
        let map = Straight { w: 10.0, h: 10.0 };
        assert!(warp_rgb(&rgb, &map, 0, 10).is_err());
        assert!(warp_rgb(&rgb, &map, 100_000, 100_000).is_err());
    }
}
