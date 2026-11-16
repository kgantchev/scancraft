//! The tests of A26 (text lines), from the specification: sloped lines are followed, two columns
//! are followed across, a sideways page gives its direction, and a page without text gives no
//! evidence.

mod common;

use common::{X_HEIGHT, copy_of, middle_in_strip, pyramid, sheet_quad, size_mm, text_sheet, to_rgb8, turn, two_column_sheet, whole_quad};
use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::Plane;
use scancraft_detect::text_lines::{lines_in_copy, text_evidence};
use scancraft_geom::sheet::TextDirection;

/// The size of the text the tests use, in points, and the leading.
const SIZE: f64 = 10.0;
const LEADING: f64 = 14.0;
/// The number of strips the copy is cut into (A26 step 3).
const STRIPS: usize = 32;

/// The x-height of the text, in the copy's rows.
fn x_height(dpi: f64) -> f64 {
    SIZE * X_HEIGHT * dpi / 72.0
}

/// How far every point of the tracked lines is from the line's true middle, in the copy's rows.
///
/// The true middle is measured from the page: the centre of mass of the line's ink over its band,
/// in the columns of the strip the point belongs to. A point's own strip is used because the ink
/// of a line is not spread evenly over the page, and the tracker measures the line where it is.
///
/// A sloped copy is measured on the page it came from: the shear moves a point at (x, y) to
/// (x, y + slope · x), so the point's place on the page is (x, y − slope · x).
fn deviations(copy: &scancraft_core::image::PlaneF32, baselines: &[f64], line: &[scancraft_core::geom::Point], dpi: f64, slope: f64) -> Vec<f64> {
    let width = copy.width() as usize;
    let height = x_height(dpi);
    let mut out = Vec::with_capacity(line.len());
    for point in line {
        let on_the_page = point.y - slope * point.x;
        let nearest = *baselines.iter().min_by(|a, b| (**a - on_the_page).abs().total_cmp(&(**b - on_the_page).abs())).unwrap_or(&0.0);
        let strip = ((point.x.max(0.0) as usize * STRIPS) / width).min(STRIPS - 1);
        let x0 = (strip * width / STRIPS) as u32;
        let x1 = (((strip + 1) * width / STRIPS) as u32).max(x0 + 1);
        let truth = middle_in_strip(copy, x0, x1, nearest, height);
        out.push((on_the_page - truth).abs());
    }
    out
}

#[test]
fn thirty_sloped_lines_are_followed() {
    let lines = 30;
    let (sheet, baselines) = text_sheet(lines, SIZE, LEADING, 1);
    let (copy, dpi) = copy_of(&sheet);
    // The copy's lines slope by 0.2, as they do when the page is bent. The slope is what the
    // tracker can follow at this page's size: the tolerance is 60 % of the leading, and a line
    // that rises by more than that between two strips cannot be kept.
    let slope = 0.2f64;
    let sloped = common::shear(&copy, slope as f32);
    let found = lines_in_copy(&sloped, dpi).unwrap();
    // The baselines in the copy's rows.
    let scale = f64::from(copy.height()) / f64::from(sheet.height);
    let baselines: Vec<f64> = baselines.iter().map(|b| b * (300.0 / 72.0) * scale).collect();
    let height = x_height(dpi);
    // Every point is on the line: within 0.15 x-heights of the line's true middle.
    let mut followed = 0usize;
    let mut all = Vec::new();
    for line in &found.lines {
        let devs = deviations(&copy, &baselines, line, dpi, slope);
        // A line that is not on a line of text is not one of them: the tracker sometimes follows the
        // band of a line's tall letters.
        if !devs.iter().any(|d| *d <= height) {
            continue;
        }
        followed += 1;
        all.extend(devs);
    }
    assert!(followed >= 27, "{followed} of {lines} lines followed");
    let tolerance = 0.15 * height;
    let within = all.iter().filter(|d| **d <= tolerance).count();
    assert!(within * 10 >= all.len() * 9, "{} of {} points within {tolerance:.1} px", within, all.len());
    // A point that is further away than that has jumped to another band of ink.
    let worst = all.iter().cloned().fold(0.0f64, f64::max);
    assert!(worst <= 2.5 * height, "the worst point is {worst:.1} px from the true middle, and {} px is the limit", 2.5 * height);
}

#[test]
fn the_lines_of_a_flat_page_are_followed_exactly() {
    let lines = 30;
    let (sheet, baselines) = text_sheet(lines, SIZE, LEADING, 2);
    let (copy, dpi) = copy_of(&sheet);
    let found = lines_in_copy(&copy, dpi).unwrap();
    assert!(found.lines.len() >= lines, "{} lines followed", found.lines.len());
    let scale = f64::from(copy.height()) / f64::from(sheet.height);
    let baselines: Vec<f64> = baselines.iter().map(|b| b * (300.0 / 72.0) * scale).collect();
    let height = x_height(dpi);
    let mut all = Vec::new();
    for line in &found.lines {
        let devs = deviations(&copy, &baselines, line, dpi, 0.0);
        if !devs.iter().any(|d| *d <= height) {
            continue;
        }
        all.extend(devs);
    }
    let tolerance = 0.15 * height;
    let within = all.iter().filter(|d| **d <= tolerance).count();
    assert!(within * 20 >= all.len() * 19, "{} of {} points within {tolerance:.1} px", within, all.len());
}

#[test]
fn two_columns_with_aligned_baselines_are_followed_across_both() {
    let lines = 20;
    let (sheet, baselines) = two_column_sheet(lines, SIZE, LEADING, 2);
    let (copy, dpi) = copy_of(&sheet);
    let found = lines_in_copy(&copy, dpi).unwrap();
    assert!(found.lines.len() >= lines - 2, "{} of {lines} lines followed", found.lines.len());
    // Each line spans both columns: its first and last points are over 70 % of the width apart.
    let width = f64::from(copy.width());
    let mut widest = 0.0f64;
    for line in &found.lines {
        let first = line.first().map(|p| p.x).unwrap_or(0.0);
        let last = line.last().map(|p| p.x).unwrap_or(0.0);
        widest = widest.max(last - first);
    }
    assert!(widest > 0.7 * width, "the lines span {widest:.0} px of {width:.0}");
    // The lines are at the drawn baselines.
    let scale = f64::from(copy.height()) / f64::from(sheet.height);
    let baselines: Vec<f64> = baselines.iter().map(|b| b * (300.0 / 72.0) * scale).collect();
    for line in &found.lines {
        let height = line.first().map(|p| p.y).unwrap_or(0.0);
        let closest = baselines.iter().map(|b| (b - height).abs()).fold(f64::INFINITY, f64::min);
        assert!(closest <= x_height(dpi), "a line at {height:.1} is {closest:.1} px from the nearest truth");
    }
}

#[test]
fn a_page_turned_sideways_gives_the_down_direction() {
    let (sheet, _) = text_sheet(20, SIZE, LEADING, 3);
    let (copy, dpi) = copy_of(&sheet);
    let sideways = turn(&copy);
    let found = lines_in_copy(&sideways, dpi).unwrap();
    assert_eq!(found.direction, TextDirection::Down);
    assert!(found.lines.len() >= 18, "{} lines followed", found.lines.len());
    // The other way round, the direction is across.
    let upright = lines_in_copy(&copy, dpi).unwrap();
    assert_eq!(upright.direction, TextDirection::Across);
    assert!(upright.lines.len() >= 18, "{} lines followed", upright.lines.len());
}

#[test]
fn a_photo_a_blank_page_and_noise_give_fewer_than_four_lines() {
    let (copy, dpi) = {
        let (sheet, _) = text_sheet(4, SIZE, LEADING, 4);
        copy_of(&sheet)
    };
    // A picture: a smooth sky over hills, with no text at all.
    let (w, h) = (copy.width(), copy.height());
    let mut data = vec![0f32; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
            let hill = 0.62 + 0.12 * (u * 7.0).sin();
            data[(y as usize) * (w as usize) + (x as usize)] = if v > hill { 0.3 } else { 0.8 };
        }
    }
    let photo = Plane::from_vec(w, h, data).unwrap();
    assert!(lines_in_copy(&photo, dpi).unwrap().lines.len() < 4, "a photo gives no lines");

    // A blank page.
    let blank = Plane::new(w, h, 0.92f32).unwrap();
    assert!(lines_in_copy(&blank, dpi).unwrap().lines.len() < 4, "a blank page gives no lines");

    // Noise.
    let mut rng = Xorshift64Star::new(7);
    let noise: Vec<f32> = (0..w * h).map(|_| rng.next_f64() as f32).collect();
    let noise = Plane::from_vec(w, h, noise).unwrap();
    assert!(lines_in_copy(&noise, dpi).unwrap().lines.len() < 4, "noise gives no lines");
}

#[test]
fn the_lines_are_mapped_back_to_the_photo() {
    let lines = 20;
    let (sheet, baselines) = text_sheet(lines, SIZE, LEADING, 5);
    let _photo = to_rgb8(&sheet);
    let pyramid = pyramid(&sheet);
    let outline = sheet_quad(&sheet, 0.0);
    let Some((evidence, direction)) = text_evidence(&pyramid, &outline, size_mm(&sheet)).unwrap() else {
        panic!("the page has text");
    };
    assert_eq!(direction, TextDirection::Across);
    assert!(evidence.len() >= lines - 2, "{} of {lines} lines found", evidence.len());
    // The points are on the photo's text: each line's height is one of the drawn lines.
    let truths: Vec<f64> = baselines.iter().map(|b| b * (300.0 / 72.0)).collect();
    let mut on_text = 0usize;
    for line in &evidence {
        let height = line.points.first().map(|p| p.y).unwrap_or(0.0);
        let closest = truths.iter().map(|t| (t - height).abs()).fold(f64::INFINITY, f64::min);
        if closest <= x_height(300.0) {
            on_text += 1;
        }
        // The points run across the page, and every one carries an uncertainty.
        assert!(line.points.len() >= 5);
        assert_eq!(line.points.len(), line.sigma.len());
        assert!(line.sigma.iter().all(|s| *s > 0.0));
    }
    assert!(on_text >= lines - 2, "{on_text} of {lines} lines are on the text");
}

#[test]
fn a_page_without_text_gives_no_evidence() {
    let blank = Plane::new(600, 800, 0.92f32).unwrap();
    let pyramid = scancraft_detect::refine::build_pyramid(&blank).unwrap();
    let outline = whole_quad(600, 800);
    assert!(text_evidence(&pyramid, &outline, (210.0, 280.0)).unwrap().is_none());
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let page = Plane::new(40, 40, 0.9f32).unwrap();
    assert!(lines_in_copy(&page, 0.0).is_err());
    assert!(lines_in_copy(&page, -300.0).is_err());
    assert!(lines_in_copy(&page, f64::NAN).is_err());
    // A tiny page is not an error.
    assert!(lines_in_copy(&Plane::new(1, 1, 0.9f32).unwrap(), 300.0).is_ok());
    assert!(lines_in_copy(&Plane::new(3, 2, 0.9f32).unwrap(), 300.0).is_ok());
    // The whole evidence: bad corners and a bad page size.
    let pyramid = scancraft_detect::refine::build_pyramid(&page).unwrap();
    let outline = whole_quad(40, 40);
    assert!(text_evidence(&pyramid, &outline, (0.0, 280.0)).is_err());
    assert!(text_evidence(&pyramid, &outline, (210.0, -1.0)).is_err());
    let bad = scancraft_core::geom::Quad([scancraft_core::geom::Point::new(f64::NAN, 0.0); 4]);
    assert!(text_evidence(&pyramid, &bad, (210.0, 280.0)).is_err());
    // A page of one pixel has no page to straighten: an error, not a panic.
    let tiny = scancraft_detect::refine::build_pyramid(&Plane::new(1, 1, 0.9f32).unwrap()).unwrap();
    assert!(text_evidence(&tiny, &outline, (21.0, 28.0)).is_err());
}
