//! The tests of A16 (Even Light) and the Color Document look, from specification A16 and A17:
//! synthetic pages under synthetic light, measured in 8-bit levels on the Color Document look at its
//! default strength.

mod common;

use common::{SHEET, levels, lit, noisy, paper_pixels, pick, quantile, share, text_sheet, to_rgb};
use scancraft_core::image::{BitPlane, RgbF32};
use scancraft_enhance::even_light::{EvenLight, even_light};
use scancraft_enhance::looks::color_document;
use scancraft_synth::page::{INK, PX_PER_PT};

const DPI: f64 = 300.0;

/// Color Document at strength 60 of the page's Even Light, as 8-bit luma levels.
fn output(page: &RgbF32) -> (Vec<u8>, EvenLight) {
    let light = even_light(page, DPI, false).unwrap();
    let look = color_document(&light, 60, DPI).unwrap();
    (levels(&look), light)
}

fn flat(level: f32) -> impl Fn(u32, u32) -> [f32; 3] {
    move |_, _| [level; 3]
}

/// The sheet of text and what is truly ink on it.
fn sheet(seed: u64) -> (RgbF32, BitPlane, scancraft_synth::page::Canvas) {
    let c = text_sheet(seed);
    (to_rgb(&c), c.ink_mask().unwrap(), c)
}

#[test]
fn with_flat_light_the_paper_is_white_and_the_ink_is_black() {
    let (page, ink, _) = sheet(1);
    let (out, _) = output(&lit(&page, flat(0.8)));
    let paper = paper_pixels(&ink, 3, &[]);
    let white = share(&out, &paper, |v| v >= 245);
    assert!(white >= 0.97, "paper at 245 or more: {white}");
    let inked: Vec<bool> = (0..ink.height()).flat_map(|y| (0..ink.width()).map(move |x| (x, y))).map(|(x, y)| ink.get(x, y) == Some(true)).collect();
    let median_ink = quantile(&pick(&out, &inked), 0.5);
    assert!(median_ink <= 30.0, "median ink level {median_ink}");
}

#[test]
fn a_two_and_a_half_to_one_gradient_leaves_a_spread_of_three_percent() {
    let (page, ink, _) = sheet(2);
    let w = page.width() as f32;
    let (out, _) = output(&lit(&page, |x, _| [1.0 - 0.6 * x as f32 / w; 3]));
    let paper = pick(&out, &paper_pixels(&ink, 3, &[]));
    let (low, high) = (quantile(&paper, 0.01), quantile(&paper, 0.99));
    assert!((high - low) / 255.0 <= 0.03, "paper from {low} to {high}");
}

/// What a hard shadow leaves behind: the step in the paper's level across its edge, the mean
/// distance from the paper's median within 15 px of the edge (the halo), and the share of all the
/// paper within 12 levels of the median.
struct Edge {
    step: f64,
    halo: f64,
    near_median: f64,
}

/// Measures the paper of `out` around the edge of a shadow. `inside` says whether a pixel is in the
/// shadow and `distance` how far it is from the edge.
fn edge(out: &[u8], paper: &[bool], w: u32, inside: impl Fn(u32, u32) -> bool, distance: impl Fn(u32, u32) -> f32) -> Edge {
    let median = quantile(&pick(out, paper), 0.5);
    let (mut dark, mut light, mut halo) = ((0.0f64, 0u32), (0.0f64, 0u32), (0.0f64, 0u32));
    for (i, &p) in paper.iter().enumerate() {
        let (x, y) = ((i as u32) % w, (i as u32) / w);
        if !p {
            continue;
        }
        let d = distance(x, y);
        let v = f64::from(out[i]);
        if d <= 10.0 {
            let side = if inside(x, y) { &mut dark } else { &mut light };
            side.0 += v;
            side.1 += 1;
        }
        if d <= 15.0 {
            halo.0 += (v - median).abs();
            halo.1 += 1;
        }
    }
    Edge {
        step: (dark.0 / f64::from(dark.1.max(1)) - light.0 / f64::from(light.1.max(1))).abs(),
        halo: halo.0 / f64::from(halo.1.max(1)),
        near_median: share(out, paper, |v| (f64::from(v) - median).abs() <= 12.0),
    }
}

/// The light of a shadow of ×0.6 whose edge lies where the signed distance `sd` is 0, negative inside
/// the shadow, with a penumbra of `PENUMBRA` px across (a smoothstep, as a shadow's edge is).
fn shadow_light(sd: impl Fn(u32, u32) -> f32) -> impl Fn(u32, u32) -> [f32; 3] {
    move |x, y| {
        let t = (sd(x, y) / PENUMBRA + 0.5).clamp(0.0, 1.0);
        [1.0 - 0.4 * (1.0 - t * t * (3.0 - 2.0 * t)); 3]
    }
}

/// How wide the edge of a "hard" shadow is, in px at 300 DPI: 30 px is 2.5 mm. A shadow's edge
/// narrower than about 13 px of the 1000 px analysis copy, about 4 mm on an A4 sheet, is as sharp as
/// the edge of a printed box, and EL4's sharpness rule reads it as an object, not a shadow; a
/// shadow's penumbra is wider than that.
const PENUMBRA: f32 = 30.0;

#[test]
fn a_hard_shadow_across_the_page_leaves_no_step_and_no_halo() {
    let (page, ink, _) = sheet(3);
    // The left part of the page is in shadow, with a slightly slanted edge.
    let sd = |x: u32, y: u32| (x as f32 - (450.0 + 0.15 * (y as f32 - 775.0))) * 0.989;
    let (out, _) = output(&lit(&page, shadow_light(sd)));
    let e = edge(&out, &paper_pixels(&ink, 3, &[]), page.width(), |x, y| sd(x, y) < 0.0, |x, y| sd(x, y).abs());
    assert!(e.step <= 4.0, "step across the edge: {} levels", e.step);
    assert!(e.halo <= 3.0, "mean halo: {} levels", e.halo);
    assert!(e.near_median >= 0.95, "paper within 12 levels of the median: {}", e.near_median);
}

#[test]
fn a_hard_shadow_on_the_page_leaves_no_step_and_no_halo() {
    let (page, ink, _) = sheet(3);
    // A shadow of 240 × 320 px inside the page.
    let (x0, x1, y0, y1) = (400.0f32, 640.0f32, 600.0f32, 920.0f32);
    let sd = |x: u32, y: u32| {
        let (x, y) = (x as f32, y as f32);
        let (dx, dy) = ((x0 - x).max(x - x1).max(0.0), (y0 - y).max(y - y1).max(0.0));
        if dx > 0.0 || dy > 0.0 { dx.hypot(dy) } else { -(x - x0).min(x1 - x).min(y - y0).min(y1 - y) }
    };
    let (out, _) = output(&lit(&page, shadow_light(sd)));
    let e = edge(&out, &paper_pixels(&ink, 3, &[]), page.width(), |x, y| sd(x, y) < 0.0, |x, y| sd(x, y).abs());
    assert!(e.step <= 4.0, "step across the edge: {} levels", e.step);
    assert!(e.halo <= 3.0, "mean halo: {} levels", e.halo);
    assert!(e.near_median >= 0.95, "paper within 12 levels of the median: {}", e.near_median);
}

#[test]
fn dappled_shade_leaves_the_paper_white() {
    let (page, ink, _) = sheet(4);
    // Six soft patches of shade.
    let blobs = [(200.0f32, 300.0f32), (700.0, 200.0), (400.0, 800.0), (900.0, 1100.0), (250.0, 1300.0), (750.0, 600.0)];
    let light = |x: u32, y: u32| {
        let shade: f32 = blobs.iter().map(|&(bx, by)| (-((x as f32 - bx).powi(2) + (y as f32 - by).powi(2)) / (2.0 * 70.0f32.powi(2))).exp()).sum();
        [(0.9 - 0.4 * shade.min(1.0)).max(0.3); 3]
    };
    let (out, _) = output(&lit(&page, light));
    let white = share(&out, &paper_pixels(&ink, 3, &[]), |v| v >= 245);
    assert!(white >= 0.90, "paper at 245 or more: {white}");
}

#[test]
fn black_bars_that_do_not_touch_the_border_stay_dark() {
    let mut c = text_sheet(5);
    let bar = (60.0, 150.0, 200.0, 170.0);
    c.fill_rect(bar, [0, 0, 0], true);
    let page = to_rgb(&c);
    let w = page.width() as f32;
    let (out, light) = output(&lit(&page, |x, _| [1.0 - 0.4 * x as f32 / w; 3]));
    let (px0, py0, px1, py1) =
        ((bar.0 * PX_PER_PT) as u32 + 3, (bar.1 * PX_PER_PT) as u32 + 3, (bar.2 * PX_PER_PT) as u32 - 3, (bar.3 * PX_PER_PT) as u32 - 3);
    let width = page.width();
    let brightest = (py0..py1).flat_map(|y| (px0..px1).map(move |x| (x, y))).map(|(x, y)| out[(y * width + x) as usize]).max().unwrap();
    assert!(brightest < 60, "the brightest level inside the bar: {brightest}");
    // The bar is one of the objects, which the paper estimate leaves out.
    let (aw, ah) = light.analysis;
    let centre = (((bar.0 + bar.2) / 2.0 * PX_PER_PT) as u32 * aw / width, ((bar.1 + bar.3) / 2.0 * PX_PER_PT) as u32 * ah / page.height());
    assert_eq!(light.objects.get(centre.0, centre.1), Some(true), "the bar is in the object mask");
}

/// The hue of a colour in degrees.
fn hue([r, g, b]: [f32; 3]) -> f32 {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max - min < 1e-6 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / (max - min)).rem_euclid(6.0)
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    h * 60.0
}

#[test]
fn a_skin_patch_at_the_edge_keeps_its_luminance_and_its_hue() {
    let mut c = text_sheet(6);
    let skin = [224u8, 172, 130];
    // A patch against the page's left edge.
    let patch = (0.0, 120.0, 36.0, 190.0);
    c.fill_rect(patch, skin, false);
    let page = to_rgb(&c);
    let (_, light) = output(&lit(&page, flat(0.9)));
    let look = color_document(&light, 60, DPI).unwrap();
    // The patch's colour against the paper's, which is what Even Light makes white.
    let paper = [243.0f32, 241.0, 236.0];
    let expected = [0, 1, 2].map(|i| (f32::from(skin[i]) / paper[i]).clamp(0.0, 1.0));
    let (x0, y0, x1, y1) = (6u32, (patch.1 * PX_PER_PT) as u32 + 6, (patch.2 * PX_PER_PT) as u32 - 6, (patch.3 * PX_PER_PT) as u32 - 6);
    let mut sum = [0f32; 3];
    let mut n = 0f32;
    for y in y1.min(y0)..y1.max(y0) {
        for x in x0..x1 {
            let p = look.get(x, y).unwrap();
            for i in 0..3 {
                sum[i] += p[i];
            }
            n += 1.0;
        }
    }
    let mean = sum.map(|v| v / n);
    let luma = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let ratio = luma(mean) / luma(expected);
    assert!((0.85..=1.15).contains(&ratio), "luminance ratio {ratio}: {mean:?} against {expected:?}");
    let turn = (hue(mean) - hue(expected)).abs();
    assert!(turn <= 8.0, "hue moved by {turn} degrees: {mean:?} against {expected:?}");
}

#[test]
fn white_paper_under_coloured_light_comes_out_neutral() {
    let (page, ink, _) = sheet(7);
    let page = lit(&page, |_, _| [1.0, 0.85, 0.6]);
    let light = even_light(&page, DPI, false).unwrap();
    let look = color_document(&light, 60, DPI).unwrap();
    let paper = paper_pixels(&ink, 3, &[]);
    let w = page.width();
    let mut diffs: Vec<u8> = Vec::new();
    for (i, &p) in paper.iter().enumerate() {
        if !p {
            continue;
        }
        let px = look.get((i as u32) % w, (i as u32) / w).unwrap();
        diffs.push(((px[0] - px[2]).abs() * 255.0).round() as u8);
    }
    let median = quantile(&diffs, 0.5);
    assert!(median <= 4.0, "median |R − B|: {median} levels");
}

#[test]
fn a_pale_pencil_line_stays_visible() {
    // A paper of 0.93 and a pencil line of 0.70.
    let mut c = scancraft_synth::page::Canvas::new(SHEET, [237, 237, 237]).unwrap();
    let mut rng = scancraft_core::determinism::Xorshift64Star::new(8);
    common::paragraphs(&mut c, &mut rng, (20.0, SHEET.0 - 20.0, 40.0, 150.0), 10.0, 14.0);
    c.stroke((20.0, 250.0), (SHEET.0 - 20.0, 252.0), 0.8, [178, 178, 178], true);
    let page = to_rgb(&c);
    let (out, _) = output(&lit(&page, flat(0.8)));
    let w = page.width();
    // The middle of the line.
    let (x, y) = ((SHEET.0 / 2.0 * PX_PER_PT) as u32, (251.0 * PX_PER_PT) as u32);
    let line = (y - 2..=y + 2).map(|yy| out[(yy * w + x) as usize]).min().unwrap();
    let paper = quantile(&pick(&out, &paper_pixels(&c.ink_mask().unwrap(), 4, &[])), 0.5);
    assert!(paper - f64::from(line) >= 40.0, "the line at {line} on paper at {paper}");
}

#[test]
fn a_blank_page_is_flat() {
    let c = scancraft_synth::page::Canvas::new(SHEET, [243, 241, 236]).unwrap();
    let page = noisy(&lit(&to_rgb(&c), |x, _| [0.9 - 0.3 * x as f32 / 1100.0; 3]), 2.0 / 255.0, 9);
    let (out, _) = output(&page);
    let mean = out.iter().map(|&v| f64::from(v)).sum::<f64>() / out.len() as f64;
    let sd = (out.iter().map(|&v| (f64::from(v) - mean).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
    assert!(sd <= 2.0, "standard deviation {sd} levels");
}

#[test]
fn one_thread_and_eight_threads_give_the_same_bytes() {
    let (page, _, _) = sheet(10);
    let page = lit(&page, |x, y| [0.9 - 0.4 * x as f32 / 1100.0 + 0.05 * (y as f32 / 90.0).sin(); 3]);
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let light = even_light(&page, DPI, true).unwrap();
            let look = color_document(&light, 60, DPI).unwrap();
            (light, look)
        })
    };
    let (one, many) = (run(1), run(8));
    assert_eq!(one.0, many.0);
    assert_eq!(one.1, many.1);
}

#[test]
fn keeping_the_paper_colour_keeps_a_tint() {
    // Cream paper under white light: with Keep paper colour on, the paper stays cream.
    let mut c = scancraft_synth::page::Canvas::new(SHEET, [236, 226, 196]).unwrap();
    let mut rng = scancraft_core::determinism::Xorshift64Star::new(11);
    common::paragraphs(&mut c, &mut rng, (20.0, SHEET.0 - 20.0, 40.0, 300.0), 10.0, 14.0);
    let page = to_rgb(&c);
    let ink = c.ink_mask().unwrap();
    let paper = paper_pixels(&ink, 3, &[]);
    let spread = |light: &EvenLight| {
        let w = page.width();
        let mut v: Vec<u8> = Vec::new();
        for (i, &p) in paper.iter().enumerate() {
            if p {
                let px = light.corrected.get((i as u32) % w, (i as u32) / w).unwrap();
                v.push(((px[0] - px[2]).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        quantile(&v, 0.5)
    };
    let plain = spread(&even_light(&page, DPI, false).unwrap());
    let kept = spread(&even_light(&page, DPI, true).unwrap());
    assert!(plain <= 4.0, "without it the paper is neutral: {plain}");
    assert!(kept >= 20.0, "with it the paper keeps its tint: {kept}");
}

#[test]
fn the_ink_comes_out_black_not_tinted() {
    // Ink printed in a bluish black.
    let mut c = scancraft_synth::page::Canvas::new(SHEET, [243, 241, 236]).unwrap();
    c.text(20.0, 60.0, 36.0, "Black", [24, 24, 64]);
    let light = even_light(&to_rgb(&c), DPI, false).unwrap();
    let ink = c.ink_mask().unwrap();
    let mut worst = 0f32;
    let mut count = 0;
    for y in 0..ink.height() {
        for x in 0..ink.width() {
            if ink.get(x, y) == Some(true) {
                let p = light.corrected.get(x, y).unwrap();
                worst = worst.max((p[2] - p[0]).abs());
                count += 1;
            }
        }
    }
    assert!(count > 100);
    // EL10 takes out up to 0.08 of the tint, and the ink's own tint of 40/255 = 0.157 leaves about
    // half of it.
    let before = (64.0 - 24.0) / 255.0;
    assert!(worst < before, "the ink's blue is {worst}, down from {before}");
    let _ = INK;
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let page = scancraft_core::image::Plane::new(40, 40, [0.9f32; 3]).unwrap();
    assert!(even_light(&page, 0.0, false).is_err());
    assert!(even_light(&page, -300.0, false).is_err());
    assert!(even_light(&page, f64::NAN, false).is_err());
    // A page with NaN pixels does not panic.
    let mut nan = page.clone();
    nan.set(3, 3, [f32::NAN; 3]);
    let _ = even_light(&nan, DPI, false);
    // Tiny pages work.
    for (w, h) in [(1, 1), (2, 3), (7, 5), (1, 64)] {
        let tiny = scancraft_core::image::Plane::new(w, h, [0.8f32; 3]).unwrap();
        let light = even_light(&tiny, DPI, false).unwrap();
        assert_eq!((light.corrected.width(), light.corrected.height()), (w, h));
        assert!(color_document(&light, 60, DPI).is_ok());
    }
}
