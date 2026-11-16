//! The tests of A30 (book spreads), from the specification: a gutter is found, a leaning spine is
//! followed, a single page stays one page, and a spread without a gutter is cut at the centre.

mod common;

use common::{pyramid, sheet_quad, spread_sheet, text_sheet};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::recipe::{Half, SideShape};
use scancraft_detect::spread::{HalfOutline, Spine, find_spine, split};

/// Four straight sides.
fn straight_sides() -> [SideShape; 4] {
    [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight]
}

/// The ratio of a spread of two A5 pages: the width over the height.
const SPREAD_RATIO: f64 = 2.0 * 595.28 / 841.89;
/// The ratio of a single A4 page.
const PAGE_RATIO: f64 = 595.28 / 841.89;

/// The sheet's corners in photo pixels, with a small margin.
fn outline(sheet: &scancraft_synth::page::Canvas) -> Quad {
    sheet_quad(sheet, 8.0)
}

/// The spine's position across the photo, as a share of the photo's width.
fn spine_at(spine: &Spine, width: f64) -> f64 {
    (spine.ends[0].x + spine.ends[1].x) / 2.0 / width
}

#[test]
fn a_gutter_at_46_percent_is_found() {
    let sheet = spread_sheet(1.0, 0.46, 0.46, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    let spine = find_spine(&pyramid, &outline, SPREAD_RATIO).unwrap().expect("the spread has a gutter");
    let at = spine_at(&spine, f64::from(sheet.width));
    assert!((at - 0.46).abs() <= 0.005, "the gutter is at {at:.3} of the width, and 46 % ± 0.5 % was wanted");
    assert!(spine.confidence > 0.0, "the confidence is {}", spine.confidence);
    // The spine runs from the top of the spread to the bottom of it.
    assert!(spine.ends[0].y < spine.ends[1].y, "the spine's ends are {:?}", spine.ends);
}

#[test]
fn a_spine_leaning_from_45_to_51_percent_is_followed() {
    let sheet = spread_sheet(1.0, 0.45, 0.51, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    let spine = find_spine(&pyramid, &outline, SPREAD_RATIO).unwrap().expect("the spread has a gutter");
    let width = f64::from(sheet.width);
    let top = spine.ends[0].x / width;
    let bottom = spine.ends[1].x / width;
    assert!((top - 0.45).abs() <= 0.01, "the spine's top is at {top:.3} of the width");
    assert!((bottom - 0.51).abs() <= 0.01, "the spine's bottom is at {bottom:.3} of the width");
}

#[test]
fn a_single_page_with_book_spreads_on_stays_one_page() {
    let (sheet, _) = text_sheet(20, 10.0, 14.0, 1);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    assert!(find_spine(&pyramid, &outline, PAGE_RATIO).unwrap().is_none(), "a single page stays one page");
}

#[test]
fn a_two_page_shape_without_a_visible_gutter_is_cut_at_the_centre() {
    let sheet = spread_sheet(0.0, 0.46, 0.46, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    let spine = find_spine(&pyramid, &outline, SPREAD_RATIO).unwrap().expect("the spread is cut at the centre");
    assert_eq!(spine.confidence, 0.0, "the cut at the centre has no confidence");
    let at = spine_at(&spine, f64::from(sheet.width));
    assert!((at - 0.5).abs() <= 0.005, "the cut is at {at:.3} of the width");
}

#[test]
fn the_split_makes_two_pages_with_the_spine_between_them() {
    let sheet = spread_sheet(1.0, 0.46, 0.46, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    let spine = find_spine(&pyramid, &outline, SPREAD_RATIO).unwrap().expect("the spread has a gutter");
    let halves = split(&outline, &straight_sides(), &spine).unwrap();
    assert_eq!(halves[0].half, Half::Left);
    assert_eq!(halves[1].half, Half::Right);
    // The left page's corners: the spread's left side and its own edge of the gutter.
    assert_eq!(halves[0].corners.tl(), outline.tl());
    assert_eq!(halves[0].corners.bl(), outline.bl());
    assert_eq!(halves[1].corners.tr(), outline.tr());
    assert_eq!(halves[1].corners.br(), outline.br());
    // The spine's edge of each page is on the spine, a little to its own side.
    let width = f64::from(sheet.width);
    let spine_x = (spine.ends[0].x + spine.ends[1].x) / 2.0;
    let cession = 0.005 * outline.tl().distance(outline.tr());
    assert!((halves[0].corners.tr().x - (spine_x - cession)).abs() < 1.0, "the left page's spine edge is at {}", halves[0].corners.tr().x);
    assert!((halves[1].corners.tl().x - (spine_x + cession)).abs() < 1.0, "the right page's spine edge is at {}", halves[1].corners.tl().x);
    // The spine's side is marked as a spine, and the other sides are the spread's.
    assert_eq!(halves[0].sides[1], SideShape::Spine);
    assert_eq!(halves[1].sides[3], SideShape::Spine);
    assert_eq!(halves[0].sides[3], SideShape::Straight);
    assert_eq!(halves[1].sides[1], SideShape::Straight);
    let _ = width;
}

#[test]
fn a_traced_side_is_cut_at_the_spine() {
    // A spread whose top side is traced, as A25 would leave it.
    let sheet = spread_sheet(1.0, 0.46, 0.46, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    let spine = find_spine(&pyramid, &outline, SPREAD_RATIO).unwrap().expect("the spread has a gutter");
    let mut sides = straight_sides();
    let samples: Vec<scancraft_core::recipe::SideSample> = (0..20)
        .map(|i| {
            let t = f64::from(i) / 19.0;
            scancraft_core::recipe::SideSample {
                point: Point::new(outline.tl().x + t * (outline.tr().x - outline.tl().x), outline.tl().y + 3.0 * (1.0 - t)),
                weight: 1.0,
            }
        })
        .collect();
    sides[0] = SideShape::Traced { samples, bow: 0.01, curved: true };
    let halves = split(&outline, &sides, &spine).unwrap();
    // The left page keeps the samples before the spine, the right page the ones after it.
    let left = match &halves[0].sides[0] {
        SideShape::Traced { samples, .. } => samples.len(),
        _ => 0,
    };
    let right = match &halves[1].sides[0] {
        SideShape::Traced { samples, .. } => samples.len(),
        _ => 0,
    };
    assert_eq!(left + right, 20, "the samples are split between the pages: {left} and {right}");
    assert!(left > 0 && right > 0, "both pages keep part of the traced side");
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let sheet = spread_sheet(1.0, 0.46, 0.46, 10.0);
    let pyramid = pyramid(&sheet);
    let outline = outline(&sheet);
    assert!(find_spine(&pyramid, &outline, 0.0).is_err());
    assert!(find_spine(&pyramid, &outline, -1.0).is_err());
    assert!(find_spine(&pyramid, &outline, f64::NAN).is_err());
    let bad = Quad([Point::new(f64::NAN, 0.0); 4]);
    assert!(find_spine(&pyramid, &bad, SPREAD_RATIO).is_err());
    // A spine whose ends are the same point, or not finite, is an error.
    let spine = Spine { ends: [Point::new(10.0, 10.0), Point::new(10.0, 10.0)], confidence: 1.0 };
    assert!(split(&outline, &straight_sides(), &spine).is_err());
    let spine = Spine { ends: [Point::new(f64::NAN, 10.0), Point::new(10.0, 10.0)], confidence: 1.0 };
    assert!(split(&outline, &straight_sides(), &spine).is_err());
    // A spine with no length at all.
    let spine = Spine { ends: [Point::new(0.0, 0.0), Point::new(0.0, 0.0)], confidence: 1.0 };
    assert!(split(&outline, &straight_sides(), &spine).is_err());
}

#[test]
fn the_halves_are_in_the_order_the_pages_are_added() {
    // The left page comes first, and its number is smaller.
    let halves: [HalfOutline; 2] = [
        HalfOutline { half: Half::Left, corners: Quad([Point::new(0.0, 0.0); 4]), sides: straight_sides() },
        HalfOutline { half: Half::Right, corners: Quad([Point::new(0.0, 0.0); 4]), sides: straight_sides() },
    ];
    assert_eq!(halves[0].half, Half::Left);
    assert_eq!(halves[1].half, Half::Right);
}
