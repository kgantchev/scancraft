//! Rendering the sheet into a photo: the sheet is meshed, the mesh is projected, and its triangles
//! are rasterised with perspective-correct (u, v) at 3 × 3 samples per pixel into a page layer.
//! Each pixel's page colour comes from the page texture at the interpolated (u, v), with a mip
//! level that follows the triangle's scale, so the page is filtered as a camera would see it.
//! Work runs in fixed 64-row bands, so the result doesn't depend on the thread count.

use crate::background::{Background, background};
use crate::blur::blur;
use crate::camera::{Pose, SceneCamera};
use crate::page::Texture;
use crate::sheet::Sheet;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use scancraft_core::CoreError;
use scancraft_core::image::{BAND_ROWS, bands};

/// Samples per pixel along each axis.
const SUB: usize = 3;
const MAX_MESH: usize = 900;

#[derive(Clone, Copy)]
struct Vertex {
    x: f64,
    y: f64,
    /// 1/Z, u/Z and v/Z, which interpolate linearly across a triangle in the photo.
    iz: f64,
    uz: f64,
    vz: f64,
}

struct Triangle {
    v: [Vertex; 3],
    /// Mip level for this triangle.
    lod: f32,
    y_min: f64,
    y_max: f64,
}

/// The page as the camera sees it: its colour (0–255) where it is, and how much of each pixel it covers.
pub struct PageLayer {
    pub width: usize,
    pub height: usize,
    pub colour: Vec<[f32; 3]>,
    pub coverage: Vec<f32>,
}

/// The projected length of the sheet's middle line in each direction, in pixels.
fn path_lengths(pose: &Pose, cam: &SceneCamera, sheet: &Sheet) -> Option<(f64, f64)> {
    let along = |f: &dyn Fn(f64) -> (f64, f64)| -> Option<f64> {
        let mut last = None;
        let mut len = 0.0;
        for i in 0..=64 {
            let (u, v) = f(f64::from(i) / 64.0);
            let p = pose.project(cam, sheet.point(u, v))?;
            if let Some(q) = last {
                len += crate::metrics::distance(p, q);
            }
            last = Some(p);
        }
        Some(len)
    };
    // Longer of the middle line and the two edges, so a strongly tilted page isn't under-meshed.
    let lu = [0.0, 0.5, 1.0].iter().filter_map(|&v| along(&|t| (t, v))).fold(0.0, f64::max);
    let lv = [0.0, 0.5, 1.0].iter().filter_map(|&u| along(&|t| (u, t))).fold(0.0, f64::max);
    (lu > 0.0 && lv > 0.0).then_some((lu, lv))
}

fn build_triangles(cam: &SceneCamera, pose: &Pose, sheet: &Sheet, texture: &Texture) -> Result<Vec<Triangle>, CoreError> {
    let (lu, lv) = path_lengths(pose, cam, sheet).ok_or_else(|| CoreError::Scene("the sheet is not in front of the camera".into()))?;
    let (nu, nv) = (((lu / 2.0).ceil() as usize).clamp(8, MAX_MESH), ((lv / 2.0).ceil() as usize).clamp(8, MAX_MESH));
    let mut verts: Vec<Option<Vertex>> = Vec::with_capacity((nu + 1) * (nv + 1));
    for j in 0..=nv {
        for i in 0..=nu {
            let (u, v) = (i as f64 / nu as f64, j as f64 / nv as f64);
            let q = pose.to_camera(sheet.point(u, v));
            verts.push(cam.project(q).map(|p| Vertex { x: p.x, y: p.y, iz: 1.0 / q[2], uz: u / q[2], vz: v / q[2] }));
        }
    }
    let (tw, th) = texture.size();
    let texel_area = f64::from(tw) * f64::from(th);
    let mut tris = Vec::with_capacity(nu * nv * 2);
    let at = |i: usize, j: usize| verts[j * (nu + 1) + i];
    for j in 0..nv {
        for i in 0..nu {
            for corners in [[(i, j), (i + 1, j), (i, j + 1)], [(i + 1, j), (i + 1, j + 1), (i, j + 1)]] {
                let [Some(a), Some(b), Some(c)] = corners.map(|(x, y)| at(x, y)) else { continue };
                let area = ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() / 2.0;
                if area.partial_cmp(&1e-9) != Some(std::cmp::Ordering::Greater) {
                    continue;
                }
                // Page area in texels over screen area in subsample areas gives the mip level.
                let (ua, ub, uc) = ((a.uz / a.iz, a.vz / a.iz), (b.uz / b.iz, b.vz / b.iz), (c.uz / c.iz, c.vz / c.iz));
                let uv_area = ((ub.0 - ua.0) * (uc.1 - ua.1) - (uc.0 - ua.0) * (ub.1 - ua.1)).abs() / 2.0;
                let lod = (0.5 * (uv_area * texel_area / (area * (SUB * SUB) as f64)).max(1e-9).log2()).max(0.0) as f32;
                tris.push(Triangle { v: [a, b, c], lod, y_min: a.y.min(b.y).min(c.y), y_max: a.y.max(b.y).max(c.y) });
            }
        }
    }
    Ok(tris)
}

/// Rasterises the sheet into a `width × height` photo.
pub fn rasterize(cam: &SceneCamera, pose: &Pose, sheet: &Sheet, texture: &Texture) -> Result<PageLayer, CoreError> {
    let (width, height) = (cam.width as usize, cam.height as usize);
    let tris = build_triangles(cam, pose, sheet, texture)?;
    let band_ranges: Vec<std::ops::Range<u32>> = bands(cam.height as u32).collect();
    // The triangles that touch each band, in triangle order.
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); band_ranges.len()];
    for (k, t) in tris.iter().enumerate() {
        let first = (t.y_min.max(0.0) / f64::from(BAND_ROWS)).floor() as usize;
        let last = ((t.y_max / f64::from(BAND_ROWS)).floor().max(0.0) as usize).min(band_ranges.len().saturating_sub(1));
        for list in lists.iter_mut().take(last + 1).skip(first) {
            list.push(k as u32);
        }
    }
    let parts: Vec<(Vec<[f32; 3]>, Vec<f32>)> = (0..band_ranges.len())
        .into_par_iter()
        .map(|b| {
            let range = band_ranges[b].clone();
            raster_band(&tris, &lists[b], range, width, texture)
        })
        .collect();
    let (mut colour, mut coverage) = (Vec::with_capacity(width * height), Vec::with_capacity(width * height));
    for (c, v) in parts {
        colour.extend(c);
        coverage.extend(v);
    }
    Ok(PageLayer { width, height, colour, coverage })
}

fn raster_band(tris: &[Triangle], list: &[u32], rows: std::ops::Range<u32>, width: usize, texture: &Texture) -> (Vec<[f32; 3]>, Vec<f32>) {
    let (y0, y1) = (rows.start as usize, rows.end as usize);
    let n = (y1 - y0) * width * SUB * SUB;
    // Per sample: the best 1/Z so far and what it saw.
    let mut best = vec![0f64; n];
    let mut seen: Vec<(f32, f32, f32)> = vec![(0.0, 0.0, 0.0); n];
    for &k in list {
        let t = &tris[k as usize];
        let [a, b, c] = t.v;
        let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
        if det == 0.0 {
            continue;
        }
        let (min_x, max_x) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
        let px0 = (min_x.floor().max(0.0)) as usize;
        let px1 = ((max_x.ceil().max(0.0)) as usize).min(width.saturating_sub(1));
        let py0 = (t.y_min.floor().max(y0 as f64)) as usize;
        let py1 = ((t.y_max.ceil().max(0.0)) as usize).min(y1.saturating_sub(1));
        for py in py0..=py1 {
            for px in px0..=px1 {
                for sy in 0..SUB {
                    for sx in 0..SUB {
                        let (x, y) = (px as f64 + (sx as f64 + 0.5) / SUB as f64, py as f64 + (sy as f64 + 0.5) / SUB as f64);
                        let w0 = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / det;
                        let w1 = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / det;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let iz = w0 * a.iz + w1 * b.iz + w2 * c.iz;
                        let slot = ((py - y0) * width + px) * SUB * SUB + sy * SUB + sx;
                        if iz > best[slot] {
                            best[slot] = iz;
                            let (u, v) = ((w0 * a.uz + w1 * b.uz + w2 * c.uz) / iz, (w0 * a.vz + w1 * b.vz + w2 * c.vz) / iz);
                            seen[slot] = (u as f32, v as f32, t.lod);
                        }
                    }
                }
            }
        }
    }
    let mut colour = Vec::with_capacity((y1 - y0) * width);
    let mut coverage = Vec::with_capacity((y1 - y0) * width);
    for pixel in 0..(y1 - y0) * width {
        let (mut sum, mut hits) = ([0f32; 3], 0u32);
        for s in 0..SUB * SUB {
            let slot = pixel * SUB * SUB + s;
            if best[slot] > 0.0 {
                let (u, v, lod) = seen[slot];
                let c = texture.sample(f64::from(u), f64::from(v), lod);
                for k in 0..3 {
                    sum[k] += c[k];
                }
                hits += 1;
            }
        }
        if hits == 0 {
            colour.push([0.0; 3]);
        } else {
            colour.push(sum.map(|v| v / hits as f32));
        }
        coverage.push(hits as f32 / (SUB * SUB) as f32);
    }
    (colour, coverage)
}

/// The photo: the page layer over the background, with a soft contact shadow where the page meets
/// it (stronger for white on white, where little else separates them).
pub fn composite(layer: &PageLayer, kind: Background, seed: u64) -> Vec<[f32; 3]> {
    let (w, h) = (layer.width, layer.height);
    let cov: Vec<[f32; 1]> = layer.coverage.iter().map(|&c| [c]).collect();
    // The shadow falls a little down and to the right, blurred over a few pixels.
    let shadow = blur(&cov, w, h, 4.0, (-3, -4), Some([0.0]));
    let strength = if kind == Background::WhiteOnWhite { 0.42 } else { 0.22 };
    let rows: Vec<Vec<[f32; 3]>> = (0..h)
        .into_par_iter()
        .map(|y| {
            (0..w)
                .map(|x| {
                    let k = y * w + x;
                    let c = layer.coverage[k];
                    let bg = background(kind, x as f64 + 0.5, y as f64 + 0.5, w as f64, h as f64, seed);
                    let dark = 1.0 - strength * shadow[k][0] * (1.0 - c);
                    let page = layer.colour[k];
                    [0, 1, 2].map(|i| c * page[i] + (1.0 - c) * bg[i] * dark)
                })
                .collect()
        })
        .collect();
    rows.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Placement, place};
    use crate::page::{Canvas, PAPER};
    use crate::sheet::{A4, BendKind};
    use scancraft_core::determinism::Xorshift64Star;

    fn texture() -> Texture {
        let mut c = Canvas::new(A4, PAPER).unwrap();
        c.fill_rect((100.0, 100.0, 300.0, 400.0), [0, 0, 0], true);
        c.into_texture()
    }

    fn setup(sheet: &Sheet, tilt: f64) -> (SceneCamera, Pose) {
        let cam = SceneCamera { width: 640.0, height: 480.0, f: 0.9 };
        let p = Placement { tilt, azimuth: 0.7, roll: 0.2, coverage: 0.8, shift: (0.0, 0.0) };
        let pose = place(&cam, &p, &|u, v| sheet.point(u, v)).unwrap();
        (cam, pose)
    }

    #[test]
    fn a_flat_page_covers_its_projected_quad() {
        let sheet = Sheet::flat(A4);
        let (cam, pose) = setup(&sheet, 0.5);
        let layer = rasterize(&cam, &pose, &sheet, &texture()).unwrap();
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| pose.project(&cam, sheet.point(u, v)).unwrap());
        // The covered area is the quad's area, to within the pixels the edge crosses.
        let covered: f64 = layer.coverage.iter().map(|&c| f64::from(c)).sum();
        let want = crate::geom::polygon_area(&corners);
        assert!((covered - want).abs() < 0.004 * want, "{covered} vs {want}");
        // The middle of the page is the paper colour; outside it there is no coverage.
        let mid = (320 + 240 * 640) as usize;
        assert!(layer.coverage[mid] > 0.99);
        assert_eq!(layer.coverage[0], 0.0);
    }

    #[test]
    fn the_page_is_drawn_in_the_right_place() {
        // The black box's centre, projected, is black in the photo; paper beside it is paper.
        let sheet = Sheet::flat(A4);
        let (cam, pose) = setup(&sheet, 0.4);
        let layer = rasterize(&cam, &pose, &sheet, &texture()).unwrap();
        let centre = pose.project(&cam, sheet.point(200.0 / A4.0, 250.0 / A4.1)).unwrap();
        let k = centre.y as usize * 640 + centre.x as usize;
        assert!(layer.colour[k][0] < 10.0, "{:?}", layer.colour[k]);
        let beside = pose.project(&cam, sheet.point(450.0 / A4.0, 250.0 / A4.1)).unwrap();
        let k = beside.y as usize * 640 + beside.x as usize;
        assert!((layer.colour[k][0] - 243.0).abs() < 3.0, "{:?}", layer.colour[k]);
    }

    #[test]
    fn a_bent_page_renders_without_holes() {
        for kind in [BendKind::BookPage, BendKind::Curl, BendKind::FoldedInThree, BendKind::Spread] {
            let sheet = Sheet::random(kind, &mut Xorshift64Star::new(11));
            let (cam, pose) = setup(&sheet, 0.3);
            let layer = rasterize(&cam, &pose, &sheet, &texture()).unwrap();
            // A hole would be a pixel of partial coverage inside the page: count pixels that are
            // neither covered nor empty whose four neighbours are all covered.
            let c = |x: usize, y: usize| layer.coverage[y * 640 + x];
            let mut holes = 0;
            for y in 1..479 {
                for x in 1..639 {
                    if c(x, y) < 0.99 && c(x - 1, y) > 0.99 && c(x + 1, y) > 0.99 && c(x, y - 1) > 0.99 && c(x, y + 1) > 0.99 {
                        holes += 1;
                    }
                }
            }
            assert!(holes < 20, "{kind:?}: {holes} holes");
            assert!(layer.coverage.iter().filter(|&&c| c > 0.99).count() > 30_000, "{kind:?}");
        }
    }

    #[test]
    fn the_nearer_part_of_a_folded_sheet_hides_the_farther() {
        // A strongly rising book page folds over part of itself seen from the side: no panic and a
        // coverage that never exceeds one.
        let sheet = Sheet::random(BendKind::BookPage, &mut Xorshift64Star::new(2));
        let (cam, pose) = setup(&sheet, 0.45);
        let layer = rasterize(&cam, &pose, &sheet, &texture()).unwrap();
        assert!(layer.coverage.iter().all(|&c| (0.0..=1.0).contains(&c)));
    }

    #[test]
    fn the_contact_shadow_darkens_the_background_beside_the_page() {
        let sheet = Sheet::flat(A4);
        let (cam, pose) = setup(&sheet, 0.2);
        let layer = rasterize(&cam, &pose, &sheet, &texture()).unwrap();
        let photo = composite(&layer, Background::LightDesk, 3);
        // Find a background pixel right next to the page and one far from it.
        let right_edge = (0..640).rev().find(|&x| layer.coverage[240 * 640 + x] > 0.99).unwrap();
        let near = photo[240 * 640 + right_edge + 4][0];
        let far = background(Background::LightDesk, (right_edge + 4) as f64 + 0.5, 240.5, 640.0, 480.0, 3)[0];
        assert!(near < far - 2.0, "{near} vs {far}");
    }

    #[test]
    fn one_and_eight_threads_agree() {
        let sheet = Sheet::random(BendKind::Curl, &mut Xorshift64Star::new(6));
        let (cam, pose) = setup(&sheet, 0.3);
        let tex = texture();
        let run = |n| {
            rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| {
                let l = rasterize(&cam, &pose, &sheet, &tex).unwrap();
                composite(&l, Background::Wood, 1)
            })
        };
        assert_eq!(run(1), run(8));
    }
}
