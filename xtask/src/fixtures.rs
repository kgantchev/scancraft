//! `cargo xtask fixtures`: draws the three test photos of build plan WP0 step 7, with their truth
//! files in the scene format (`scancraft_core::scene`).
//!
//! They are drawn, not photographed, so their corners, proportions and page positions are exact:
//! a flat page, a book page curling into its spine, and an open book's two-page spread, each on a
//! dark desk under a tilted camera. Real photos belong in the reference set (specification §10.2),
//! outside the repository.

use scancraft_core::determinism::stable_hash;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::recipe::BendMode;
use scancraft_core::scene::{SCENE_FORMAT, SceneShape, SceneTruth};
use scancraft_geom::homography::Homography;
use std::path::Path;

const W: u32 = 960;
const H: u32 = 720;
const F: f64 = 0.8;
const GRID: usize = 17;

/// Rotation about x, then y, then z (roll), as a row-major 3 × 3.
fn rotation(ax: f64, ay: f64, az: f64) -> [[f64; 3]; 3] {
    let (cx, sx, cy, sy, cz, sz) = (ax.cos(), ax.sin(), ay.cos(), ay.sin(), az.cos(), az.sin());
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul(&rz, &mul(&rx, &ry))
}

fn mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

struct Pose {
    r: [[f64; 3]; 3],
    t: [f64; 3],
}

impl Pose {
    /// The camera sees the page frame point `centre` at distance `d` straight ahead.
    fn new(ax: f64, ay: f64, roll: f64, centre: [f64; 3], d: f64) -> Pose {
        let r = rotation(ax.to_radians(), ay.to_radians(), roll.to_radians());
        let rc: Vec<f64> = (0..3).map(|i| (0..3).map(|k| r[i][k] * centre[k]).sum()).collect();
        Pose { r, t: [-rc[0], -rc[1], d - rc[2]] }
    }

    fn project(&self, p: [f64; 3]) -> Point {
        let q: Vec<f64> = (0..3).map(|i| (0..3).map(|k| self.r[i][k] * p[k]).sum::<f64>() + self.t[i]).collect();
        let fp = F * f64::from(W.max(H));
        Point::new(f64::from(W) / 2.0 + fp * q[0] / q[2], f64::from(H) / 2.0 + fp * q[1] / q[2])
    }
}

/// Page content at developed page coordinates (s, v) ∈ [0, 1]²: white paper with rows of dark
/// word bars inside margins. `columns` splits the page into side-by-side pages (a spread).
fn paper(s: f64, v: f64, columns: u32) -> f64 {
    let col = ((s * f64::from(columns)).floor() as u32).min(columns - 1);
    let local = s * f64::from(columns) - f64::from(col);
    if !(0.1..=0.9).contains(&local) || !(0.08..=0.92).contains(&v) {
        return 0.95;
    }
    let line = ((v - 0.08) / 0.035).floor();
    let within = (v - 0.08) / 0.035 - line;
    if within > 0.38 {
        return 0.95;
    }
    let word = ((local - 0.1) / 0.06).floor();
    let h = stable_hash(format!("{col}/{line}/{word}").as_bytes());
    let gap = (local - 0.1) / 0.06 - word;
    if gap > 0.25 + (h % 50) as f64 / 100.0 { 0.95 } else { 0.15 }
}

/// A dark desk with a gentle diagonal gradient (smooth, so the PNGs stay small).
fn desk(x: u32, y: u32) -> f64 {
    0.17 + 0.06 * (f64::from(x) / f64::from(W) + f64::from(y) / f64::from(H)) / 2.0
}

fn to_png(values: &[f64], path: &Path) -> Result<(), String> {
    let bytes: Vec<u8> = values.iter().flat_map(|&v| [(v.clamp(0.0, 1.0) * 255.0).round() as u8; 3]).collect();
    image::save_buffer(path, &bytes, W, H, image::ExtendedColorType::Rgb8).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(dir: &Path, name: &str, values: &[f64], truth: &SceneTruth) -> Result<(), String> {
    to_png(values, &dir.join(format!("{name}.png")))?;
    let json = truth.to_json().map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.json")), json + "\n").map_err(|e| format!("{name}.json: {e}"))
}

/// The flat page: rendered pixel by pixel through the inverse homography, 2 × 2 samples each.
fn flat(dir: &Path) -> Result<(), String> {
    let ratio = 210.0 / 297.0;
    let pose = Pose::new(14.0, -9.0, 4.0, [ratio / 2.0, 0.5, 0.0], 1.45);
    let corners = Quad([[0.0, 0.0], [ratio, 0.0], [ratio, 1.0], [0.0, 1.0]].map(|[x, y]| pose.project([x, y, 0.0])));
    let hmg = Homography::from_unit_square(&corners, f64::from(W)).map_err(|e| e.to_string())?;
    let mut px = vec![0.0; (W * H) as usize];
    for y in 0..H {
        for x in 0..W {
            let mut acc = 0.0;
            for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                acc += match hmg.invert(f64::from(x) + dx, f64::from(y) + dy) {
                    Some((u, v)) if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) => paper(u, v, 1),
                    _ => desk(x, y),
                };
            }
            px[(y * W + x) as usize] = acc / 4.0;
        }
    }
    let truth = SceneTruth {
        format: SCENE_FORMAT,
        description: "flat A4-shaped page on a dark desk, camera tilted 14° and 9°, rolled 4°".into(),
        size: (W, H),
        focal: F,
        corners,
        ratio,
        shape: SceneShape::Flat,
        spine: None,
        page_grid: None,
    };
    write(dir, "flat", &px, &truth)
}

/// A bent sheet: the cross-section's turning angle φ at each developed position s ∈ [0, S], with
/// rulings of length 1 running down the page.
struct Bent {
    s: Vec<f64>,
    x: Vec<f64>,
    z: Vec<f64>,
    phi: Vec<f64>,
}

impl Bent {
    fn new(len: f64, phi: impl Fn(f64) -> f64) -> Bent {
        let n = 4001;
        let ds = len / (n - 1) as f64;
        let (mut s, mut x, mut z, mut ph) = (vec![0.0], vec![0.0], vec![0.0], vec![phi(0.0)]);
        for i in 1..n {
            let (a, b) = (phi((i - 1) as f64 * ds), phi(i as f64 * ds));
            s.push(i as f64 * ds);
            x.push(x[i - 1] + ds * (a.cos() + b.cos()) / 2.0);
            z.push(z[i - 1] + ds * (a.sin() + b.sin()) / 2.0);
            ph.push(b);
        }
        Bent { s, x, z, phi: ph }
    }

    /// Page-frame point at developed position `sd` and ruling position `b`; z toward the camera.
    fn point(&self, sd: f64, b: f64) -> [f64; 3] {
        let k = ((sd / self.s[self.s.len() - 1]) * (self.s.len() - 1) as f64).clamp(0.0, (self.s.len() - 1) as f64);
        let (i, f) = (k.floor() as usize, k.fract());
        let j = (i + 1).min(self.s.len() - 1);
        [self.x[i] + f * (self.x[j] - self.x[i]), b, -(self.z[i] + f * (self.z[j] - self.z[i]))]
    }
}

/// Forward rendering of a bent page: page samples three times denser than the photo's pixels are
/// projected and averaged per pixel; untouched pixels show the desk. `shade` darkens a gutter.
fn splat(pose: &Pose, sheet: &Bent, columns: u32, shade: impl Fn(f64) -> f64) -> Vec<f64> {
    let len = sheet.s[sheet.s.len() - 1];
    let (ns, nv) = (3 * W as usize, 3 * H as usize);
    let mut sum = vec![0.0; (W * H) as usize];
    let mut count = vec![0u32; (W * H) as usize];
    for j in 0..nv {
        let v = (j as f64 + 0.5) / nv as f64;
        for i in 0..ns {
            let u = (i as f64 + 0.5) / ns as f64;
            let p = pose.project(sheet.point(u * len, v));
            if p.x < 0.0 || p.y < 0.0 || p.x >= f64::from(W) || p.y >= f64::from(H) {
                continue;
            }
            let k = (p.y as u32 * W + p.x as u32) as usize;
            sum[k] += paper(u, v, columns) * shade(u);
            count[k] += 1;
        }
    }
    (0..(W * H) as usize).map(|k| if count[k] > 0 { sum[k] / f64::from(count[k]) } else { desk(k as u32 % W, k as u32 / W) }).collect()
}

fn grid(pose: &Pose, sheet: &Bent) -> Vec<Vec<Point>> {
    let len = sheet.s[sheet.s.len() - 1];
    (0..GRID)
        .map(|j| (0..GRID).map(|i| pose.project(sheet.point(i as f64 / (GRID - 1) as f64 * len, j as f64 / (GRID - 1) as f64))).collect())
        .collect()
}

fn turning(sheet: &Bent) -> Vec<f64> {
    (0..33).map(|k| sheet.phi[k * (sheet.phi.len() - 1) / 32]).collect()
}

fn bent_truth(pose: &Pose, sheet: &Bent, description: &str, ratio: f64, spine: Option<[Point; 2]>) -> SceneTruth {
    let len = sheet.s[sheet.s.len() - 1];
    let corners = Quad([(0.0, 0.0), (len, 0.0), (len, 1.0), (0.0, 1.0)].map(|(s, b)| pose.project(sheet.point(s, b))));
    SceneTruth {
        format: SCENE_FORMAT,
        description: description.into(),
        size: (W, H),
        focal: F,
        corners,
        ratio,
        shape: SceneShape::Bent { mode: BendMode::Across, profile_turning: turning(sheet) },
        spine,
        page_grid: Some(grid(pose, sheet)),
    }
}

/// One book page, its spine on the left: it rises 50° out of the spine and levels off.
fn curved(dir: &Path) -> Result<(), String> {
    let len = 0.705;
    let sheet = Bent::new(len, |s| 50f64.to_radians() * (-s / (0.1 * len)).exp());
    let pose = Pose::new(10.0, 6.0, -3.0, [len / 2.0, 0.5, 0.0], 1.45);
    let px = splat(&pose, &sheet, 1, |_| 1.0);
    let truth = bent_truth(&pose, &sheet, "book page curling 50° into its spine on the left, dark desk, camera tilted 10° and 6°", len, None);
    write(dir, "curved", &px, &truth)
}

/// An open book: two pages of 0.705 × 1 dipping into a shaded gutter at the middle.
fn spread(dir: &Path) -> Result<(), String> {
    let page = 0.705;
    let rise = |d: f64| 45f64.to_radians() * (-d / (0.08 * page)).exp();
    // Left page runs from its outer edge down into the spine, the right page up out of it.
    let sheet = Bent::new(2.0 * page, |s| if s < page { -rise(page - s) } else { rise(s - page) });
    let pose = Pose::new(12.0, 0.0, 2.0, [sheet.point(page, 0.5)[0], 0.5, 0.0], 1.9);
    let px = splat(&pose, &sheet, 2, |u| 1.0 - 0.35 * (-(u - 0.5).abs() / 0.03).exp());
    let mid = sheet.point(page, 0.0);
    let spine = [pose.project(mid), pose.project([mid[0], 1.0, mid[2]])];
    let truth = bent_truth(&pose, &sheet, "open book, two pages dipping 45° into a shaded gutter, camera tilted 12°", 2.0 * page, Some(spine));
    write(dir, "spread", &px, &truth)
}

pub fn run() -> Result<(), String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fixtures");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    flat(&dir)?;
    curved(&dir)?;
    spread(&dir)?;
    println!("fixtures: wrote flat, curved and spread to {}", dir.display());
    Ok(())
}
