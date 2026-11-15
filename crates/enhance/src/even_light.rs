//! A16: Even Light (specification A16).
//!
//! The steps EL1–EL10 of the specification, in order. A photographed page is the paper's own
//! lightness multiplied by the light falling on it. Even Light estimates how bright the bare paper
//! would look at every point, using only the pixels it is confident are bare paper, and divides the
//! photo by that estimate: the paper comes out evenly white and the ink keeps its darkness against
//! the paper around it.

use crate::EnhanceError;
use crate::helpers::{
    bilinear, bits_to_plane, blur_rgb, box_mean, channel, map3, median, median_colour, octagon_dilate, percentile, plane_to_bits, sobel,
};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
use scancraft_core::image::{BitPlane, Plane, PlaneF32, RgbF32};
use scancraft_raster::RasterError;
use scancraft_raster::blur::gaussian;
use scancraft_raster::colour::luma_plane;
use scancraft_raster::downscale::{area_downscale, area_downscale_rgb, fit_long_side};
use scancraft_raster::helpers::{connected_components, max_filter, median3, min_filter, octagon_close};

/// The long side of the analysis copy (EL1).
const ANALYSIS_LONG_SIDE: u32 = 1000;
/// The window of EL2's local mean.
const EL2_WINDOW: u32 = 31;
/// The smallest component EL4 looks at.
const EL4_MIN_AREA: u64 = 12;
/// The largest share of the page a component may cover and still be looked at (EL4).
const EL4_MAX_SHARE: f64 = 0.30;
/// The smallest share of the page an EL5 component must cover.
const EL5_MIN_SHARE: f64 = 0.001;
/// The share of the darkest pixels EL10 takes.
const EL10_SHARE: f64 = 0.02;
/// The most components EL4 and EL5 each examine, largest first. A page of noise can break into
/// hundreds of thousands of specks; the objects that matter are the big ones, and the examination
/// of each one costs a filter pass over its neighbourhood.
const MAX_COMPONENTS: usize = 512;

/// A16's output: the lighting-corrected page and the object mask O (photos, logos, fingers).
#[derive(Debug, Clone, PartialEq)]
pub struct EvenLight {
    /// n, the lighting-corrected page, at the page's own resolution.
    pub corrected: RgbF32,
    /// At the analysis copy's size (long side 1000 px).
    pub objects: BitPlane,
    /// The analysis copy's size, so that O can be scaled to the page.
    pub analysis: (u32, u32),
    /// The page's density.
    pub dpi: f64,
    /// The warped page as it came in. A17's strength 0 returns it unchanged.
    pub warped: RgbF32,
}

/// The analysis copy of EL1: the page reduced to a long side of 1000 px and blurred with σ = 0.6 px.
struct Analysis {
    /// The three channels of the copy.
    channels: [PlaneF32; 3],
    /// Its luma, Y.
    y: PlaneF32,
    /// The copy's size.
    size: (u32, u32),
}

/// The masks of EL6.
struct Masks {
    /// O, the objects.
    objects: BitPlane,
    /// O_d, the objects dilated by an octagon of radius r + 2.
    objects_d: BitPlane,
    /// The paper: neither ink nor object.
    paper: BitPlane,
    /// The lit paper: the pixels whose luma(b0) is at or above its P85.
    lit: Vec<bool>,
}

/// `page` is the warped page at output resolution; `dpi` its density.
pub fn even_light(page: &RgbF32, dpi: f64, keep_paper_colour: bool) -> Result<EvenLight, EnhanceError> {
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(RasterError::BadParameter("the page's density must be a positive number").into());
    }
    let analysis = el1(page)?;
    let radius = el2(&analysis)?;
    let b0 = el3(&analysis, radius)?;
    let luma_b0 = luma_of(&b0)?;

    // EL4 and EL5: the dark and the warm objects.
    let dark = dark_objects(&b0, &luma_b0, analysis.size)?;
    let warm = warm_objects(&b0, &luma_b0, analysis.size)?;

    let masks = el6(&analysis, &luma_b0, &dark, &warm, radius)?;
    let mut background = el7(&analysis, &masks.paper, radius)?;
    el8(&mut background, &b0, &masks.objects_d);
    let mut corrected = el9(page, &background)?;
    if keep_paper_colour {
        keep_paper(&mut corrected, &background, &masks.lit)?;
    }
    el10(&mut corrected)?;

    Ok(EvenLight { corrected, objects: masks.objects, analysis: analysis.size, dpi, warped: page.clone() })
}

/// EL1: the analysis copy, a long side of 1000 px, blurred with σ = 0.6 px. Y is its luma.
fn el1(page: &RgbF32) -> Result<Analysis, EnhanceError> {
    let (pw, ph) = (page.width(), page.height());
    let (aw, ah) = fit_long_side(pw, ph, ANALYSIS_LONG_SIDE);
    let copy = blur_rgb(&area_downscale_rgb(page, aw, ah)?, 0.6)?;
    let channels = map3(&[0usize, 1, 2], |&c| channel(&copy, c))?;
    let y = luma_plane(&copy)?;
    Ok(Analysis { channels, y, size: (aw, ah) })
}

/// EL2: the stroke width, from the runs of the marks that are darker than their surroundings.
///
/// A pixel is marked when it is darker than 0.8 × the mean of its 31 × 31 neighbourhood. On every
/// 6th row the horizontal runs of marked pixels of 1 to 59 px are collected; s is their P75, or 3
/// when there are fewer than 50 runs, and the closing radius is 2.5 · s, between 4 and 24 px.
fn el2(analysis: &Analysis) -> Result<u32, EnhanceError> {
    let (aw, ah) = analysis.size;
    let local_mean = box_mean(&analysis.y, EL2_WINDOW)?;
    let mut runs: Vec<f32> = Vec::new();
    for y in (0..ah).step_by(6) {
        let mut run = 0u32;
        for x in 0..aw {
            let marked = analysis.y.get(x, y).unwrap_or(0.0) < 0.8 * local_mean.get(x, y).unwrap_or(0.0);
            if marked {
                run += 1;
                continue;
            }
            if (1..=59).contains(&run) {
                runs.push(run as f32);
            }
            run = 0;
        }
        if (1..=59).contains(&run) {
            runs.push(run as f32);
        }
    }
    let stroke = if runs.len() < 50 { 3.0 } else { percentile(&runs, 0.75).unwrap_or(3.0) };
    Ok(((2.5 * stroke).round() as i64).clamp(4, 24) as u32)
}

/// EL3: the first background b0, per channel: a 3 × 3 median, a closing with an octagon of radius
/// r, then a Gaussian with σ = 1.5 px. Dark marks thinner than about 2r disappear; shadows, which
/// are wide, stay.
fn el3(analysis: &Analysis, radius: u32) -> Result<[PlaneF32; 3], EnhanceError> {
    map3(&analysis.channels, |plane| -> Result<PlaneF32, EnhanceError> { Ok(gaussian(&octagon_close(&median3(plane)?, radius)?, 1.5)?) })
}

/// EL6: the masks. O is the union of the objects, O_d its dilation, the ink is what is much darker
/// than the paper around it, and the paper is what is neither ink nor object.
fn el6(analysis: &Analysis, luma_b0: &PlaneF32, dark: &BitPlane, warm: &BitPlane, radius: u32) -> Result<Masks, EnhanceError> {
    let (aw, ah) = analysis.size;
    let mut objects = BitPlane::new(aw, ah)?;
    let mut objects_d = BitPlane::new(aw, ah)?;
    let mut paper = BitPlane::new(aw, ah)?;
    for y in 0..ah {
        for x in 0..aw {
            if dark.get(x, y) == Some(true) || warm.get(x, y) == Some(true) {
                objects.set(x, y, true);
            }
        }
    }
    let dilated = octagon_dilate(&bits_to_plane(&objects)?, radius + 2)?;
    let ink = ink_mask(&analysis.y, luma_b0, analysis.size)?;
    for y in 0..ah {
        for x in 0..aw {
            let in_objects_d = dilated.get(x, y).unwrap_or(0.0) >= 0.5;
            objects_d.set(x, y, in_objects_d);
            paper.set(x, y, !in_objects_d && ink.get(x, y) != Some(true));
        }
    }
    let lit = lit_paper(luma_b0);
    Ok(Masks { objects, objects_d, paper, lit })
}

/// EL7: the paper colour under the local light, B, per channel.
///
/// At every pixel the first scale whose local paper share is high enough is used: σ1 = max(2, r/2)
/// with a share over 0.15, then 4σ1 over 0.02, then 16σ1 over 0.003. Where no scale is sure, the
/// median paper colour of the page is used.
fn el7(analysis: &Analysis, paper: &BitPlane, radius: u32) -> Result<[PlaneF32; 3], EnhanceError> {
    let (aw, ah) = analysis.size;
    let paper_float = bits_to_plane(paper)?;
    let sigma1 = (0.5 * f64::from(radius)).max(2.0);
    let scales = [(sigma1, 0.15), (4.0 * sigma1, 0.02), (16.0 * sigma1, 0.003)];
    let mut background = map3(&[(), (), ()], |_| PlaneF32::new(aw, ah, 0.0))?;
    let mut assigned = vec![false; aw as usize * ah as usize];
    for (sigma, share_limit) in scales {
        let (normalised, share) = normalized_channels(&analysis.channels, &paper_float, sigma as f32)?;
        for (i, done) in assigned.iter_mut().enumerate() {
            if *done || share.data().get(i).copied().unwrap_or(0.0) <= share_limit {
                continue;
            }
            *done = true;
            for (plane, source) in background.iter_mut().zip(normalised.iter()) {
                if let (Some(slot), Some(&v)) = (plane.data_mut().get_mut(i), source.data().get(i)) {
                    *slot = v;
                }
            }
        }
    }
    // The pixels no scale could judge: the median paper colour of the page.
    let fallback = median_paper_colour(paper, analysis)?;
    for (plane, colour) in background.iter_mut().zip(fallback) {
        for (slot, &done) in plane.data_mut().iter_mut().zip(assigned.iter()) {
            if !done {
                *slot = colour;
            }
        }
    }
    Ok(background)
}

/// EL8: keep the hard shadow edges. Outside the objects, B is the smaller of B and b0, so the
/// smooth estimate does not blur a hard shadow's edge into a halo.
fn el8(background: &mut [PlaneF32; 3], b0: &[PlaneF32; 3], objects_d: &BitPlane) {
    let width = objects_d.width().max(1) as usize;
    for (plane, first) in background.iter_mut().zip(b0.iter()) {
        for (i, (slot, &v)) in plane.data_mut().iter_mut().zip(first.data()).enumerate() {
            if objects_d.get((i % width) as u32, (i / width) as u32) != Some(true) {
                *slot = slot.min(v);
            }
        }
    }
}

/// EL9: divide. B is scaled to the page's resolution, and the page is divided by it: the paper
/// becomes 1.0 in every channel, which also takes the light's colour cast away.
fn el9(page: &RgbF32, background: &[PlaneF32; 3]) -> Result<RgbF32, EnhanceError> {
    let (pw, ph) = (page.width(), page.height());
    let scaled = map3(background, |plane| bilinear(plane, pw, ph))?;
    let mut data = vec![[0f32; 3]; pw as usize * ph as usize];
    data.par_chunks_mut(pw as usize).enumerate().for_each(|(y, row)| {
        for (x, slot) in row.iter_mut().enumerate() {
            let i = y * pw as usize + x;
            let px = page.data().get(i).copied().unwrap_or([0.0; 3]);
            for (value, (plane, &v)) in slot.iter_mut().zip(scaled.iter().zip(px.iter())) {
                let b = plane.data().get(i).copied().unwrap_or(0.0);
                *value = (v / b.max(0.02)).clamp(0.0, 1.3);
            }
        }
    });
    Ok(RgbF32::from_vec(pw, ph, data)?)
}

/// The Keep paper colour rule of EL9: the paper keeps its tint, so n_c ← n_c · P_c / max(P), where
/// P is the median of B over the lit paper.
fn keep_paper(corrected: &mut RgbF32, background: &[PlaneF32; 3], lit: &[bool]) -> Result<(), EnhanceError> {
    let p = map3(background, |plane| -> Result<f32, EnhanceError> {
        let values: Vec<f32> = lit.iter().zip(plane.data()).filter(|&(&l, _)| l).map(|(_, &v)| v).collect();
        Ok(median(&values).unwrap_or(1.0))
    })?;
    let largest = p[0].max(p[1]).max(p[2]).max(1e-6);
    for px in corrected.data_mut() {
        for (value, tint) in px.iter_mut().zip(p) {
            *value *= tint / largest;
        }
    }
    Ok(())
}

/// EL10: the neutral black. The darkest pixels of the page give the colour of the ink, and that
/// tint is taken out of the whole page, so the ink comes out black instead of the bluish or
/// brownish black a camera records.
fn el10(corrected: &mut RgbF32) -> Result<(), EnhanceError> {
    let ink_colour = darkest_colour(corrected);
    let l = scancraft_core::image::luma(ink_colour);
    let offset = ink_colour.map(|c| (c - l).clamp(-0.08, 0.08));
    for px in corrected.data_mut() {
        let yv = scancraft_core::image::luma(*px);
        let weight = ((1.0 - yv) / (1.0 - l).max(1e-6)).clamp(0.0, 1.0);
        for (value, off) in px.iter_mut().zip(offset) {
            *value -= off * weight;
        }
    }
    Ok(())
}

/// The luma of three channels.
fn luma_of(channels: &[PlaneF32; 3]) -> Result<PlaneF32, EnhanceError> {
    let [r, g, b] = channels;
    let data: Vec<f32> = r.data().iter().zip(g.data()).zip(b.data()).map(|((&rv, &gv), &bv)| 0.299 * rv + 0.587 * gv + 0.114 * bv).collect();
    Ok(Plane::from_vec(r.width(), r.height(), data)?)
}

/// The lit paper: the pixels whose luma(b0) is at or above its P85.
fn lit_paper(luma_b0: &PlaneF32) -> Vec<bool> {
    let limit = percentile(luma_b0.data(), 0.85).unwrap_or(1.0);
    luma_b0.data().iter().map(|&v| v >= limit).collect()
}

/// The ink: the pixels much darker than the paper around them, dilated by 3 × 3.
fn ink_mask(y_analysis: &PlaneF32, luma_b0: &PlaneF32, size: (u32, u32)) -> Result<BitPlane, EnhanceError> {
    let data: Vec<f32> = y_analysis.data().iter().zip(luma_b0.data()).map(|(&yv, &b)| if yv / b.max(0.02) < 0.88 { 1.0 } else { 0.0 }).collect();
    plane_to_bits(&max_filter(&Plane::from_vec(size.0, size.1, data)?, 3)?)
}

/// The median colour of the paper, per channel.
fn median_paper_colour(paper: &BitPlane, analysis: &Analysis) -> Result<[f32; 3], EnhanceError> {
    let width = analysis.size.0.max(1) as usize;
    map3(&analysis.channels, |plane| -> Result<f32, EnhanceError> {
        let values: Vec<f32> = plane
            .data()
            .iter()
            .enumerate()
            .filter(|&(i, _)| paper.get((i % width) as u32, (i / width) as u32) == Some(true))
            .map(|(_, &v)| v)
            .collect();
        Ok(median(&values).unwrap_or(1.0))
    })
}

/// A19's normalized convolution (Knutsson and Westin): (G_σ ∗ (c · I)) / (G_σ ∗ c) for each
/// channel, with the local confidence G_σ ∗ c returned as well.
///
/// The confidence is the same for every channel, so it is convolved once.
fn normalized_channels(values: &[PlaneF32; 3], confidence: &PlaneF32, sigma: f32) -> Result<([PlaneF32; 3], PlaneF32), EnhanceError> {
    let denominator = gaussian(confidence, sigma)?;
    let out = map3(values, |value| -> Result<PlaneF32, EnhanceError> {
        let weighted: Vec<f32> = confidence.data().iter().zip(value.data()).map(|(&c, &v)| c * v).collect();
        let numerator = gaussian(&Plane::from_vec(value.width(), value.height(), weighted)?, sigma)?;
        let data: Vec<f32> = numerator.data().iter().zip(denominator.data()).map(|(&n, &d)| if d > 0.0 { n / d } else { 0.0 }).collect();
        Ok(Plane::from_vec(value.width(), value.height(), data)?)
    })?;
    Ok((out, denominator))
}

/// The median colour of the darkest pixels of the page: the colour of the ink (EL10).
fn darkest_colour(page: &RgbF32) -> [f32; 3] {
    let lumas: Vec<f32> = page.data().iter().map(|px| scancraft_core::image::luma(*px)).collect();
    let limit = percentile(&lumas, EL10_SHARE).unwrap_or(0.0);
    let darkest: Vec<[f32; 3]> = page.data().iter().zip(&lumas).filter(|&(_, &l)| l <= limit).map(|(px, _)| *px).collect();
    median_colour(&darkest).unwrap_or([0.0; 3])
}

/// One connected component of a mask.
struct Component {
    /// The pixels of the component.
    pixels: Vec<(u32, u32)>,
    /// The box around the component, grown by the width of the widest band looked at.
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// The components of a label plane whose area is between `min_area` and `max_area`, the largest
/// [`MAX_COMPONENTS`] of them, largest first.
fn components(labels: &Plane<u32>, count: u32, areas: (u64, u64), margin: u32) -> Vec<Component> {
    let (w, h) = (labels.width(), labels.height());
    let mut pixels: Vec<Vec<(u32, u32)>> = vec![Vec::new(); count as usize + 1];
    for y in 0..h {
        for x in 0..w {
            let label = labels.get(x, y).unwrap_or(0);
            if label != 0
                && let Some(list) = pixels.get_mut(label as usize)
            {
                list.push((x, y));
            }
        }
    }
    let mut kept: Vec<Vec<(u32, u32)>> = pixels.into_iter().skip(1).filter(|list| (areas.0..=areas.1).contains(&(list.len() as u64))).collect();
    // Largest first; the sort is stable, so equal areas keep the order of their labels.
    kept.sort_by_key(|list| std::cmp::Reverse(list.len()));
    kept.truncate(MAX_COMPONENTS);
    kept.into_iter()
        .map(|list| {
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
            for &(x, y) in &list {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
            Component {
                pixels: list,
                x0: x0.saturating_sub(margin),
                y0: y0.saturating_sub(margin),
                x1: (x1 + margin).min(w.saturating_sub(1)),
                y1: (y1 + margin).min(h.saturating_sub(1)),
            }
        })
        .collect()
}

/// The two bands around a component.
struct Rings {
    /// The pixels within `inner` px of the component's boundary, on both sides.
    boundary: Vec<(u32, u32)>,
    /// The pixels between `outer_from` and `outer` px outside the component.
    outside: Vec<(u32, u32)>,
}

/// The bands around a component: `inner` px from the boundary on both sides, and the pixels more
/// than `outer_from` and at most `outer` px outside it.
///
/// The distances are measured with a square window, which stands in for a disk.
fn rings(component: &Component, inner: u32, outer: u32, outer_from: u32) -> Result<Rings, EnhanceError> {
    let bw = component.x1 - component.x0 + 1;
    let bh = component.y1 - component.y0 + 1;
    let mut inside = vec![0f32; bw as usize * bh as usize];
    for &(x, y) in &component.pixels {
        let index = (y - component.y0) as usize * bw as usize + (x - component.x0) as usize;
        if let Some(slot) = inside.get_mut(index) {
            *slot = 1.0;
        }
    }
    let plane = Plane::from_vec(bw, bh, inside)?;
    let near_inner = max_filter(&plane, 2 * inner + 1)?;
    let core = min_filter(&plane, 2 * inner + 1)?;
    let near_outer = max_filter(&plane, 2 * outer + 1)?;
    let near_from = max_filter(&plane, 2 * outer_from + 1)?;
    let mut boundary = Vec::new();
    let mut outside = Vec::new();
    for y in 0..bh {
        for x in 0..bw {
            let at = |p: &PlaneF32| p.get(x, y).unwrap_or(0.0) >= 0.5;
            let place = (component.x0 + x, component.y0 + y);
            if at(&near_inner) && !at(&core) {
                boundary.push(place);
            }
            if at(&near_outer) && !at(&near_from) {
                outside.push(place);
            }
        }
    }
    Ok(Rings { boundary, outside })
}

/// The values of a plane at the given pixels.
fn values_at(plane: &PlaneF32, pixels: &[(u32, u32)]) -> Vec<f32> {
    pixels.iter().filter_map(|&(x, y)| plane.get(x, y)).collect()
}

/// The colours of three planes at the given pixels.
fn colours_at(b0: &[PlaneF32; 3], pixels: &[(u32, u32)]) -> Vec<[f32; 3]> {
    let [r, g, b] = b0;
    pixels.iter().filter_map(|&(x, y)| Some([r.get(x, y)?, g.get(x, y)?, b.get(x, y)?])).collect()
}

/// The largest channel difference between two colours, each divided by its own largest channel.
fn colour_difference(inside: &[[f32; 3]], outside: &[[f32; 3]]) -> f32 {
    let (Some(a), Some(b)) = (median_colour(inside), median_colour(outside)) else {
        return 0.0;
    };
    let norm = |c: [f32; 3]| {
        let largest = c[0].max(c[1]).max(c[2]).max(1e-6);
        c.map(|v| v / largest)
    };
    norm(a).iter().zip(norm(b)).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max)
}

/// Everything EL4 measures a component against.
struct DarkContext<'a> {
    b0: &'a [PlaneF32; 3],
    luma_b0: &'a PlaneF32,
    /// The paper's level, including the large-scale shading.
    bl: PlaneF32,
    /// luma(b0) over the paper's level.
    depth: PlaneF32,
    /// The Sobel gradient of luma(b0), divided by 8.
    gradient: PlaneF32,
    size: (u32, u32),
}

/// EL4: the dark objects. The photos, the logos and the black boxes are kept out of the paper
/// estimate, so they are not mistaken for shadows and bleached.
fn dark_objects(b0: &[PlaneF32; 3], luma_b0: &PlaneF32, size: (u32, u32)) -> Result<BitPlane, EnhanceError> {
    let (w, h) = size;
    // bl: on a copy four times smaller, a closing with a radius of 12 % of the copy's long side,
    // a Gaussian with σ = 3 px, and back up bilinearly.
    let (sw, sh) = ((w / 4).max(1), (h / 4).max(1));
    let small = area_downscale(luma_b0, sw, sh)?;
    let small_radius = (0.12 * f64::from(sw.max(sh))).round() as u32;
    let bl = bilinear(&gaussian(&octagon_close(&small, small_radius)?, 3.0)?, w, h)?;
    let depth_data: Vec<f32> = luma_b0.data().iter().zip(bl.data()).map(|(&v, &b)| v / b.max(0.03)).collect();
    let depth = Plane::from_vec(w, h, depth_data)?;
    let mut below = BitPlane::new(w, h)?;
    for y in 0..h {
        for x in 0..w {
            if depth.get(x, y).unwrap_or(1.0) < 0.8 {
                below.set(x, y, true);
            }
        }
    }
    let (labels, count) = connected_components(&below)?;
    let max_area = (EL4_MAX_SHARE * f64::from(w) * f64::from(h)) as u64;
    let context = DarkContext { b0, luma_b0, bl, depth, gradient: sobel(luma_b0)?, size };
    let mut found = BitPlane::new(w, h)?;
    for component in components(&labels, count, (EL4_MIN_AREA, max_area), 6) {
        if is_dark_object(&context, &component)? {
            for &(x, y) in &component.pixels {
                found.set(x, y, true);
            }
        }
    }
    Ok(found)
}

/// Is this component of "darker than the paper's level" an object (EL4)?
///
/// It is, if its median depth is under 0.5, or if its edge is sharp (a drop in level over a short
/// distance, which a shadow's soft edge is not) and it either doesn't touch the page's border or
/// differs in colour from what surrounds it.
fn is_dark_object(c: &DarkContext, component: &Component) -> Result<bool, EnhanceError> {
    let d50 = median(&values_at(&c.depth, &component.pixels)).unwrap_or(1.0);
    if d50 < 0.5 {
        return Ok(true);
    }
    let rings = rings(component, 2, 6, 1)?;
    let drop = median(&values_at(&c.bl, &component.pixels)).unwrap_or(0.0) - median(&values_at(c.luma_b0, &component.pixels)).unwrap_or(0.0);
    let sharp = percentile(&values_at(&c.gradient, &rings.boundary), 0.75).unwrap_or(0.0) / drop.max(0.02);
    if sharp < 0.12 {
        return Ok(false);
    }
    let (w, h) = c.size;
    let touches_border = component.pixels.iter().any(|&(x, y)| x == 0 || y == 0 || x + 1 == w || y + 1 == h);
    if !touches_border {
        return Ok(true);
    }
    let cd = colour_difference(&colours_at(c.b0, &component.pixels), &colours_at(c.b0, &rings.outside));
    Ok(cd > 0.2)
}

/// Everything EL5 measures a component against.
struct WarmContext {
    /// ln(R / B) of b0 blurred with σ = 1 px.
    rb: PlaneF32,
    /// The gradient of `rb`, divided by 8.
    gradient: PlaneF32,
}

/// EL5: the warm objects — the skin, the wood and the cardboard at the page's edge.
fn warm_objects(b0: &[PlaneF32; 3], luma_b0: &PlaneF32, size: (u32, u32)) -> Result<BitPlane, EnhanceError> {
    let (w, h) = size;
    let mask = warm_candidates(b0, luma_b0, size)?;
    let (labels, count) = connected_components(&mask)?;
    let blurred = map3(b0, |plane| gaussian(plane, 1.0))?;
    let rb = log_red_over_blue(&blurred)?;
    let context = WarmContext { gradient: sobel(&rb)?, rb };
    let least = (EL5_MIN_SHARE * f64::from(w) * f64::from(h)) as u64;
    let mut found = BitPlane::new(w, h)?;
    for component in components(&labels, count, (least, u64::from(w) * u64::from(h)), 7) {
        if is_warm_object(&context, &component)? {
            for &(x, y) in &component.pixels {
                found.set(x, y, true);
            }
        }
    }
    Ok(found)
}

/// ln(max(R, 0.01) / max(B, 0.01)): how much warmer than blue each pixel is.
fn log_red_over_blue(planes: &[PlaneF32; 3]) -> Result<PlaneF32, EnhanceError> {
    let [r, _, b] = planes;
    let data: Vec<f32> = r.data().iter().zip(b.data()).map(|(&rv, &bv)| rv.max(0.01).ln() - bv.max(0.01).ln()).collect();
    Ok(Plane::from_vec(r.width(), r.height(), data)?)
}

/// The candidates of EL5: warmer and more saturated than the lit paper, opened with a 3 × 3 square.
fn warm_candidates(b0: &[PlaneF32; 3], luma_b0: &PlaneF32, size: (u32, u32)) -> Result<BitPlane, EnhanceError> {
    let (w, h) = size;
    let rb = log_red_over_blue(b0)?;
    let [r, g, b] = b0;
    let sat: Vec<f32> = r
        .data()
        .iter()
        .zip(g.data())
        .zip(b.data())
        .map(|((&rv, &gv), &bv)| {
            let largest = rv.max(gv).max(bv).max(0.01);
            (largest - rv.min(gv).min(bv)) / largest
        })
        .collect();
    let lit = lit_paper(luma_b0);
    let median_of_lit = |values: &[f32]| median(&lit.iter().zip(values).filter(|&(&l, _)| l).map(|(_, &v)| v).collect::<Vec<f32>>()).unwrap_or(0.0);
    let (median_rb, median_sat) = (median_of_lit(rb.data()), median_of_lit(&sat));
    let candidates: Vec<f32> =
        rb.data().iter().zip(&sat).map(|(&r, &s)| if r > 1.22f32.ln() + median_rb && s > 0.10 + median_sat { 1.0 } else { 0.0 }).collect();
    let opened = max_filter(&min_filter(&Plane::from_vec(w, h, candidates)?, 3)?, 3)?;
    plane_to_bits(&opened)
}

/// Is this warm component an object (EL5)? It is, if it is much warmer than what surrounds it
/// (a jump of at least 0.25 in ln(R/B)) and the edge is sharp.
fn is_warm_object(c: &WarmContext, component: &Component) -> Result<bool, EnhanceError> {
    let rings = rings(component, 3, 7, 2)?;
    let jump = median(&values_at(&c.rb, &component.pixels)).unwrap_or(0.0) - median(&values_at(&c.rb, &rings.outside)).unwrap_or(0.0);
    let sharp = percentile(&values_at(&c.gradient, &rings.boundary), 0.75).unwrap_or(0.0) / jump.abs().max(0.03);
    Ok(jump >= 0.25 && sharp >= 0.08)
}
