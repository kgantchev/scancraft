//! The five backgrounds of §10.1 and the light models: pure functions of position and seed, so
//! the same seed always gives the same pixels.

use scancraft_core::determinism::{Xorshift64Star, stable_hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    DarkDesk,
    LightDesk,
    Wood,
    CheckedCloth,
    WhiteOnWhite,
}

impl Background {
    pub const ALL: [Background; 5] =
        [Background::DarkDesk, Background::LightDesk, Background::Wood, Background::CheckedCloth, Background::WhiteOnWhite];

    pub fn words(self) -> &'static str {
        match self {
            Background::DarkDesk => "a dark desk",
            Background::LightDesk => "a light desk",
            Background::Wood => "wood",
            Background::CheckedCloth => "a checked tablecloth",
            Background::WhiteOnWhite => "a white surface",
        }
    }
}

/// A value in [0, 1) from a lattice point.
fn lattice(ix: i64, iy: i64, seed: u64) -> f64 {
    let mut bytes = [0u8; 24];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    bytes[8..16].copy_from_slice(&ix.to_le_bytes());
    bytes[16..].copy_from_slice(&iy.to_le_bytes());
    (stable_hash(&bytes) >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in [0, 1) at (x, y), with features about one unit across.
pub fn noise(x: f64, y: f64, seed: u64) -> f64 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (ix as i64, iy as i64);
    let a = lattice(ix, iy, seed) * (1.0 - sx) + lattice(ix + 1, iy, seed) * sx;
    let b = lattice(ix, iy + 1, seed) * (1.0 - sx) + lattice(ix + 1, iy + 1, seed) * sx;
    a * (1.0 - sy) + b * sy
}

/// The colour (0–255 per channel) of the background at photo pixel (x, y) of a `w × h` photo.
pub fn background(kind: Background, x: f64, y: f64, w: f64, h: f64, seed: u64) -> [f32; 3] {
    let diagonal = (x / w + y / h) / 2.0;
    let rgb = match kind {
        Background::DarkDesk => {
            let v = 0.17 + 0.06 * diagonal + 0.012 * (noise(x / 40.0, y / 40.0, seed) - 0.5);
            [v * 1.04, v, v * 0.94]
        }
        Background::LightDesk => {
            let v = 0.74 + 0.07 * diagonal + 0.01 * (noise(x / 50.0, y / 50.0, seed) - 0.5);
            [v * 1.02, v, v * 0.93]
        }
        Background::Wood => {
            // Rings across a slanted axis, bent by noise, with grain and a few darker knots.
            let angle = 0.3 + 2.0 * lattice(0, 0, seed);
            let along = x * angle.cos() + y * angle.sin();
            let ring = (along / 22.0 + 4.0 * noise(x / 160.0, y / 160.0, seed ^ 1)).sin() * 0.5 + 0.5;
            let grain = noise(x / 3.0, y / 90.0, seed ^ 2);
            let knot = (-((noise(x / 300.0, y / 300.0, seed ^ 3) - 0.5).powi(2)) / 0.004).exp() * 0.25;
            let v = 0.32 + 0.16 * ring + 0.05 * grain - knot;
            [v * 1.25, v * 0.88, v * 0.55]
        }
        Background::CheckedCloth => {
            let (cx, cy) = ((x / 24.0).floor() as i64, (y / 24.0).floor() as i64);
            let check = (cx + cy).rem_euclid(2) == 0;
            let weave = 0.025 * ((x * 1.4).sin() * (y * 1.4).sin());
            let base = if check { [0.78, 0.18, 0.18] } else { [0.88, 0.86, 0.80] };
            base.map(|v| v + weave + 0.02 * (noise(x / 2.0, y / 2.0, seed) - 0.5))
        }
        Background::WhiteOnWhite => {
            let v = 0.95 - 0.015 * diagonal + 0.006 * (noise(x / 60.0, y / 60.0, seed) - 0.5);
            [v, v, v]
        }
    };
    rgb.map(|v| (v * 255.0).clamp(0.0, 255.0) as f32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    Gradient,
    HardShadow,
    Dappled,
    WarmCast,
}

impl Light {
    pub const ALL: [Light; 4] = [Light::Gradient, Light::HardShadow, Light::Dappled, Light::WarmCast];

    pub fn words(self) -> &'static str {
        match self {
            Light::Gradient => "a 2.5 : 1 light gradient",
            Light::HardShadow => "a hard shadow",
            Light::Dappled => "dappled shade",
            Light::WarmCast => "a warm colour cast",
        }
    }
}

/// A light model with its random choices made: the gain it gives each channel at a pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct LightModel {
    kind: Light,
    width: f64,
    height: f64,
    /// Gradient: direction (unit); shadow: a half-plane's unit normal and offset.
    direction: (f64, f64),
    offset: f64,
    /// Dappled: three (direction, frequency, phase).
    waves: [((f64, f64), f64, f64); 3],
}

impl LightModel {
    pub fn new(kind: Light, width: f64, height: f64, rng: &mut Xorshift64Star) -> LightModel {
        let angle = rng.next_f64() * std::f64::consts::TAU;
        let direction = (angle.cos(), angle.sin());
        // The half-plane's boundary passes within the middle half of the photo.
        let offset = (rng.next_f64() - 0.5) * 0.5 * width.hypot(height);
        let mut wave = || {
            let a = rng.next_f64() * std::f64::consts::TAU;
            ((a.cos(), a.sin()), (0.004 + 0.012 * rng.next_f64()) * std::f64::consts::TAU, rng.next_f64() * std::f64::consts::TAU)
        };
        let waves = [wave(), wave(), wave()];
        LightModel { kind, width, height, direction, offset, waves }
    }

    /// The multiplier for (red, green, blue) at pixel (x, y).
    pub fn gain(&self, x: f64, y: f64) -> [f32; 3] {
        let (cx, cy) = (x - self.width / 2.0, y - self.height / 2.0);
        match self.kind {
            Light::Gradient => {
                // The ramp spans the photo's extent along its direction, corner to corner.
                let half = (self.width * self.direction.0.abs() + self.height * self.direction.1.abs()) / 2.0;
                let t = ((cx * self.direction.0 + cy * self.direction.1) / (2.0 * half) + 0.5).clamp(0.0, 1.0);
                // 0.62 at the dark end and 1.55 at the bright end: a ratio of 2.5, a middle near 1.
                let (lo, hi) = (0.62f32, 1.55f32);
                let v = lo + (hi - lo) * t as f32;
                [v; 3]
            }
            Light::HardShadow => {
                // ×0.6 on one side of a line, with a linear 3 px penumbra across it.
                let d = cx * self.direction.0 + cy * self.direction.1 - self.offset;
                let t = (d / 3.0 + 0.5).clamp(0.0, 1.0) as f32;
                [1.0 - 0.4 * (1.0 - t); 3]
            }
            Light::Dappled => {
                let sum: f64 = self.waves.iter().map(|((dx, dy), f, phase)| ((cx * dx + cy * dy) * f + phase).sin()).sum();
                let v = (0.62 + 0.2 * sum).clamp(0.25, 1.0) as f32;
                [v; 3]
            }
            Light::WarmCast => [1.08, 1.0, 0.92],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backgrounds_are_pure_functions_of_position_and_seed() {
        for kind in Background::ALL {
            let a = background(kind, 100.5, 40.5, 800.0, 600.0, 7);
            assert_eq!(a, background(kind, 100.5, 40.5, 800.0, 600.0, 7));
            assert!(a.iter().all(|v| (0.0..=255.0).contains(v)));
        }
        // A different seed changes the wood and the cloth's texture.
        assert_ne!(background(Background::Wood, 300.0, 200.0, 800.0, 600.0, 1), background(Background::Wood, 300.0, 200.0, 800.0, 600.0, 2));
    }

    #[test]
    fn the_backgrounds_have_their_character() {
        let mean = |k: Background| {
            (0..400)
                .map(|i| background(k, f64::from(i % 20) * 37.0, f64::from(i / 20) * 29.0, 800.0, 600.0, 3).iter().sum::<f32>() / 3.0)
                .sum::<f32>()
                / 400.0
        };
        assert!(mean(Background::DarkDesk) < 70.0 && mean(Background::LightDesk) > 170.0);
        assert!(mean(Background::WhiteOnWhite) > 235.0);
        // The wood is warm: red above blue. The cloth checks: two colours.
        let w = background(Background::Wood, 120.0, 90.0, 800.0, 600.0, 5);
        assert!(w[0] > w[2] * 1.3);
        let (a, b) =
            (background(Background::CheckedCloth, 5.0, 5.0, 800.0, 600.0, 1), background(Background::CheckedCloth, 29.0, 5.0, 800.0, 600.0, 1));
        assert!((a[1] - b[1]).abs() > 60.0);
    }

    #[test]
    fn the_gradient_runs_two_and_a_half_to_one() {
        let m = LightModel::new(Light::Gradient, 1000.0, 800.0, &mut Xorshift64Star::new(4));
        let gains: Vec<f32> =
            (0..1000).flat_map(|x| (0..800).step_by(50).map(move |y| (x, y))).map(|(x, y)| m.gain(f64::from(x), f64::from(y))[0]).collect();
        let (lo, hi) = gains.iter().fold((f32::INFINITY, 0f32), |(l, h), &g| (l.min(g), h.max(g)));
        assert!(((hi / lo) - 2.5).abs() < 0.05, "{}", hi / lo);
    }

    #[test]
    fn the_hard_shadow_is_point_six_with_a_three_pixel_penumbra() {
        let m = LightModel::new(Light::HardShadow, 1000.0, 800.0, &mut Xorshift64Star::new(9));
        // Walk along the line's normal through the photo's middle and find the transition.
        let (nx, ny) = m.direction;
        let at = |d: f64| m.gain(500.0 + (m.offset + d) * nx, 400.0 + (m.offset + d) * ny)[0];
        assert!((at(-10.0) - 0.6).abs() < 1e-6 && (at(10.0) - 1.0).abs() < 1e-6);
        // Over 3 px the gain climbs from 0.6 to 1.
        let width = (0..600).map(|i| -3.0 + f64::from(i) * 0.01).filter(|&d| at(d) > 0.6 + 1e-4 && at(d) < 1.0 - 1e-4).count() as f64 * 0.01;
        assert!((width - 3.0).abs() < 0.05, "{width}");
    }

    #[test]
    fn dappled_shade_stays_in_range_and_the_cast_is_warm() {
        let m = LightModel::new(Light::Dappled, 640.0, 480.0, &mut Xorshift64Star::new(2));
        let values: Vec<f32> = (0..2000).map(|i| m.gain(f64::from(i % 64) * 10.0, f64::from(i / 64) * 15.0)[0]).collect();
        assert!(values.iter().all(|v| (0.25..=1.0).contains(v)));
        assert!(values.iter().cloned().fold(0f32, f32::max) - values.iter().cloned().fold(1f32, f32::min) > 0.2);
        let w = LightModel::new(Light::WarmCast, 10.0, 10.0, &mut Xorshift64Star::new(1)).gain(3.0, 3.0);
        assert_eq!(w, [1.08, 1.0, 0.92]);
    }
}
