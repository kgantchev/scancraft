//! A6: contour candidates (specification A6). Owner: WP8.
//!
//! Border following traces the outline of every connected group of edge pixels. The convex hull
//! wraps a rubber band around an outline, and Douglas–Peucker drops points from an outline until
//! only its main corners remain.

use crate::DetectError;
use crate::edges::EdgeMaps;
use crate::shapes::{in_allowed_area, is_positive, line_intersection, line_through};
use imageproc::contours::{BorderType, find_contours};
use imageproc::geometry::{approximate_polygon_dp, arc_length, contour_area, convex_hull};
use imageproc::point::Point as ImagePoint;
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::BitPlane;

/// The fewest points a contour must have to be traced (A6 step 2).
const MIN_CONTOUR_POINTS: usize = 16;
/// The most contours kept for each edge map (A6 step 3).
const MAX_CONTOURS: usize = 24;
/// The smallest hull area, as a share of the image's area (A6 step 4).
const MIN_AREA_SHARE: f64 = 0.05;
/// The shares of the hull's perimeter used to simplify it (A6 step 4.1).
const SIMPLIFY_SHARES: [f64; 7] = [0.010, 0.015, 0.020, 0.030, 0.040, 0.060, 0.080];
/// The share used for the four longest sides (A6 step 4.2).
const FOUR_SIDES_SHARE: f64 = 0.01;
/// How close two neighbouring lines may be to parallel, in degrees (A6 step 4.2).
const MIN_LINE_ANGLE: f64 = 3.0;

/// Candidate quadrilaterals in work-image pixels, canonical order.
pub fn contour_candidates(maps: &EdgeMaps) -> Result<Vec<Quad>, DetectError> {
    let mut out = Vec::new();
    for map in [&maps.strict, &maps.sensitive] {
        out.extend(candidates_of(map)?);
    }
    Ok(out)
}

/// The candidates of one edge map.
fn candidates_of(map: &BitPlane) -> Result<Vec<Quad>, DetectError> {
    let (w, h) = (map.width(), map.height());
    let image_area = f64::from(w) * f64::from(h);
    // Step 1: close one-pixel gaps.
    let closed = dilate(map);
    let image = gray_image(&closed, w, h)?;
    // Step 2: trace the outer borders.
    let contours = find_contours::<i32>(&image);
    let mut traced: Vec<Vec<ImagePoint<i32>>> = Vec::new();
    for contour in contours {
        if contour.border_type != BorderType::Outer || contour.points.len() < MIN_CONTOUR_POINTS {
            continue;
        }
        traced.push(contour.points);
    }
    // Step 3: keep the 24 with the largest hull areas.
    let mut hulls: Vec<Vec<ImagePoint<i32>>> = Vec::with_capacity(traced.len());
    for points in &traced {
        hulls.push(convex_hull(points.clone()));
    }
    let mut order: Vec<usize> = (0..hulls.len()).collect();
    order.sort_by(|&a, &b| contour_area(&hulls[b]).partial_cmp(&contour_area(&hulls[a])).unwrap_or(std::cmp::Ordering::Equal));
    order.truncate(MAX_CONTOURS);
    let mut out = Vec::new();
    for index in order {
        let hull = &hulls[index];
        // Step 4: skip the small ones.
        if contour_area(hull) < MIN_AREA_SHARE * image_area {
            continue;
        }
        let perimeter = arc_length(hull, true);
        if !is_positive(perimeter) {
            continue;
        }
        // Steps 4.1 and 4.2 each give a candidate of their own, and A9 decides between them: the
        // first takes the hull's own vertices, which a corner cut off by the frame or hidden under
        // a thumb leaves bevelled, and the second extends the four longest sides to recover it.
        if let Some(quad) = four_points(hull, perimeter, w, h) {
            out.push(quad);
        }
        if let Some(quad) = four_sides(hull, perimeter, w, h) {
            out.push(quad);
        }
    }
    Ok(out)
}

/// A hull simplified until four corners are left (A6 step 4.1).
fn four_points(hull: &[ImagePoint<i32>], perimeter: f64, w: u32, h: u32) -> Option<Quad> {
    for share in SIMPLIFY_SHARES {
        let epsilon = share * perimeter;
        if !is_positive(epsilon) {
            return None;
        }
        let simplified = approximate_polygon_dp(hull, epsilon, true);
        if simplified.len() < 4 {
            return None;
        }
        if simplified.len() == 4 {
            let points: Vec<Point> = simplified.iter().map(|p| Point::new(f64::from(p.x) + 0.5, f64::from(p.y) + 0.5)).collect();
            return allowed(points, w, h);
        }
    }
    None
}

/// The four longest sides of a hull, extended into lines and crossed (A6 step 4.2).
///
/// This recovers a corner hidden under a thumb or cut off by the frame, which otherwise appears
/// as a fifth, bevelled vertex.
fn four_sides(hull: &[ImagePoint<i32>], perimeter: f64, w: u32, h: u32) -> Option<Quad> {
    let epsilon = FOUR_SIDES_SHARE * perimeter;
    if !is_positive(epsilon) {
        return None;
    }
    let simplified = approximate_polygon_dp(hull, epsilon, true);
    if simplified.len() < 5 {
        return None;
    }
    // The edges, longest first, but kept in the order they appear in the outline.
    let n = simplified.len();
    let mut edges: Vec<(usize, f64)> = (0..n)
        .map(|i| {
            let a = simplified[i];
            let b = simplified[(i + 1) % n];
            let d = (f64::from(b.x) - f64::from(a.x)).hypot(f64::from(b.y) - f64::from(a.y));
            (i, d)
        })
        .collect();
    edges.sort_by(|a, b| b.1.total_cmp(&a.1));
    edges.truncate(4);
    edges.sort_by_key(|(i, _)| *i);
    let lines: Vec<Option<scancraft_core::geom::Line>> = edges
        .iter()
        .map(|(i, _)| {
            let a = simplified[*i];
            let b = simplified[(i + 1) % n];
            line_through(Point::new(f64::from(a.x) + 0.5, f64::from(a.y) + 0.5), Point::new(f64::from(b.x) + 0.5, f64::from(b.y) + 0.5))
        })
        .collect();
    // No two neighbouring lines may be nearly parallel.
    for k in 0..4 {
        let a = lines.get(k).and_then(|l| *l)?;
        let b = lines.get((k + 1) % 4).and_then(|l| *l)?;
        let angle = crate::shapes::angle_between(a.direction, b.direction);
        if angle < MIN_LINE_ANGLE {
            return None;
        }
    }
    // The four corners: where neighbouring lines cross.
    let mut corners = Vec::with_capacity(4);
    for k in 0..4 {
        let a = lines.get(k).and_then(|l| *l)?;
        let b = lines.get((k + 1) % 4).and_then(|l| *l)?;
        corners.push(line_intersection(&a, &b)?);
    }
    allowed(corners, w, h)
}

/// Puts four points in canonical order and keeps them when they lie in the allowed area.
fn allowed(points: Vec<Point>, w: u32, h: u32) -> Option<Quad> {
    if points.len() != 4 || !points.iter().all(|p| p.is_finite()) {
        return None;
    }
    if !points.iter().all(|p| in_allowed_area(*p, w, h)) {
        return None;
    }
    Some(Quad::canonical([points[0], points[1], points[2], points[3]]))
}

/// Grows every set pixel by one in each direction (A6 step 1).
fn dilate(map: &BitPlane) -> Vec<bool> {
    let (w, h) = (map.width() as i64, map.height() as i64);
    let mut out = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut any = false;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    if map.get(nx as u32, ny as u32) == Some(true) {
                        any = true;
                    }
                }
            }
            out[(y * w + x) as usize] = any;
        }
    }
    out
}

/// The edge map as the greyscale image `imageproc` reads.
fn gray_image(pixels: &[bool], w: u32, h: u32) -> Result<image::GrayImage, DetectError> {
    let data: Vec<u8> = pixels.iter().map(|&on| if on { 255 } else { 0 }).collect();
    image::GrayImage::from_raw(w, h, data).ok_or(CoreError::BadBuffer { len: 0, expected: (w as usize) * (h as usize) }.into())
}

/// The area of a quadrilateral, for the tests.
#[cfg(test)]
fn area_of(quad: &Quad) -> f64 {
    crate::shapes::polygon_area(&quad.0)
}

#[cfg(test)]
mod tests {
    use super::{area_of, contour_candidates, dilate};
    use crate::edges::colour_edges;
    use scancraft_core::image::{BitPlane, Plane, RgbF32};

    fn work(w: u32, h: u32, f: impl Fn(f64, f64) -> [f32; 3]) -> RgbF32 {
        let mut px: Vec<[f32; 3]> = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(f(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
        Plane::from_vec(w, h, px).unwrap()
    }

    /// A white page on a dark desk, as a rectangle.
    fn page_scene(w: u32, h: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> RgbF32 {
        work(w, h, move |x, y| if x > x0 && x < x1 && y > y0 && y < y1 { [0.92; 3] } else { [0.18; 3] })
    }

    #[test]
    fn a_page_is_found_as_a_contour() {
        let img = page_scene(400, 300, 80.0, 60.0, 320.0, 240.0);
        let maps = colour_edges(&img).unwrap();
        let candidates = contour_candidates(&maps).unwrap();
        assert!(!candidates.is_empty(), "no contour candidate was found");
        // The best candidate is the page itself, within a pixel or so of the truth.
        let truth = scancraft_core::geom::Quad([
            scancraft_core::geom::Point::new(80.0, 60.0),
            scancraft_core::geom::Point::new(320.0, 60.0),
            scancraft_core::geom::Point::new(320.0, 240.0),
            scancraft_core::geom::Point::new(80.0, 240.0),
        ]);
        let best = candidates.iter().map(|q| q.0.iter().zip(truth.0.iter()).map(|(a, b)| a.distance(*b)).sum::<f64>()).fold(f64::INFINITY, f64::min);
        assert!(best < 8.0, "the closest candidate is {best:.2} px from the page's corners");
    }

    #[test]
    fn a_dark_square_grows_by_one_pixel() {
        let mut map = BitPlane::new(5, 5).unwrap();
        map.set(2, 2, true);
        let grown = dilate(&map);
        // The middle is set, and its four neighbours.
        assert!(grown[2 * 5 + 2]);
        assert!(grown[2 * 5 + 1] && grown[2 * 5 + 3] && grown[5 + 2] && grown[3 * 5 + 2]);
        // The corners are not.
        assert!(!grown[0]);
        assert!(!grown[4 * 5 + 4]);
    }

    #[test]
    fn an_empty_edge_map_gives_no_candidates() {
        let mut strict = BitPlane::new(40, 40).unwrap();
        let mut sensitive = BitPlane::new(40, 40).unwrap();
        let magnitude = Plane::new(40, 40, 0.0f32).unwrap();
        let direction = Plane::new(40, 40, 0.0f32).unwrap();
        let blurred = Plane::new(40, 40, [0.5f32; 3]).unwrap();
        strict.set(0, 0, false);
        sensitive.set(0, 0, false);
        let maps = crate::edges::EdgeMaps { strict, sensitive, magnitude, direction, blurred };
        assert!(contour_candidates(&maps).unwrap().is_empty());
    }

    #[test]
    fn the_area_of_a_candidate_is_its_own() {
        let q = scancraft_core::geom::Quad([
            scancraft_core::geom::Point::new(0.0, 0.0),
            scancraft_core::geom::Point::new(10.0, 0.0),
            scancraft_core::geom::Point::new(10.0, 10.0),
            scancraft_core::geom::Point::new(0.0, 10.0),
        ]);
        assert!((area_of(&q) - 100.0).abs() < 1e-9);
    }
}
