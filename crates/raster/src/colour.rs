//! sRGB and Oklab (specification §7.3, A19). Owner: WP2.
//!
//! Processing works on sRGB-encoded f32 values in [0, 1] (§7.3): the byte values divided by
//! 255, with no gamma change. Oklab (Ottosson 2020) is the one place that linearises: the
//! Oklab transform starts from linear light.

use crate::RasterError;
use scancraft_core::image::{Gray8, Plane, PlaneF32, Rgb8, RgbF32};

/// Ottosson's linear sRGB → LMS matrix (M1).
const M1: [[f32; 3]; 3] =
    [[0.412_221_46, 0.536_332_55, 0.051_445_995], [0.211_903_5, 0.680_699_5, 0.107_396_96], [0.088_302_46, 0.281_718_85, 0.629_978_7]];

/// Ottosson's LMS′ → Lab matrix (M2).
const M2: [[f32; 3]; 3] =
    [[0.210_454_26, 0.793_617_8, -0.004_072_047], [1.977_998_5, -2.428_592_2, 0.450_593_7], [0.025_904_037, 0.782_771_77, -0.808_675_77]];

/// The sRGB electro-optical transfer function: encoded value → linear light.
fn linearise(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// One f32 channel in [0, 1] to 8 bits, clamped and rounded. A value that is not finite is an
/// error: casting it would be undefined.
fn quantise(v: f32) -> Result<u8, RasterError> {
    if !v.is_finite() {
        return Err(RasterError::BadParameter("colour values must be finite"));
    }
    Ok((v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// sRGB 8-bit to f32 in [0, 1], still sRGB-encoded.
pub fn to_f32(src: &Rgb8) -> Result<RgbF32, RasterError> {
    let px: Vec<[f32; 3]> = src.data().iter().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
    Ok(Plane::from_vec(src.width(), src.height(), px)?)
}

/// f32 in [0, 1] to 8 bits, clamped and rounded.
pub fn to_u8(src: &RgbF32) -> Result<Rgb8, RasterError> {
    let mut px = Vec::with_capacity(src.data().len());
    for p in src.data() {
        px.push([quantise(p[0])?, quantise(p[1])?, quantise(p[2])?]);
    }
    Ok(Plane::from_vec(src.width(), src.height(), px)?)
}

/// Oklab (Ottosson 2020) of one sRGB-encoded colour.
///
/// The input stays sRGB-encoded; the transform linearises it, multiplies by M1, takes the cube
/// roots and multiplies by M2. The result is not clamped: L, a and b are unbounded by definition.
pub fn oklab(rgb: [f32; 3]) -> Result<[f32; 3], RasterError> {
    if rgb.iter().any(|v| !v.is_finite()) {
        return Err(RasterError::BadParameter("colour channels must be finite"));
    }
    let lin = [linearise(rgb[0]), linearise(rgb[1]), linearise(rgb[2])];
    let mut lms = [0f32; 3];
    for (i, row) in M1.iter().enumerate() {
        lms[i] = row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2];
    }
    // The cube root, sign preserving: out-of-gamut colours can give a negative LMS value.
    let lms = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];
    let mut lab = [0f32; 3];
    for (i, row) in M2.iter().enumerate() {
        lab[i] = row[0] * lms[0] + row[1] * lms[1] + row[2] * lms[2];
    }
    Ok(lab)
}

/// The luma of an RGB plane: Y = 0.299 R + 0.587 G + 0.114 B on sRGB-encoded values (§7.3).
///
/// A16, A17, A18 and A26 all need "the luma of n"; `scancraft_core::image::luma` gives the
/// formula for one pixel, this gives the plane.
pub fn luma_plane(src: &RgbF32) -> Result<PlaneF32, RasterError> {
    let data: Vec<f32> = src.data().iter().map(|p| scancraft_core::image::luma(*p)).collect();
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

/// One f32 channel in [0, 1] to 8 bits, clamped and rounded.
pub fn to_u8_plane(src: &PlaneF32) -> Result<Gray8, RasterError> {
    let mut data = Vec::with_capacity(src.data().len());
    for v in src.data() {
        data.push(quantise(*v)?);
    }
    Ok(Plane::from_vec(src.width(), src.height(), data)?)
}

#[cfg(test)]
mod tests {
    use super::{luma_plane, oklab, to_f32, to_u8, to_u8_plane};
    use scancraft_core::image::Plane;

    #[test]
    fn to_f32_and_to_u8_round_trip() {
        let src =
            Plane::from_vec(16, 16, (0..16u32 * 16).map(|i| [(i % 256) as u8, ((i * 7) % 256) as u8, ((i * 13) % 256) as u8]).collect()).unwrap();
        let back = to_u8(&to_f32(&src).unwrap()).unwrap();
        assert_eq!(back, src);
    }

    #[test]
    fn to_f32_keeps_the_values_srgb_encoded() {
        let src = Plane::from_vec(1, 1, vec![[128u8, 0, 255]]).unwrap();
        let f = to_f32(&src).unwrap();
        assert_eq!(f.get(0, 0), Some([128.0 / 255.0, 0.0, 1.0]));
    }

    #[test]
    fn to_u8_clamps_and_rounds() {
        let src = Plane::from_vec(3, 1, vec![[0.5, -1.0, 2.0], [0.0, 0.5, 1.0], [1.0, 0.0, 0.25]]).unwrap();
        let u = to_u8(&src).unwrap();
        assert_eq!(u.get(0, 0), Some([128, 0, 255]));
        assert_eq!(u.get(1, 0), Some([0, 128, 255]));
        assert_eq!(u.get(2, 0), Some([255, 0, 64]));
    }

    #[test]
    fn to_u8_rejects_a_value_that_is_not_finite() {
        let src = Plane::from_vec(1, 1, vec![[f32::NAN, 0.0, 0.0]]).unwrap();
        assert!(to_u8(&src).is_err());
    }

    #[test]
    fn to_u8_plane_clamps_and_rounds() {
        let src = Plane::from_vec(2, 1, vec![0.5f32, 2.0]).unwrap();
        assert_eq!(to_u8_plane(&src).unwrap().data(), &[128, 255]);
    }

    #[test]
    fn oklab_of_white_and_black() {
        let white = oklab([1.0, 1.0, 1.0]).unwrap();
        assert!((white[0] - 1.0).abs() < 1e-5, "L = {}", white[0]);
        assert!(white[1].abs() < 1e-4, "a = {}", white[1]);
        assert!(white[2].abs() < 1e-4, "b = {}", white[2]);
        let black = oklab([0.0, 0.0, 0.0]).unwrap();
        assert!(black[0].abs() < 1e-6);
        assert!(black[1].abs() < 1e-4);
        assert!(black[2].abs() < 1e-4);
    }

    #[test]
    fn oklab_of_grey_is_neutral() {
        for g in [0.0f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            let lab = oklab([g, g, g]).unwrap();
            assert!(lab[1].abs() < 1e-3, "a = {} for {g}", lab[1]);
            assert!(lab[2].abs() < 1e-3, "b = {} for {g}", lab[2]);
        }
    }

    #[test]
    fn oklab_matches_the_published_matrices() {
        // Reference values from the Oklab reference implementation (Ottosson 2020) for the sRGB
        // primaries.
        let red = oklab([1.0, 0.0, 0.0]).unwrap();
        assert!((red[0] - 0.627_955_4).abs() < 1e-6, "L = {}", red[0]);
        assert!((red[1] - 0.22486307).abs() < 1e-6, "a = {}", red[1]);
        assert!((red[2] - 0.12584628).abs() < 1e-6, "b = {}", red[2]);
        let green = oklab([0.0, 1.0, 0.0]).unwrap();
        assert!((green[0] - 0.86643962).abs() < 1e-6, "L = {}", green[0]);
        assert!((green[1] + 0.23388758).abs() < 1e-6, "a = {}", green[1]);
        assert!((green[2] - 0.17949845).abs() < 1e-6, "b = {}", green[2]);
        let blue = oklab([0.0, 0.0, 1.0]).unwrap();
        assert!((blue[0] - 0.45201372).abs() < 1e-6, "L = {}", blue[0]);
        assert!((blue[1] + 0.03245698).abs() < 1e-6, "a = {}", blue[1]);
        assert!((blue[2] + 0.31152817).abs() < 1e-6, "b = {}", blue[2]);
    }

    #[test]
    fn oklab_rejects_a_value_that_is_not_finite() {
        assert!(oklab([f32::NAN, 0.0, 0.0]).is_err());
        assert!(oklab([f32::INFINITY, 0.0, 0.0]).is_err());
    }

    #[test]
    fn luma_plane_uses_the_specification_weights() {
        let src = Plane::from_vec(1, 1, vec![[1.0f32, 0.0, 0.0]]).unwrap();
        assert!((luma_plane(&src).unwrap().get(0, 0).unwrap() - 0.299).abs() < 1e-6);
        let src = Plane::from_vec(1, 1, vec![[0.0f32, 1.0, 0.0]]).unwrap();
        assert!((luma_plane(&src).unwrap().get(0, 0).unwrap() - 0.587).abs() < 1e-6);
        let src = Plane::from_vec(1, 1, vec![[0.0f32, 0.0, 1.0]]).unwrap();
        assert!((luma_plane(&src).unwrap().get(0, 0).unwrap() - 0.114).abs() < 1e-6);
    }
}
