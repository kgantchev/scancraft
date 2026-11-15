//! The sheet: a page of paper, flat or bent without stretching (specification §10.1), written for
//! the generator alone so that it shares no code with A27's model.
//!
//! A bent sheet is a profile curve extruded along straight rulings. The profile is given by its
//! heading φ(s) against arclength s, integrated by the trapezoid rule, so its length is the paper's
//! width exactly and the chord is shorter. Units are points (1/72 in), so "6 pt text" is a size.

use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::recipe::BendMode;
use std::f64::consts::PI;

/// A4 and A5 in points.
pub const A4: (f64, f64) = (595.28, 841.89);
pub const A5: (f64, f64) = (419.53, 595.28);

/// The samples of the profile integral, as in the fixtures.
const SAMPLES: usize = 4001;
/// The values `profile_turning` records.
pub const TURNING_VALUES: usize = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BendKind {
    Flat,
    BookPage,
    Curl,
    Sag,
    FoldedInThree,
    Spread,
}

impl BendKind {
    pub const BENT: [BendKind; 5] = [BendKind::BookPage, BendKind::Curl, BendKind::Sag, BendKind::FoldedInThree, BendKind::Spread];
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub kind: BendKind,
    pub mode: BendMode,
    pub width_pt: f64,
    pub height_pt: f64,
    /// Words for the truth file's description.
    pub description: String,
    /// Arclength of the profile and the profile's heading and position at each sample.
    length: f64,
    x: Vec<f64>,
    z: Vec<f64>,
    phi: Vec<f64>,
}

/// A smooth step from 0 to 1 over `[0, 1]`.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Sheet {
    /// A sheet whose profile runs along the page's width (`Across`) or height (`Down`) with the
    /// given heading at each arclength.
    pub fn new(kind: BendKind, mode: BendMode, size_pt: (f64, f64), description: String, heading: impl Fn(f64) -> f64) -> Sheet {
        let length = if mode == BendMode::Across { size_pt.0 } else { size_pt.1 };
        let ds = length / (SAMPLES - 1) as f64;
        let (mut x, mut z, mut phi) = (vec![0.0], vec![0.0], vec![heading(0.0)]);
        for i in 1..SAMPLES {
            let (a, b) = (heading((i - 1) as f64 * ds), heading(i as f64 * ds));
            x.push(x[i - 1] + ds * (a.cos() + b.cos()) / 2.0);
            z.push(z[i - 1] + ds * (a.sin() + b.sin()) / 2.0);
            phi.push(b);
        }
        Sheet { kind, mode, width_pt: size_pt.0, height_pt: size_pt.1, description, length, x, z, phi }
    }

    pub fn flat(size_pt: (f64, f64)) -> Sheet {
        Sheet::new(BendKind::Flat, BendMode::Across, size_pt, "flat page".to_string(), |_| 0.0)
    }

    /// A bent sheet of `kind`, with its angles and direction drawn from `rng`. `Flat` gives a flat one.
    pub fn random(kind: BendKind, rng: &mut Xorshift64Star) -> Sheet {
        let mut between = |lo: f64, hi: f64| lo + (hi - lo) * rng.next_f64();
        let sign = if between(0.0, 1.0) < 0.5 { -1.0 } else { 1.0 };
        let mode_draw = between(0.0, 1.0);
        match kind {
            BendKind::Flat => Sheet::flat(A4),
            BendKind::BookPage => {
                let (theta, l) = (between(25.0, 65.0).to_radians(), A4.0);
                let description =
                    format!("book page rising {:.0}° from its spine on the {}", theta.to_degrees(), if sign < 0.0 { "right" } else { "left" });
                // The page rises out of its spine and levels off; the right-hand spine is the mirror image.
                Sheet::new(kind, BendMode::Across, A4, description, move |s| {
                    if sign > 0.0 { theta * (-s / (0.1 * l)).exp() } else { -theta * (-(l - s) / (0.1 * l)).exp() }
                })
            }
            BendKind::Curl => {
                let theta = between(20.0, 80.0).to_radians();
                let mode = if mode_draw < 0.5 { BendMode::Across } else { BendMode::Down };
                let l = if mode == BendMode::Across { A4.0 } else { A4.1 };
                Sheet::new(kind, mode, A4, format!("sheet curled through {:.0}°", theta.to_degrees()), move |s| sign * theta * s / l)
            }
            BendKind::Sag => {
                let theta = between(8.0, 20.0).to_radians();
                let mode = if mode_draw < 0.5 { BendMode::Across } else { BendMode::Down };
                let l = if mode == BendMode::Across { A4.0 } else { A4.1 };
                Sheet::new(kind, mode, A4, format!("sheet sagging by {:.0}° each way", theta.to_degrees()), move |s| {
                    sign * theta * (2.0 * s / l - 1.0)
                })
            }
            BendKind::FoldedInThree => {
                let alpha = between(10.0, 25.0).to_radians();
                let l = A4.1;
                let w = 0.02 * l;
                // 0, then +α after the first fold, then −α after the second, with rounded creases.
                Sheet::new(kind, BendMode::Down, A4, format!("letter folded in three, folds of {:.0}°", alpha.to_degrees()), move |s| {
                    sign * alpha * (smoothstep((s - l / 3.0) / w + 0.5) - 2.0 * smoothstep((s - 2.0 * l / 3.0) / w + 0.5))
                })
            }
            BendKind::Spread => {
                let theta = between(30.0, 55.0).to_radians();
                let page = A5.0;
                let rise = move |d: f64| theta * (-d / (0.08 * page)).exp();
                let size = (2.0 * A5.0, A5.1);
                Sheet::new(
                    kind,
                    BendMode::Across,
                    size,
                    format!("open book with a gutter, pages dipping {:.0}° into the spine", theta.to_degrees()),
                    move |s| {
                        if s < page { -rise(page - s) } else { rise(s - page) }
                    },
                )
            }
        }
    }

    /// The arclength of the profile, which is the paper's width (`Across`) or height (`Down`).
    pub fn profile_length(&self) -> f64 {
        self.length
    }

    /// The straight distance between the profile's ends.
    pub fn chord(&self) -> f64 {
        let (x, z) = (self.x.last().copied().unwrap_or(0.0), self.z.last().copied().unwrap_or(0.0));
        x.hypot(z)
    }

    /// The profile's position at arclength `s`: (x, z), z growing toward the camera.
    fn profile_at(&self, s: f64) -> (f64, f64) {
        let last = (SAMPLES - 1) as f64;
        let k = ((s / self.length) * last).clamp(0.0, last);
        let (i, f) = (k.floor() as usize, k.fract());
        let j = (i + 1).min(SAMPLES - 1);
        (self.x[i] + f * (self.x[j] - self.x[i]), self.z[i] + f * (self.z[j] - self.z[i]))
    }

    /// The sheet's point at page fractions (u across, v down), in points; the camera's z points
    /// away, so a page rising toward the camera has negative z.
    pub fn point(&self, u: f64, v: f64) -> [f64; 3] {
        match self.mode {
            BendMode::Across => {
                let (x, z) = self.profile_at(u * self.length);
                [x, v * self.height_pt, -z]
            }
            BendMode::Down => {
                let (y, z) = self.profile_at(v * self.length);
                [u * self.width_pt, y, -z]
            }
        }
    }

    /// The profile's heading at the 33 evenly spaced arclengths `profile_turning` records. (The
    /// field is named for the turning angle, but the fixtures store the heading; so does this.)
    pub fn turning(&self) -> Vec<f64> {
        (0..TURNING_VALUES).map(|k| self.phi[k * (SAMPLES - 1) / (TURNING_VALUES - 1)]).collect()
    }

    /// The heading at arclength `s`.
    pub fn heading_at(&self, s: f64) -> f64 {
        let k = ((s / self.length) * (SAMPLES - 1) as f64).round().clamp(0.0, (SAMPLES - 1) as f64) as usize;
        self.phi[k]
    }

    pub fn is_spread(&self) -> bool {
        self.kind == BendKind::Spread
    }
}

/// Degrees in a full turn, kept here for the docs' sake.
pub const FULL_TURN: f64 = 2.0 * PI;

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> Xorshift64Star {
        Xorshift64Star::new(seed)
    }

    #[test]
    fn a_flat_sheet_is_the_plane() {
        let s = Sheet::flat(A4);
        assert_eq!(s.point(0.0, 0.0), [0.0, 0.0, 0.0]);
        let p = s.point(1.0, 1.0);
        assert!((p[0] - A4.0).abs() < 1e-9 && (p[1] - A4.1).abs() < 1e-9 && p[2].abs() < 1e-12);
        assert!((s.chord() - s.profile_length()).abs() < 1e-9);
    }

    #[test]
    fn a_bent_page_is_bent_without_stretching() {
        for (n, kind) in BendKind::BENT.iter().enumerate() {
            for seed in 0..10 {
                let s = Sheet::random(*kind, &mut rng(seed * 7 + n as u64));
                let want = if s.mode == BendMode::Across { s.width_pt } else { s.height_pt };
                assert!((s.profile_length() - want).abs() < 1e-9);
                // The profile's length by the point table equals the arclength, and the chord is shorter.
                let steps = 4000;
                let mut length = 0.0;
                let mut last = if s.mode == BendMode::Across { s.point(0.0, 0.0) } else { s.point(0.0, 0.0) };
                for i in 1..=steps {
                    let t = f64::from(i) / f64::from(steps);
                    let p = if s.mode == BendMode::Across { s.point(t, 0.0) } else { s.point(0.0, t) };
                    length += ((p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2) + (p[2] - last[2]).powi(2)).sqrt();
                    last = p;
                }
                assert!((length - want).abs() < 1e-3 * want, "{kind:?}: {length} vs {want}");
                assert!(s.chord() < s.profile_length() || *kind == BendKind::FoldedInThree, "{kind:?}");
            }
        }
    }

    #[test]
    fn the_five_shapes_have_their_modes_and_sizes() {
        let spread = Sheet::random(BendKind::Spread, &mut rng(1));
        assert_eq!((spread.mode, spread.width_pt), (BendMode::Across, 2.0 * A5.0));
        assert!(spread.is_spread());
        let fold = Sheet::random(BendKind::FoldedInThree, &mut rng(2));
        assert_eq!(fold.mode, BendMode::Down);
        let book = Sheet::random(BendKind::BookPage, &mut rng(3));
        assert_eq!(book.mode, BendMode::Across);
        // A book page's first heading is its full rise, 25–65°.
        let rise = book.heading_at(0.0).abs().max(book.heading_at(book.profile_length()).abs()).to_degrees();
        assert!((25.0..=65.0).contains(&rise), "{rise}");
    }

    #[test]
    fn the_recorded_turning_matches_the_sheet() {
        let s = Sheet::random(BendKind::Curl, &mut rng(5));
        let t = s.turning();
        assert_eq!(t.len(), TURNING_VALUES);
        for (k, v) in t.iter().enumerate() {
            let at = s.profile_length() * k as f64 / (TURNING_VALUES - 1) as f64;
            assert!((v - s.heading_at(at)).abs() < 1e-3, "{k}");
        }
        // A curl turns steadily: the heading is monotonic.
        assert!(t.windows(2).all(|w| (w[1] - w[0]) * (t[32] - t[0]) >= -1e-12));
    }

    #[test]
    fn the_same_seed_gives_the_same_sheet() {
        assert_eq!(Sheet::random(BendKind::Sag, &mut rng(9)), Sheet::random(BendKind::Sag, &mut rng(9)));
        assert_ne!(Sheet::random(BendKind::Sag, &mut rng(9)), Sheet::random(BendKind::Sag, &mut rng(10)));
    }
}
