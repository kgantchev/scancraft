//! The tests of A17 (Color Document, Grayscale) and A18 (Black & White), from the specification:
//! synthetic pages through Even Light and each look.

mod common;

use common::{SHEET, at_density, lit, noisy, paragraphs, photographed, sized_sheet, text_sheet, to_rgb};
use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::{BitPlane, Plane, PlaneF32, RgbF32};
use scancraft_enhance::even_light::{EvenLight, even_light};
use scancraft_enhance::looks::{black_white, color_document, grayscale};
use scancraft_raster::colour::{to_u8, to_u8_plane};
use scancraft_raster::helpers::unsharp;
use scancraft_synth::metrics::binarization_f;
use scancraft_synth::page::{Canvas, INK, PAPER, PENCIL, PX_PER_PT};

/// The light a photo of a page has: a flat exposure, the camera's blur and sensor noise.
fn photograph(page: &RgbF32, dpi: f64, seed: u64) -> RgbF32 {
    lit(&photographed(page, dpi, 3.0 / 255.0, seed), |_, _| [0.85; 3])
}

/// Black & White at the default strength of Even Light's result for a page.
fn binarize(page: &RgbF32, dpi: f64, strength: u8) -> BitPlane {
    black_white(&even_light(page, dpi, false).unwrap(), strength, dpi).unwrap()
}

fn bits(plane: &BitPlane, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<(u32, u32, bool)> {
    (y0..y1.min(plane.height())).flat_map(|y| (x0..x1.min(plane.width())).map(move |x| (x, y, plane.get(x, y).unwrap()))).collect()
}

#[test]
fn strength_zero_gives_the_warped_page_byte_for_byte() {
    let c = text_sheet(1);
    let page = lit(&to_rgb(&c), |x, _| [0.9 - 0.4 * x as f32 / 1100.0; 3]);
    let light = even_light(&page, 300.0, false).unwrap();
    let look = color_document(&light, 0, 300.0).unwrap();
    assert_eq!(to_u8(&look).unwrap(), to_u8(&page).unwrap());
    // The Grayscale look at strength 0 is the warped page's luma.
    let grey = grayscale(&light, 0, 300.0).unwrap();
    assert_eq!(to_u8_plane(&grey).unwrap(), to_u8_plane(&scancraft_raster::colour::luma_plane(&page).unwrap()).unwrap());
}

#[test]
fn the_grayscale_look_is_the_tone_curve_exactly() {
    let c = text_sheet(2);
    let page = lit(&to_rgb(&c), |_, _| [0.8; 3]);
    let light = even_light(&page, 300.0, false).unwrap();
    for strength in [20u8, 60, 100] {
        // T from the specification, written out: the unsharp mask of the luma of n, then
        // clamp((Y − bp) / (wp − bp), 0, 1)^γ with bp = 0.42 k, wp = 1 − 0.05 k and γ = 1 + 0.67 k.
        let luma = scancraft_raster::colour::luma_plane(&light.corrected).unwrap();
        let sharp = unsharp(&luma, 0.8, 0.5, 0.01).unwrap();
        let k = f32::from(strength) / 100.0;
        let (bp, wp, gamma) = (0.42 * k, 1.0 - 0.05 * k, 1.0 + 0.67 * k);
        let t: Vec<f32> = sharp.data().iter().map(|&y| ((y - bp) / (wp - bp)).clamp(0.0, 1.0).powf(gamma)).collect();
        let want = Plane::from_vec(sharp.width(), sharp.height(), t).unwrap();
        let got = grayscale(&light, strength, 300.0).unwrap();
        assert_eq!(got, want, "strength {strength}");
    }
    // At strength 60 the constants are a black point of 0.25, a white point of 0.97 and a gamma of 1.4.
    let k = 0.6f32;
    assert!((0.42 * k - 0.252).abs() < 1e-6 && (1.0 - 0.05 * k - 0.97).abs() < 1e-6 && (1.0 + 0.67 * k - 1.402).abs() < 1e-6);
}

#[test]
fn the_slider_has_no_jump_near_zero() {
    let c = text_sheet(3);
    let page = lit(&to_rgb(&c), |_, _| [0.8; 3]);
    let light = even_light(&page, 300.0, false).unwrap();
    let levels = |look: &RgbF32| to_u8(look).unwrap().into_vec();
    let (warped, one, ten, nineteen, twenty) = (
        levels(&page),
        levels(&color_document(&light, 1, 300.0).unwrap()),
        levels(&color_document(&light, 10, 300.0).unwrap()),
        levels(&color_document(&light, 19, 300.0).unwrap()),
        levels(&color_document(&light, 20, 300.0).unwrap()),
    );
    let mean_gap = |a: &[[u8; 3]], b: &[[u8; 3]]| {
        a.iter().zip(b).map(|(x, y)| (0..3).map(|i| f64::from(x[i].abs_diff(y[i]))).sum::<f64>() / 3.0).sum::<f64>() / a.len() as f64
    };
    // Strengths 1 to 19 blend the strength-20 result with the warped page by s/20.
    assert!(mean_gap(&one, &warped) < 0.1 * mean_gap(&twenty, &warped), "strength 1 is almost the warped page");
    assert!(mean_gap(&nineteen, &twenty) < 0.1 * mean_gap(&twenty, &warped), "strength 19 is almost strength 20");
    let half = mean_gap(&ten, &warped) / mean_gap(&twenty, &warped);
    assert!((0.4..=0.6).contains(&half), "strength 10 is halfway: {half}");
}

#[test]
fn a_higher_strength_is_a_stronger_look() {
    let c = text_sheet(4);
    let page = lit(&to_rgb(&c), |_, _| [0.8; 3]);
    let light = even_light(&page, 300.0, false).unwrap();
    let mean_dark = |s: u8| {
        let g = grayscale(&light, s, 300.0).unwrap();
        g.data().iter().map(|&v| f64::from(1.0 - v)).sum::<f64>() / g.data().len() as f64
    };
    // The curve's black point and gamma darken the page's tones as the strength grows.
    assert!(mean_dark(30) < mean_dark(60) && mean_dark(60) < mean_dark(100));
}

#[test]
fn bad_strength_or_pieces_that_do_not_belong_together_are_errors() {
    let page = Plane::new(40, 40, [0.8f32; 3]).unwrap();
    let light = even_light(&page, 300.0, false).unwrap();
    assert!(color_document(&light, 101, 300.0).is_err());
    assert!(grayscale(&light, 255, 300.0).is_err());
    assert!(black_white(&light, 101, 300.0).is_err());
    assert!(color_document(&light, 60, 0.0).is_err());
    assert!(black_white(&light, 55, f64::NAN).is_err());
    let mut broken: EvenLight = light.clone();
    broken.objects = BitPlane::new(3, 3).unwrap();
    assert!(color_document(&broken, 60, 300.0).is_err());
    let mut other = light;
    other.warped = Plane::new(41, 40, [0.8f32; 3]).unwrap();
    assert!(grayscale(&other, 60, 300.0).is_err());
}

#[test]
fn printed_text_is_found_at_300_and_200_dpi() {
    // The reference page's 12 pt text, photographed with the camera's blur and sensor noise.
    for (dpi, least) in [(300.0, 0.96), (200.0, 0.90)] {
        let mut worst: f64 = 1.0;
        for seed in 1..=3 {
            let (rgb, truth) = at_density(&sized_sheet(seed, 12.0), dpi);
            let found = binarize(&photograph(&rgb, dpi, seed), dpi, 55);
            worst = worst.min(binarization_f(&truth, &found).unwrap());
        }
        assert!(worst >= least, "{dpi} DPI: the worst F-measure is {worst}, wanted {least}");
    }
}

/// The share of the truth's ink inside the rectangle (in points) that the result found.
fn recall(truth: &BitPlane, found: &BitPlane, rect: (f64, f64, f64, f64)) -> f64 {
    let (x0, y0, x1, y1) = ((rect.0 * PX_PER_PT) as u32, (rect.1 * PX_PER_PT) as u32, (rect.2 * PX_PER_PT) as u32, (rect.3 * PX_PER_PT) as u32);
    let (mut hit, mut all) = (0u32, 0u32);
    for (x, y, t) in bits(truth, x0, y0, x1, y1) {
        if t {
            all += 1;
            hit += u32::from(found.get(x, y) == Some(true));
        }
    }
    f64::from(hit) / f64::from(all.max(1))
}

#[test]
fn a_pale_pencil_line_and_a_bold_heading_are_both_found() {
    let mut c = Canvas::new(SHEET, PAPER).unwrap();
    let mut rng = Xorshift64Star::new(5);
    c.text(20.0, 50.0, 36.0, "Heading", INK);
    paragraphs(&mut c, &mut rng, (20.0, SHEET.0 - 20.0, 90.0, 200.0), 10.0, 14.0);
    // A pencil line of contrast 0.2, away from the print.
    c.stroke((20.0, 300.0), (SHEET.0 - 20.0, 302.0), 0.8, PENCIL, true);
    let truth = c.ink_mask().unwrap();
    let found = binarize(&photograph(&to_rgb(&c), 300.0, 5), 300.0, 55);
    let (heading, pencil) = (recall(&truth, &found, (15.0, 20.0, SHEET.0, 60.0)), recall(&truth, &found, (15.0, 296.0, SHEET.0 - 15.0, 306.0)));
    assert!(heading >= 0.85, "heading recall {heading}");
    assert!(pencil >= 0.85, "pencil recall {pencil}");
}

#[test]
fn blank_noisy_paper_has_almost_no_ink() {
    let c = Canvas::new(SHEET, PAPER).unwrap();
    let blank = lit(&noisy(&to_rgb(&c), 6.0 / 255.0, 6), |_, _| [0.85; 3]);
    let found = binarize(&blank, 300.0, 55);
    let count = bits(&found, 0, 0, found.width(), found.height()).iter().filter(|b| b.2).count();
    let share = count as f64 / (f64::from(found.width()) * f64::from(found.height()));
    assert!(share < 1e-4, "{share}");
}

#[test]
fn a_solid_black_box_stays_black() {
    let mut c = Canvas::new(SHEET, PAPER).unwrap();
    c.fill_rect((40.0, 100.0, 140.0, 160.0), [0, 0, 0], true);
    let truth = c.ink_mask().unwrap();
    let found = binarize(&photograph(&to_rgb(&c), 300.0, 7), 300.0, 55);
    let r = recall(&truth, &found, (30.0, 90.0, 150.0, 170.0));
    assert!(r >= 0.97, "{r}");
}

#[test]
fn full_stops_and_the_dots_on_i_survive_at_low_resolution() {
    for dpi in [150.0, 200.0, 300.0] {
        let mut c = Canvas::new(SHEET, PAPER).unwrap();
        let centres: Vec<(f64, f64)> = (0..10).map(|i| (30.0 + 20.0 * f64::from(i), 60.0 + f64::from(i % 3) * 25.0)).collect();
        // Dots 0.4 mm across: a full stop at 10 pt, a little under the dot of an i.
        for &(x, y) in &centres {
            c.stroke((x, y), (x, y), 0.4 / 25.4 * 72.0, INK, true);
        }
        let (rgb, _) = at_density(&c, dpi);
        let found = binarize(&photograph(&rgb, dpi, 8), dpi, 55);
        let per_pt = dpi / 72.0;
        for &(x, y) in &centres {
            let (px, py) = ((x * per_pt) as u32, (y * per_pt) as u32);
            let kept = bits(&found, px.saturating_sub(2), py.saturating_sub(2), px + 3, py + 3).iter().any(|b| b.2);
            assert!(kept, "{dpi} DPI: the dot at ({x}, {y}) pt was lost");
        }
    }
}

/// An Even Light result built by hand: a page of paper with a photo, and the photo as an object.
fn page_with_photo(photo_share: f64) -> (EvenLight, (u32, u32, u32, u32)) {
    let (w, h) = (600u32, 800u32);
    let side = (f64::from(w) * f64::from(h) * photo_share).sqrt() as u32;
    let (x0, y0) = (150u32, 200u32);
    let mut data = vec![[1.0f32; 3]; (w * h) as usize];
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            // A smooth picture: a ramp across and a swell down, from dark to light.
            let (u, v) = ((x - x0) as f32 / side as f32, (y - y0) as f32 / side as f32);
            let t = (0.15 + 0.5 * u + 0.25 * (v * std::f32::consts::PI).sin()).clamp(0.0, 1.0);
            data[(y * w + x) as usize] = [t, t * 0.95, t * 0.9];
        }
    }
    let page = Plane::from_vec(w, h, data).unwrap();
    let mut objects = BitPlane::new(w, h).unwrap();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            objects.set(x, y, true);
        }
    }
    (EvenLight { corrected: page.clone(), objects, analysis: (w, h), dpi: 300.0, warped: page }, (x0, y0, x0 + side, y0 + side))
}

#[test]
fn a_dithered_photo_keeps_its_mean_darkness() {
    let (light, (x0, y0, x1, y1)) = page_with_photo(0.1);
    let ink = black_white(&light, 55, 300.0).unwrap();
    let tone: PlaneF32 = grayscale(&light, 55, 300.0).unwrap();
    let (mut dark, mut source, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for y in y0 + 4..y1 - 4 {
        for x in x0 + 4..x1 - 4 {
            dark += f64::from(u8::from(ink.get(x, y).unwrap()));
            source += f64::from(1.0 - tone.get(x, y).unwrap());
            n += 1.0;
        }
    }
    let (dithered, original) = (dark / n, source / n);
    assert!((dithered - original).abs() <= 0.1 * original, "dithered {dithered}, source {original}");
    // The dither is a pattern, not a threshold: both ink and paper are there inside the photo.
    assert!(dithered > 0.1 && dithered < 0.9);
}

/// How often a pixel's right-hand neighbour differs from it inside the rectangle: a dither's
/// pattern changes from pixel to pixel, while the midpoint rule gives smooth areas one value.
fn busyness(ink: &BitPlane, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> f64 {
    let (mut changes, mut n) = (0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 - 1 {
            changes += u32::from(ink.get(x, y) != ink.get(x + 1, y));
            n += 1;
        }
    }
    f64::from(changes) / f64::from(n)
}

#[test]
fn only_a_component_of_half_a_percent_of_the_page_is_a_photo() {
    // A photo of 3 % of the page, and the same photo with only a 0.3 % piece of it in the object
    // mask. The photo is dithered; the piece is too small to count as one, so the midpoint rule
    // judges it, and gives a smooth picture's tones large areas of one value.
    let (full, (x0, y0, x1, y1)) = page_with_photo(0.03);
    let middle = ((x0 + x1) / 2, (y0 + y1) / 2);
    let inner = (middle.0 - 14, middle.1 - 14, middle.0 + 14, middle.1 + 14);
    let mut small = full.clone();
    small.objects = BitPlane::new(600, 800).unwrap();
    for y in inner.1 - 4..inner.3 + 4 {
        for x in inner.0 - 4..inner.2 + 4 {
            small.objects.set(x, y, true);
        }
    }
    assert!(f64::from((inner.2 - inner.0 + 8) * (inner.3 - inner.1 + 8)) / (600.0 * 800.0) < 0.005);
    let dithered = busyness(&black_white(&full, 55, 300.0).unwrap(), inner);
    let judged = busyness(&black_white(&small, 55, 300.0).unwrap(), inner);
    assert!(dithered > 0.3, "the photo is dithered: {dithered}");
    assert!(judged < 0.1, "the small piece is not: {judged}");
}

#[test]
fn one_thread_and_eight_threads_give_the_same_look() {
    let c = text_sheet(6);
    let page = photograph(&to_rgb(&c), 300.0, 6);
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let light = even_light(&page, 300.0, false).unwrap();
            (
                color_document(&light, 60, 300.0).unwrap(),
                grayscale(&light, 60, 300.0).unwrap(),
                black_white(&light, 55, 300.0).unwrap(),
                color_document(&light, 12, 300.0).unwrap(),
            )
        })
    };
    assert_eq!(run(1), run(8));
}
