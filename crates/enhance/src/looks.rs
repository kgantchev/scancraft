//! A17 (Color Document, Grayscale) and A18 (Black & White).
//!
//! The looks all work on the page Even Light has corrected: the sharpening and the tone curve of
//! A17, and the black and white rule of A18, which compares every pixel with the midpoint between
//! the lightest value nearby (the paper) and the darkest value very nearby (the stroke).

use crate::EnhanceError;
use crate::even_light::EvenLight;
use crate::helpers::{bayer8, bilinear, bits_to_plane};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSlice, ParallelSliceMut};
use scancraft_core::image::{BitPlane, Plane, PlaneF32, RgbF32};
use scancraft_raster::RasterError;
use scancraft_raster::blur::gaussian;
use scancraft_raster::colour::luma_plane;
use scancraft_raster::helpers::{connected_components, unsharp};
use scancraft_raster::ink::midpoint_ink;

/// The strength at which A17's "low strengths" rule stops blending.
const LOW_STRENGTH: u8 = 20;
/// The share of the page a component must cover to count as a photo region (A18 step 6).
const PHOTO_SHARE: f64 = 0.005;

/// The Color Document look: the page with the document's tone curve and its colours kept.
pub fn color_document(light: &EvenLight, strength: u8, dpi: f64) -> Result<RgbF32, EnhanceError> {
    check(light, strength, dpi)?;
    if strength == 0 {
        // A strength of 0 changes nothing: the page comes back as it came in.
        return Ok(light.warped.clone());
    }
    if strength < LOW_STRENGTH {
        // Strengths 1 to 19 blend the strength-20 result with the warped page, so the slider has
        // no jump near zero.
        let twenty = document(light, LOW_STRENGTH, dpi)?;
        return blend(&light.warped, &twenty, f32::from(strength) / f32::from(LOW_STRENGTH));
    }
    document(light, strength, dpi)
}

/// The Grayscale look: the same steps with sat = 0, so the output is the tone curve T.
pub fn grayscale(light: &EvenLight, strength: u8, dpi: f64) -> Result<PlaneF32, EnhanceError> {
    check(light, strength, dpi)?;
    if strength == 0 {
        return Ok(luma_plane(&light.warped)?);
    }
    if strength < LOW_STRENGTH {
        let twenty = tone_curve(&sharpened_luma(light, dpi)?.sharp, LOW_STRENGTH)?;
        let warped = luma_plane(&light.warped)?;
        return blend_planes(&warped, &twenty, f32::from(strength) / f32::from(LOW_STRENGTH));
    }
    tone_curve(&sharpened_luma(light, dpi)?.sharp, strength)
}

/// The Black & White look: one bit per pixel, 1 meaning ink.
pub fn black_white(light: &EvenLight, strength: u8, dpi: f64) -> Result<BitPlane, EnhanceError> {
    check(light, strength, dpi)?;
    let luma = luma_plane(&light.corrected)?;
    // Steps 1 to 5: the lightest/darkest midpoint rule.
    let mut ink = midpoint_ink(&luma, dpi, strength)?;
    // Step 6: in the photo regions, an 8 × 8 Bayer ordered dither of the Grayscale look.
    let photos = photo_regions(light)?;
    if photos.data().iter().any(|&byte| byte != 0) {
        let tone = tone_curve(&sharpened_luma(light, dpi)?.sharp, strength)?;
        let matrix = bayer8();
        for y in 0..ink.height() {
            for x in 0..ink.width() {
                if photos.get(x, y) != Some(true) {
                    continue;
                }
                let threshold = (matrix.get((y % 8) as usize).and_then(|row| row.get((x % 8) as usize)).copied().unwrap_or(0.0) + 0.5) / 64.0;
                ink.set(x, y, tone.get(x, y).unwrap_or(1.0) < threshold);
            }
        }
    }
    // Step 7: the despeckle.
    despeckle(&mut ink, dpi)?;
    Ok(ink)
}

/// The checks every look makes: the strength and the density are in range, and the pieces of the
/// Even Light result belong together.
fn check(light: &EvenLight, strength: u8, dpi: f64) -> Result<(), EnhanceError> {
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(RasterError::BadParameter("the page's density must be a positive number").into());
    }
    if strength > 100 {
        return Err(RasterError::BadParameter("the strength must be in 0..=100").into());
    }
    let (w, h) = (light.corrected.width(), light.corrected.height());
    if light.warped.width() != w || light.warped.height() != h {
        return Err(RasterError::BadParameter("the warped page and the corrected page must have the same size").into());
    }
    if light.objects.width() != light.analysis.0 || light.objects.height() != light.analysis.1 {
        return Err(RasterError::BadParameter("the object mask must have the analysis copy's size").into());
    }
    Ok(())
}

/// The luma of the corrected page and its unsharp mask: A17 step 1.
struct Sharpened {
    /// Y, the luma of n.
    y: PlaneF32,
    /// Y_s, the unsharp mask of Y (σ = 0.8 px at 300 DPI, amount 0.5, threshold 0.01).
    sharp: PlaneF32,
}

/// A17 step 1: the luma of the corrected page, and its unsharp mask.
fn sharpened_luma(light: &EvenLight, dpi: f64) -> Result<Sharpened, EnhanceError> {
    let y = luma_plane(&light.corrected)?;
    let sigma = (0.8 * dpi / 300.0).max(0.3) as f32;
    let sharp = unsharp(&y, sigma, 0.5, 0.01)?;
    Ok(Sharpened { y, sharp })
}

/// A17 step 2: the tone curve T = clamp((Y_s − bp) / (wp − bp), 0, 1)^γ, with bp = 0.42 k,
/// wp = 1 − 0.05 k and γ = 1 + 0.67 k. At strength 60 these are 0.25, 0.97 and 1.4.
fn tone_curve(sharp: &PlaneF32, strength: u8) -> Result<PlaneF32, EnhanceError> {
    let k = f32::from(strength) / 100.0;
    let (bp, wp, gamma) = (0.42 * k, 1.0 - 0.05 * k, 1.0 + 0.67 * k);
    let span = (wp - bp).max(1e-6);
    let mut data = vec![0f32; sharp.data().len()];
    data.par_chunks_mut(sharp.width().max(1) as usize).zip(sharp.data().par_chunks(sharp.width().max(1) as usize)).for_each(|(out, input)| {
        for (slot, &v) in out.iter_mut().zip(input) {
            *slot = ((v - bp) / span).clamp(0.0, 1.0).powf(gamma);
        }
    });
    Ok(Plane::from_vec(sharp.width(), sharp.height(), data)?)
}

/// The smoothstep of A17 step 3: 0 below `e0`, 1 above `e1`.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A17 steps 1 to 4 at one strength.
fn document(light: &EvenLight, strength: u8, dpi: f64) -> Result<RgbF32, EnhanceError> {
    let k = f32::from(strength) / 100.0;
    let saturation = 1.0 + 0.33 * k;
    let sharpened = sharpened_luma(light, dpi)?;
    let tone = tone_curve(&sharpened.sharp, strength)?;
    let (pw, ph) = (light.corrected.width(), light.corrected.height());
    // Step 4's weight m: O blurred with σ = 2 px at the analysis scale, scaled up to the page.
    let photo = bilinear(&gaussian(&bits_to_plane(&light.objects)?, 2.0)?, pw, ph)?;
    let mut data = vec![[0f32; 3]; pw as usize * ph as usize];
    data.par_chunks_mut(pw as usize).enumerate().for_each(|(row, out)| {
        for (x, slot) in out.iter_mut().enumerate() {
            let i = row * pw as usize + x;
            let n = light.corrected.data().get(i).copied().unwrap_or([0.0; 3]);
            let y = sharpened.y.data().get(i).copied().unwrap_or(0.0);
            let t = tone.data().get(i).copied().unwrap_or(0.0);
            let m = photo.data().get(i).copied().unwrap_or(0.0);
            // Step 3: real colour keeps its strength, while faint tints vanish, so paper and black
            // ink become neutral. n_c − Y is the colour's distance from grey; the sharpening
            // added the same amount to n_c and to Y, so it cancels.
            let w = smoothstep(0.04, 0.12, n[0].max(n[1]).max(n[2]) - n[0].min(n[1]).min(n[2]));
            for (value, &channel) in slot.iter_mut().zip(&n) {
                let out = (t + saturation * w * (channel - y)).clamp(0.0, 1.0);
                // Step 4: the photo regions keep their corrected colours, without the document curve.
                *value = (1.0 - m) * out + m * channel.clamp(0.0, 1.0);
            }
        }
    });
    Ok(RgbF32::from_vec(pw, ph, data)?)
}

/// The photo regions of A18 step 6: the components of O that cover at least half a percent of the
/// page, at the page's resolution.
fn photo_regions(light: &EvenLight) -> Result<BitPlane, EnhanceError> {
    let (aw, ah) = light.analysis;
    let least = (PHOTO_SHARE * f64::from(aw) * f64::from(ah)) as u64;
    let (labels, count) = connected_components(&light.objects)?;
    let mut sizes = vec![0u64; count as usize + 1];
    for &label in labels.data() {
        if let Some(slot) = sizes.get_mut(label as usize) {
            *slot += 1;
        }
    }
    let mut mask = BitPlane::new(aw, ah)?;
    for y in 0..ah {
        for x in 0..aw {
            let label = labels.get(x, y).unwrap_or(0) as usize;
            if label != 0 && sizes.get(label).copied().unwrap_or(0) >= least {
                mask.set(x, y, true);
            }
        }
    }
    // The mask is scaled to the page's resolution, and its boundary kept sharp.
    let (pw, ph) = (light.corrected.width(), light.corrected.height());
    let scaled = bilinear(&bits_to_plane(&mask)?, pw, ph)?;
    let mut out = BitPlane::new(pw, ph)?;
    for y in 0..ph {
        for x in 0..pw {
            if scaled.get(x, y).unwrap_or(0.0) >= 0.5 {
                out.set(x, y, true);
            }
        }
    }
    Ok(out)
}

/// A18 step 7: the despeckle. A speck of ink of at most round(3 · (D/300)²) px with no other ink
/// within round(2 · D/300) px is removed.
fn despeckle(ink: &mut BitPlane, dpi: f64) -> Result<(), EnhanceError> {
    let (w, h) = (ink.width(), ink.height());
    let scale = dpi / 300.0;
    let least = (3.0 * scale * scale).round() as usize;
    let near = (2.0 * scale).round() as i64;
    let (labels, count) = connected_components(ink)?;
    let mut pixels: Vec<Vec<(u32, u32)>> = vec![Vec::new(); count as usize + 1];
    for y in 0..h {
        for x in 0..w {
            let label = labels.get(x, y).unwrap_or(0) as usize;
            if label != 0
                && let Some(list) = pixels.get_mut(label)
            {
                list.push((x, y));
            }
        }
    }
    for (label, list) in pixels.iter().enumerate().skip(1) {
        if list.len() > least || has_other_ink_near(&labels, list, label as u32, near) {
            continue;
        }
        for &(x, y) in list {
            ink.set(x, y, false);
        }
    }
    Ok(())
}

/// Does any pixel within `near` px of a pixel of the speck carry another label?
fn has_other_ink_near(labels: &Plane<u32>, speck: &[(u32, u32)], label: u32, near: i64) -> bool {
    speck.iter().any(|&(x, y)| {
        (-near..=near).any(|dy| {
            (-near..=near).any(|dx| {
                let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                let (Ok(nx), Ok(ny)) = (u32::try_from(nx), u32::try_from(ny)) else { return false };
                labels.get(nx, ny).is_some_and(|other| other != 0 && other != label)
            })
        })
    })
}

/// The blend of two colours: `t` of the way from `a` to `b`.
fn blend(a: &RgbF32, b: &RgbF32, t: f32) -> Result<RgbF32, EnhanceError> {
    let data: Vec<[f32; 3]> = a.data().iter().zip(b.data()).map(|(x, y)| [0, 1, 2].map(|c| x[c] + (y[c] - x[c]) * t)).collect();
    Ok(RgbF32::from_vec(a.width(), a.height(), data)?)
}

/// The blend of two planes.
fn blend_planes(a: &PlaneF32, b: &PlaneF32, t: f32) -> Result<PlaneF32, EnhanceError> {
    let data: Vec<f32> = a.data().iter().zip(b.data()).map(|(&x, &y)| x + (y - x) * t).collect();
    Ok(Plane::from_vec(a.width(), a.height(), data)?)
}

#[cfg(test)]
mod tests {
    use super::despeckle;
    use scancraft_core::image::BitPlane;

    /// A 40 × 40 plane with ink at the given pixels.
    fn ink(pixels: &[(u32, u32)]) -> BitPlane {
        let mut bits = BitPlane::new(40, 40).unwrap();
        for &(x, y) in pixels {
            bits.set(x, y, true);
        }
        bits
    }

    fn count(bits: &BitPlane) -> usize {
        (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| bits.get(x, y) == Some(true)).count()
    }

    #[test]
    fn a_speck_of_three_pixels_or_fewer_with_no_ink_near_it_is_removed() {
        // At 300 DPI: at most round(3) = 3 px, with no other ink within round(2) = 2 px.
        let mut bits = ink(&[(10, 10), (11, 10), (12, 11)]);
        despeckle(&mut bits, 300.0).unwrap();
        assert_eq!(count(&bits), 0);
        let mut single = ink(&[(5, 5)]);
        despeckle(&mut single, 300.0).unwrap();
        assert_eq!(count(&single), 0);
    }

    #[test]
    fn a_speck_of_four_pixels_is_kept() {
        let mut bits = ink(&[(10, 10), (11, 10), (12, 10), (13, 10)]);
        despeckle(&mut bits, 300.0).unwrap();
        assert_eq!(count(&bits), 4);
    }

    #[test]
    fn a_speck_with_ink_within_two_pixels_is_kept() {
        // Another stroke 2 px away (one empty pixel between them), and one 3 px away (two between).
        let mut near = ink(&[(10, 10), (12, 10), (12, 11), (12, 12), (12, 13), (12, 14)]);
        despeckle(&mut near, 300.0).unwrap();
        assert_eq!(count(&near), 6, "the dot beside a stroke stays");
        let mut far = ink(&[(10, 10), (13, 10), (13, 11), (13, 12), (13, 13), (13, 14)]);
        despeckle(&mut far, 300.0).unwrap();
        assert_eq!(count(&far), 5, "the dot three pixels from a stroke goes");
    }

    #[test]
    fn the_limits_scale_with_the_density() {
        // At 600 DPI a speck may be round(3 · 4) = 12 px, with no other ink within round(4) = 4 px.
        let twelve: Vec<(u32, u32)> = (0..12).map(|i| (10 + i, 10)).collect();
        let mut bits = ink(&twelve);
        despeckle(&mut bits, 600.0).unwrap();
        assert_eq!(count(&bits), 0);
        // At 150 DPI a single pixel is round(0.75) = 1 px, and 2 px are kept.
        let mut two = ink(&[(5, 5), (6, 5)]);
        despeckle(&mut two, 150.0).unwrap();
        assert_eq!(count(&two), 2);
    }
}
