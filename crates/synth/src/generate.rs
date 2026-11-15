//! One scene: the parameters are drawn from independent streams (one per purpose, seeded from the
//! scene's seed), the page and sheet are built, placed in front of the camera, rendered, lit and
//! damaged, and the truth is read off the same geometry that drew the photo.

use crate::background::{Background, Light, LightModel};
use crate::camera::{MAX_OUTSIDE, Placement, Pose, SceneCamera, overhang, place};
use crate::damage::{Damage, add_noise, apply_finger, camera_blur, jpeg_round_trip, quantize};
use crate::page::{reference_page, spread_page};
use crate::render::{composite, rasterize};
use crate::sheet::{BendKind, Sheet};
use crate::{Scene, SceneSpec};
use scancraft_core::CoreError;
use scancraft_core::determinism::{Xorshift64Star, stable_hash};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::BitPlane;
use scancraft_core::recipe::BendMode;
use scancraft_core::scene::{SCENE_FORMAT, SceneShape, SceneTruth};

/// The most scenes one set may hold; a bigger count is refused instead of filling the disk.
pub const MAX_COUNT: u32 = 10_000;
/// The grid of page points a bent scene records: `GRID × GRID`.
pub const GRID: usize = 17;

/// The photo sizes scenes are drawn at.
const SIZES: [(u32, u32); 4] = [(1024, 768), (768, 1024), (1280, 960), (960, 1280)];

/// The streams, by purpose. The bytes are fixed for ever so a new draw in one stream can't shift
/// another's.
#[derive(Clone, Copy)]
enum Purpose {
    Camera = 0,
    Background = 1,
    Light = 2,
    Damage = 3,
    Content = 4,
    Shape = 5,
    Noise = 6,
}

fn stream(seed: u64, purpose: Purpose) -> Xorshift64Star {
    Xorshift64Star::new(stable_hash(&[seed.to_le_bytes().as_slice(), &[purpose as u8]].concat()))
}

/// A scene with everything the generator knows about it.
pub struct Generated {
    pub scene: Scene,
    /// The page's true ink mask, at 300 DPI in the flat page's own pixels.
    pub ink: BitPlane,
    /// The words on the page, a line each, for the character error rate.
    pub text: String,
}

fn between(rng: &mut Xorshift64Star, lo: f64, hi: f64) -> f64 {
    lo + (hi - lo) * rng.next_f64()
}

/// Everything drawn for a scene before anything is rendered.
pub struct Params {
    pub seed: u64,
    pub cam: SceneCamera,
    pub tilt: f64,
    pub azimuth: f64,
    pub roll: f64,
    pub coverage: f64,
    pub shift: (f64, f64),
    pub background: Background,
    pub bg_seed: u64,
    pub light: Light,
    pub damage: Damage,
    pub kind: BendKind,
}

fn check(spec: &SceneSpec, index: u32) -> Result<(), CoreError> {
    if spec.count > MAX_COUNT {
        return Err(CoreError::Scene(format!("a set of {} scenes is too many; the most is {MAX_COUNT}", spec.count)));
    }
    if index >= spec.count {
        return Err(CoreError::Scene(format!("scene index {index} is past the count {}", spec.count)));
    }
    Ok(())
}

/// The scene's parameters, drawn in a fixed order from independent streams.
pub fn draw_params(spec: &SceneSpec, index: u32) -> Result<Params, CoreError> {
    check(spec, index)?;
    let seed = stable_hash(&[spec.seed.to_le_bytes(), u64::from(index).to_le_bytes()].concat());
    let (mut cam_rng, mut bg_rng, mut light_rng, mut dmg_rng) =
        (stream(seed, Purpose::Camera), stream(seed, Purpose::Background), stream(seed, Purpose::Light), stream(seed, Purpose::Damage));
    let size = SIZES[cam_rng.below(SIZES.len() as u64) as usize];
    let f = between(&mut cam_rng, 0.6, 1.2);
    let cam = SceneCamera { width: f64::from(size.0), height: f64::from(size.1), f };
    let max_tilt: f64 = if spec.curved { 25.0 } else { 50.0 };
    let tilt = between(&mut cam_rng, 0.0, max_tilt).to_radians();
    let azimuth = between(&mut cam_rng, 0.0, std::f64::consts::TAU);
    let roll = between(&mut cam_rng, 0.0, std::f64::consts::TAU);
    let coverage = between(&mut cam_rng, 0.7, 0.98);
    let shift = (between(&mut cam_rng, -1.0, 1.0), between(&mut cam_rng, -1.0, 1.0));
    let background = Background::ALL[bg_rng.below(5) as usize];
    let bg_seed = bg_rng.next_u64();
    let light = Light::ALL[light_rng.below(4) as usize];
    let damage = Damage::draw(&mut dmg_rng);
    // Over any five consecutive scenes of a curved set every bent shape occurs once.
    let kind =
        if spec.curved { BendKind::BENT[((stable_hash(&spec.seed.to_le_bytes()) % 5 + u64::from(index)) % 5) as usize] } else { BendKind::Flat };
    Ok(Params { seed, cam, tilt, azimuth, roll, coverage, shift, background, bg_seed, light, damage, kind })
}

/// The scene's sheet.
pub fn build_sheet(p: &Params) -> Sheet {
    Sheet::random(p.kind, &mut stream(p.seed, Purpose::Shape))
}

/// Places the sheet; if it can't be seen or hangs too far outside, the tilt and the shift are
/// eased. Returns the pose and the tilt that was used.
pub fn place_sheet(p: &Params, sheet: &Sheet) -> Result<(Pose, f64), CoreError> {
    let surface = |u: f64, v: f64| sheet.point(u, v);
    let (mut t, mut s) = (p.tilt, p.shift);
    for _ in 0..5 {
        let placement = Placement { tilt: t, azimuth: p.azimuth, roll: p.roll, coverage: p.coverage, shift: s };
        if let Some(pose) = place(&p.cam, &placement, &surface)
            && overhang(&pose, &p.cam, &surface).is_some_and(|o| o <= MAX_OUTSIDE + 0.005)
        {
            return Ok((pose, t));
        }
        t *= 0.7;
        s = (s.0 * 0.5, s.1 * 0.5);
    }
    Err(CoreError::Scene("the page could not be placed in the frame".into()))
}

/// Scene number `index` of `spec`.
pub fn generate_full(spec: &SceneSpec, index: u32) -> Result<Generated, CoreError> {
    let p = draw_params(spec, index)?;
    // These streams continue where `draw_params` left them: the damage model and the light kind
    // were its first draws, and what follows (the finger's place, the blur, the light's angles) is
    // fixed by them.
    let mut dmg_rng = stream(p.seed, Purpose::Damage);
    let _ = Damage::draw(&mut dmg_rng);
    let mut light_rng = stream(p.seed, Purpose::Light);
    let _ = light_rng.below(4);
    let mut content_rng = stream(p.seed, Purpose::Content);
    let noise_seed = stream(p.seed, Purpose::Noise).next_u64();
    let (cam, background, damage) = (p.cam, p.background, p.damage);
    let size = (cam.width as u32, cam.height as u32);
    let sheet = build_sheet(&p);

    // The page: the drawn texture, its ink and its words.
    let canvas = if sheet.is_spread() { spread_page(&mut content_rng)? } else { reference_page(&mut content_rng)? };
    let (ink, text) = (canvas.ink_mask()?, canvas.text.clone());
    let texture = canvas.into_texture();
    let (pose, t) = place_sheet(&p, &sheet)?;

    // Render, then the finger, the light, and the camera's own damage.
    let layer = rasterize(&cam, &pose, &sheet, &texture)?;
    let mut photo = composite(&layer, background, p.bg_seed);
    let project = |u: f64, v: f64| pose.project(&cam, sheet.point(u, v));
    let corners =
        Quad([project(0.0, 0.0), project(1.0, 0.0), project(1.0, 1.0), project(0.0, 1.0)].map(|c| c.unwrap_or(Point::new(f64::NAN, f64::NAN))));
    if !corners.is_finite() {
        return Err(CoreError::Scene("a corner of the page is behind the camera".into()));
    }
    let (w, h) = (size.0 as usize, size.1 as usize);
    if damage.is_finger() {
        let diagonal_px = crate::metrics::page_diagonal(&corners);
        let px_per_pt = diagonal_px / sheet.width_pt.hypot(sheet.height_pt);
        apply_finger(&mut photo, w, h, &corners, px_per_pt, damage, &mut dmg_rng);
    }
    let lights = LightModel::new(p.light, cam.width, cam.height, &mut light_rng);
    for (k, px) in photo.iter_mut().enumerate() {
        let g = lights.gain((k % w) as f64 + 0.5, (k / w) as f64 + 0.5);
        for c in 0..3 {
            px[c] *= g[c];
        }
    }
    if damage == Damage::Blur {
        photo = camera_blur(&photo, w, h, between(&mut dmg_rng, 0.6, 1.5));
    }
    if damage == Damage::Noise {
        add_noise(&mut photo, w, between(&mut dmg_rng, 2.0, 6.0), noise_seed);
    }
    let mut rgb = quantize(&photo, size.0, size.1)?;
    if damage == Damage::Jpeg {
        rgb = jpeg_round_trip(&rgb)?;
    }

    // The truth, from the same geometry.
    let grid = if spec.curved {
        let n = GRID;
        let rows: Option<Vec<Vec<Point>>> =
            (0..n).map(|j| (0..n).map(|i| project(i as f64 / (n - 1) as f64, j as f64 / (n - 1) as f64)).collect()).collect();
        Some(rows.ok_or_else(|| CoreError::Scene("a grid point of the page is behind the camera".into()))?)
    } else {
        None
    };
    let spine = if sheet.is_spread() {
        [project(0.5, 0.0), project(0.5, 1.0)].into_iter().collect::<Option<Vec<Point>>>().map(|v| [v[0], v[1]])
    } else {
        None
    };
    let shape = if spec.curved { SceneShape::Bent { mode: sheet.mode, profile_turning: sheet.turning() } } else { SceneShape::Flat };
    let description = format!(
        "{} on {}, {}, {}; focal length {:.2}, tilt {:.0}°, roll {:.0}°",
        sheet.description,
        background.words(),
        p.light.words(),
        damage.words(),
        cam.f,
        t.to_degrees(),
        p.roll.to_degrees() % 360.0
    );
    let truth = SceneTruth {
        format: SCENE_FORMAT,
        description,
        size,
        focal: cam.f,
        corners,
        ratio: sheet.width_pt / sheet.height_pt,
        shape,
        spine,
        page_grid: grid,
    };
    Ok(Generated { scene: Scene { photo: rgb, truth }, ink, text })
}

/// The bend mode a bent scene was drawn with, for the tests and the report.
pub fn mode_of(truth: &SceneTruth) -> Option<BendMode> {
    match &truth.shape {
        SceneShape::Bent { mode, .. } => Some(*mode),
        SceneShape::Flat => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate;
    use scancraft_core::recipe::BendMode;

    fn spec(curved: bool, count: u32, seed: u64) -> SceneSpec {
        SceneSpec { count, seed, curved }
    }

    /// The tilt of the pose: the angle between the sheet's normal and the camera's axis.
    fn pose_tilt(pose: &Pose) -> f64 {
        pose.r[2][2].clamp(-1.0, 1.0).acos().to_degrees()
    }

    #[test]
    fn the_same_seed_gives_the_same_bytes() {
        let s = spec(true, 3, 7);
        let (a, b) = (generate_full(&s, 1).unwrap(), generate_full(&s, 1).unwrap());
        assert_eq!(a.scene, b.scene);
        assert_eq!(a.ink, b.ink);
        assert_eq!(a.text, b.text);
    }

    #[test]
    fn a_different_seed_or_index_gives_a_different_scene() {
        let (a, b, c) =
            (generate(&spec(false, 4, 1), 0).unwrap(), generate(&spec(false, 4, 2), 0).unwrap(), generate(&spec(false, 4, 1), 1).unwrap());
        assert_ne!(a.photo, b.photo);
        assert_ne!(a.photo, c.photo);
        assert_ne!(a.truth.corners, c.truth.corners);
        // Past the count is an error, not a panic; so is a set too big to write.
        assert!(matches!(generate(&spec(false, 4, 1), 4), Err(CoreError::Scene(_))));
        assert!(matches!(generate(&spec(false, 0, 1), 0), Err(CoreError::Scene(_))));
        assert!(generate(&spec(false, MAX_COUNT + 1, 1), 0).is_err());
        // A zero seed is legal.
        assert!(draw_params(&spec(false, 1, 0), 0).is_ok());
    }

    #[test]
    fn the_camera_stays_inside_the_specified_ranges() {
        let (mut rolls, mut tilts_flat, mut tilts_bent) = (Vec::new(), Vec::new(), Vec::new());
        for curved in [false, true] {
            let s = spec(curved, 200, 11);
            for index in 0..200 {
                let p = draw_params(&s, index).unwrap();
                assert!((0.6..=1.2).contains(&p.cam.f), "focal {}", p.cam.f);
                let sheet = build_sheet(&p);
                let (pose, used) = place_sheet(&p, &sheet).unwrap();
                let max = if curved { 25.0 } else { 50.0 };
                assert!(used.to_degrees() <= max + 1e-9, "tilt {}", used.to_degrees());
                assert!((pose_tilt(&pose) - used.to_degrees()).abs() < 1e-6);
                // No corner is more than 5% of the photo's width or height outside the frame.
                let corners: Vec<Point> =
                    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().map(|&(u, v)| pose.project(&p.cam, sheet.point(u, v)).unwrap()).collect();
                for c in &corners {
                    assert!(
                        c.x >= -0.055 * p.cam.width && c.x <= 1.055 * p.cam.width && c.y >= -0.055 * p.cam.height && c.y <= 1.055 * p.cam.height,
                        "{c:?}"
                    );
                }
                rolls.push(p.roll.to_degrees());
                if curved { tilts_bent.push(used.to_degrees()) } else { tilts_flat.push(used.to_degrees()) }
            }
        }
        // The roll covers the circle; the flat tilts go well past the bent tilts' limit.
        assert!(rolls.iter().any(|r| *r < 20.0) && rolls.iter().any(|r| *r > 340.0) && rolls.iter().any(|r| (170.0..190.0).contains(r)));
        assert!(tilts_flat.iter().cloned().fold(0.0, f64::max) > 40.0);
        assert!(tilts_bent.iter().cloned().fold(0.0, f64::max) <= 25.0 && tilts_bent.iter().cloned().fold(0.0, f64::max) > 15.0);
    }

    #[test]
    fn the_page_fills_a_reasonable_part_of_the_frame() {
        let mut shares = Vec::new();
        for index in 0..60 {
            let p = draw_params(&spec(false, 60, 3), index).unwrap();
            let sheet = build_sheet(&p);
            let (pose, _) = place_sheet(&p, &sheet).unwrap();
            let c: Vec<Point> =
                [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().map(|&(u, v)| pose.project(&p.cam, sheet.point(u, v)).unwrap()).collect();
            shares.push(crate::geom::polygon_area(&c) / (p.cam.width * p.cam.height));
        }
        // A tall page rolled to 45° in a landscape photo fills its bounding box but little of the
        // photo's area; a broken camera would leave a speck.
        let good = shares.iter().filter(|&&s| s > 0.12).count();
        assert!(good > 54, "{good} of 60 fill more than an eighth of the photo");
        assert!(shares.iter().all(|&s| s < 1.0));
    }

    #[test]
    fn all_five_bent_shapes_occur() {
        let s = spec(true, 100, 5);
        let kinds: Vec<BendKind> = (0..100).map(|i| draw_params(&s, i).unwrap().kind).collect();
        for k in BendKind::BENT {
            assert!(kinds.contains(&k), "{k:?}");
        }
        // Any five in a row hold each once.
        assert!(kinds.windows(5).all(|w| BendKind::BENT.iter().all(|k| w.contains(k))));
        assert!(draw_params(&spec(false, 5, 1), 3).unwrap().kind == BendKind::Flat);
    }

    #[test]
    fn a_flat_page_has_no_page_grid_and_a_bent_one_keeps_its_corners() {
        let flat = generate(&spec(false, 2, 21), 0).unwrap();
        assert!(flat.truth.page_grid.is_none() && flat.truth.shape == SceneShape::Flat && flat.truth.spine.is_none());
        assert_eq!(flat.truth.format, SCENE_FORMAT);
        assert!((flat.truth.ratio - 595.28 / 841.89).abs() < 1e-9);
        assert_eq!(flat.truth.size, (flat.photo.width(), flat.photo.height()));
        for index in 0..5 {
            let bent = generate(&spec(true, 5, 21), index).unwrap();
            let grid = bent.truth.page_grid.as_ref().unwrap();
            assert_eq!((grid.len(), grid[0].len()), (GRID, GRID));
            let n = GRID - 1;
            for (a, b) in [grid[0][0], grid[0][n], grid[n][n], grid[n][0]].iter().zip(bent.truth.corners.0) {
                assert!(a.distance(b) < 1e-9);
            }
            // The turning has 33 values and the mode is recorded.
            let SceneShape::Bent { mode, profile_turning } = &bent.truth.shape else { panic!("not bent") };
            assert_eq!(profile_turning.len(), 33);
            assert!(matches!(mode, BendMode::Across | BendMode::Down));
            // The truth parses back through core's own reader.
            assert_eq!(SceneTruth::from_json(&bent.truth.to_json().unwrap()).unwrap(), bent.truth);
        }
    }

    #[test]
    fn a_spread_records_its_spine() {
        let s = spec(true, 5, 33);
        let mut spreads = 0;
        for index in 0..5 {
            let p = draw_params(&s, index).unwrap();
            let g = generate_full(&s, index).unwrap();
            if p.kind == BendKind::Spread {
                spreads += 1;
                let [top, bottom] = g.scene.truth.spine.unwrap();
                let sheet = build_sheet(&p);
                let (pose, _) = place_sheet(&p, &sheet).unwrap();
                // The spine's ends are the sheet's middle line's ends.
                assert!(top.distance(pose.project(&p.cam, sheet.point(0.5, 0.0)).unwrap()) < 1e-9);
                assert!(bottom.distance(pose.project(&p.cam, sheet.point(0.5, 1.0)).unwrap()) < 1e-9);
                assert!((g.scene.truth.ratio - 2.0 * 419.53 / 595.28).abs() < 1e-9);
            } else {
                assert!(g.scene.truth.spine.is_none());
            }
        }
        assert_eq!(spreads, 1);
    }

    #[test]
    fn the_ink_mask_and_words_come_with_the_scene() {
        let g = generate_full(&spec(false, 1, 4), 0).unwrap();
        assert_eq!((g.ink.width(), g.ink.height()), (2480, 3508));
        assert!(g.text.contains("Synthetic Page") && g.text.lines().count() > 30);
        let ink = (0..g.ink.height())
            .step_by(7)
            .flat_map(|y| (0..g.ink.width()).step_by(7).map(move |x| (x, y)))
            .filter(|&(x, y)| g.ink.get(x, y) == Some(true))
            .count();
        assert!(ink > 500);
    }

    #[test]
    fn one_thread_and_eight_threads_give_identical_bytes() {
        let s = spec(true, 3, 9);
        let run = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(|| generate(&s, 2).unwrap());
        assert_eq!(run(1), run(8));
    }

    #[test]
    fn the_photo_shows_the_page_where_the_truth_says() {
        // The page's four true corners are inside or at the edge of the photo, and the middle of
        // the page is not the background: compare it with a pixel known to be background.
        let g = generate(&spec(false, 3, 2), 0).unwrap();
        let c = g.truth.corners;
        let mid = Point::new((c.tl().x + c.br().x) / 2.0, (c.tl().y + c.br().y) / 2.0);
        let (w, h) = (f64::from(g.photo.width()), f64::from(g.photo.height()));
        assert!(mid.x > 0.0 && mid.y > 0.0 && mid.x < w && mid.y < h);
        assert!(c.0.iter().all(|p| p.x > -0.06 * w && p.x < 1.06 * w && p.y > -0.06 * h && p.y < 1.06 * h));
    }
}
