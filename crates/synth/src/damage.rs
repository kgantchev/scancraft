//! The damage models of §10.1: a finger over an edge or a corner, blur up to σ = 1.5 px, noise up
//! to σ = 6 levels, and a JPEG round trip at quality 85. The finger is a physical thing in the
//! scene, so it is drawn before the light; blur, noise and JPEG are the camera's, applied after.

use crate::blur::blur;
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use rayon::slice::ParallelSliceMut;
use scancraft_core::CoreError;
use scancraft_core::determinism::{Xorshift64Star, stable_hash};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{BAND_ROWS, Rgb8};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    FingerEdge,
    FingerCorner,
    Blur,
    Noise,
    Jpeg,
}

impl Damage {
    /// One of the four models (§10.1), the finger being over an edge or a corner.
    pub fn draw(rng: &mut Xorshift64Star) -> Damage {
        match rng.below(4) {
            0 => {
                if rng.below(2) == 0 {
                    Damage::FingerEdge
                } else {
                    Damage::FingerCorner
                }
            }
            1 => Damage::Blur,
            2 => Damage::Noise,
            _ => Damage::Jpeg,
        }
    }

    pub fn is_finger(self) -> bool {
        matches!(self, Damage::FingerEdge | Damage::FingerCorner)
    }

    pub fn words(self) -> &'static str {
        match self {
            Damage::FingerEdge => "a finger over an edge",
            Damage::FingerCorner => "a finger over a corner",
            Damage::Blur => "blur",
            Damage::Noise => "noise",
            Damage::Jpeg => "JPEG at quality 85",
        }
    }
}

const SKIN: [f32; 3] = [226.0, 172.0, 140.0];

/// Draws a finger of about 70 × 45 pt over an edge or a corner of the page, with a soft shadow on
/// what is beneath it. `corners` are the page's true corners in the photo and `px_per_pt` the
/// scale there. The truth keeps the page's true corners; the finger hides them from the pipeline.
pub fn apply_finger(photo: &mut [[f32; 3]], width: usize, height: usize, corners: &Quad, px_per_pt: f64, kind: Damage, rng: &mut Xorshift64Star) {
    let centre = Point::new(corners.0.iter().map(|p| p.x).sum::<f64>() / 4.0, corners.0.iter().map(|p| p.y).sum::<f64>() / 4.0);
    let anchor = if kind == Damage::FingerCorner {
        corners.0[rng.below(4) as usize]
    } else {
        let side = rng.below(4) as usize;
        let (a, b) = (corners.0[side], corners.0[(side + 1) % 4]);
        let t = 0.3 + 0.4 * rng.next_f64();
        Point::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y))
    };
    // The finger points in from outside: its long axis runs from the page's middle through the anchor.
    let (dx, dy) = (anchor.x - centre.x, anchor.y - centre.y);
    let norm = dx.hypot(dy).max(1e-9);
    let axis = (dx / norm, dy / norm);
    let (semi_long, semi_short) = (35.0 * px_per_pt, 22.5 * px_per_pt);
    let middle = Point::new(anchor.x + axis.0 * 0.25 * semi_long, anchor.y + axis.1 * 0.25 * semi_long);
    // r = 1 on the ellipse's edge; `soft` px is how wide the feathering is.
    let radius = |x: f64, y: f64, c: Point, scale: f64| {
        let (px, py) = (x - c.x, y - c.y);
        let along = (px * axis.0 + py * axis.1) / (semi_long * scale);
        let across = (-px * axis.1 + py * axis.0) / (semi_short * scale);
        (along.hypot(across), along)
    };
    let shadow_centre = Point::new(middle.x + 0.12 * semi_short, middle.y + 0.16 * semi_short);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let pixel = &mut photo[y * width + x];
            // The shadow: a larger ellipse, fading out over a few tenths of its size.
            let (rs, _) = radius(fx, fy, shadow_centre, 1.2);
            if rs < 1.4 {
                let k = 1.0 - 0.3 * (1.0 - ((rs - 0.7) / 0.7).clamp(0.0, 1.0)).powi(2);
                pixel.iter_mut().for_each(|v| *v *= k as f32);
            }
            let (r, along) = radius(fx, fy, middle, 1.0);
            let cover = ((1.0 - r) * semi_short.min(semi_long) / 1.5 + 0.5).clamp(0.0, 1.0) as f32;
            if cover > 0.0 {
                // A little lighter at the tip, darker toward the base.
                let shade = (0.9 + 0.1 * (0.5 - along * 0.5).clamp(0.0, 1.0)) as f32;
                for c in 0..3 {
                    pixel[c] = pixel[c] * (1.0 - cover) + SKIN[c] * shade * cover;
                }
            }
        }
    }
}

/// Gaussian noise of σ `sigma` levels on every channel, from one stream per 64-row band so the
/// result doesn't depend on the thread count.
pub fn add_noise(photo: &mut [[f32; 3]], width: usize, sigma: f64, seed: u64) {
    let band = BAND_ROWS as usize * width;
    photo.par_chunks_mut(band.max(1)).enumerate().for_each(|(b, chunk)| {
        let mut rng = Xorshift64Star::new(stable_hash(&[seed.to_le_bytes().as_slice(), (b as u64).to_le_bytes().as_slice()].concat()));
        for px in chunk.iter_mut() {
            for v in px.iter_mut() {
                let (a, c) = (rng.next_f64().max(1e-12), rng.next_f64());
                let g = (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * c).cos();
                *v += (sigma * g) as f32;
            }
        }
    });
}

/// The camera's blur, σ in pixels.
pub fn camera_blur(photo: &[[f32; 3]], width: usize, height: usize, sigma: f64) -> Vec<[f32; 3]> {
    blur(photo, width, height, sigma, (0, 0), None)
}

/// A round trip through JPEG at quality 85 (4:2:0), through craft-jpeg and craft-image-codecs.
pub fn jpeg_round_trip(photo: &Rgb8) -> Result<Rgb8, CoreError> {
    let (w, h) = (photo.width(), photo.height());
    let flat: Vec<u8> = photo.data().iter().flatten().copied().collect();
    let bytes = craft_jpeg::encode_baseline(&flat, w as usize, h as usize, 3, 85, craft_jpeg::ChromaSubsampling::S420, &[])
        .map_err(|e| CoreError::Scene(format!("JPEG damage: {e}")))?;
    let image = craft_image_codecs::decode(&bytes).map_err(|e| CoreError::Scene(format!("JPEG damage: {e}")))?;
    let data = image.as_u8().ok_or_else(|| CoreError::Scene("JPEG damage: unexpected sample type".into()))?;
    let (pixels, _) = data.as_chunks::<3>();
    Ok(Rgb8::from_vec(w, h, pixels.to_vec())?)
}

/// Rounds a float photo (0–255) to 8 bits.
pub fn quantize(photo: &[[f32; 3]], width: u32, height: u32) -> Result<Rgb8, CoreError> {
    let data = photo.iter().map(|p| p.map(|v| if v.is_finite() { (v + 0.5).clamp(0.0, 255.0) as u8 } else { 0 })).collect();
    Ok(Rgb8::from_vec(width, height, data)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Quad {
        Quad([Point::new(100.0, 80.0), Point::new(500.0, 90.0), Point::new(490.0, 400.0), Point::new(110.0, 390.0)])
    }

    #[test]
    fn a_finger_covers_the_anchor_and_leaves_the_far_side_alone() {
        for kind in [Damage::FingerEdge, Damage::FingerCorner] {
            let mut photo = vec![[200.0f32; 3]; 600 * 480];
            let before = photo.clone();
            apply_finger(&mut photo, 600, 480, &quad(), 0.8, kind, &mut Xorshift64Star::new(5));
            let changed: usize = photo.iter().zip(&before).filter(|(a, b)| a != b).count();
            // About a 56 × 36 px ellipse and its shadow.
            assert!((1500..30_000).contains(&changed), "{kind:?}: {changed}");
            // Skin-coloured pixels exist, and the middle of the page is untouched.
            assert!(photo.iter().any(|p| (p[0] - 226.0).abs() < 25.0 && (p[2] - 140.0).abs() < 25.0));
            assert_eq!(photo[240 * 600 + 300], [200.0; 3]);
        }
    }

    #[test]
    fn noise_has_the_requested_sigma_and_any_thread_count_gives_the_same() {
        let mut a = vec![[128.0f32; 3]; 200 * 150];
        add_noise(&mut a, 200, 5.0, 9);
        let mean = a.iter().map(|p| f64::from(p[0])).sum::<f64>() / a.len() as f64;
        let var = a.iter().map(|p| (f64::from(p[0]) - mean).powi(2)).sum::<f64>() / a.len() as f64;
        assert!((var.sqrt() - 5.0).abs() < 0.15, "{}", var.sqrt());
        let run = |n| {
            rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| {
                let mut p = vec![[10.0f32; 3]; 200 * 150];
                add_noise(&mut p, 200, 4.0, 3);
                p
            })
        };
        assert_eq!(run(1), run(8));
        // A different seed gives different noise.
        let mut b = vec![[128.0f32; 3]; 200 * 150];
        add_noise(&mut b, 200, 5.0, 10);
        assert_ne!(a, b);
    }

    #[test]
    fn a_jpeg_round_trip_loses_a_little_and_keeps_the_size() {
        let photo = Rgb8::from_vec(64, 48, (0..64 * 48).map(|i| [((i % 64) * 4) as u8, ((i / 64) * 5) as u8, 90]).collect()).unwrap();
        let back = jpeg_round_trip(&photo).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
        let mean = photo.data().iter().zip(back.data()).map(|(a, b)| (0..3).map(|c| f64::from(a[c].abs_diff(b[c]))).sum::<f64>() / 3.0).sum::<f64>()
            / photo.data().len() as f64;
        assert!(mean > 0.0 && mean < 4.0, "{mean}");
    }

    #[test]
    fn the_models_are_drawn_from_the_four_of_the_specification() {
        let mut rng = Xorshift64Star::new(1);
        let drawn: Vec<Damage> = (0..200).map(|_| Damage::draw(&mut rng)).collect();
        for d in [Damage::FingerEdge, Damage::FingerCorner, Damage::Blur, Damage::Noise, Damage::Jpeg] {
            assert!(drawn.contains(&d), "{d:?}");
        }
        assert!(Damage::FingerCorner.is_finger() && !Damage::Blur.is_finger());
    }

    #[test]
    fn quantizing_clamps_and_survives_junk() {
        let q = quantize(&[[-5.0, 300.0, 127.6], [f32::NAN, 0.4, 0.6]], 2, 1).unwrap();
        assert_eq!(q.data(), &[[0, 255, 128], [0, 0, 1]]);
        assert!(quantize(&[[0.0; 3]], 5, 5).is_err());
    }
}
