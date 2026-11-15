//! A7: line candidates (specification A7). Owner: WP8.
//!
//! Every edge pixel votes for all the straight lines that could pass through it. Lines that real
//! edges support collect many votes, even when the edges are broken into pieces. The lines are
//! then grouped by direction, and pairs of lines from the two groups give quadrilaterals.

use crate::DetectError;
use crate::edges::EdgeMaps;
use crate::shapes::{dot, in_allowed_area, line_intersection, normalised, sub};
use imageproc::hough::{LineDetectionOptions, PolarLine, detect_lines};
use scancraft_core::CoreError;
use scancraft_core::geom::{Line, Point, Quad};
use scancraft_core::image::BitPlane;
use std::cmp::Ordering;

/// The most lines kept after ranking them by support (A7 step 2).
const MAX_LINES: usize = 16;
/// The most candidates kept (A7 step 6).
const MAX_CANDIDATES: usize = 1024;
/// How close to parallel two lines of a pair may be, in degrees (A7 step 5).
const MAX_PAIR_ANGLE: f64 = 35.0;
/// How far apart two lines of a pair must lie, as a share of the photo's short side (A7 step 5).
const MIN_PAIR_SEPARATION: f64 = 0.15;
/// How many times the direction clustering runs (A7 step 4).
const CLUSTER_ROUNDS: usize = 10;

/// One line of the page's outline.
#[derive(Debug, Clone)]
struct EdgeLine {
    line: Line,
    /// The direction in degrees, between 0 and 180.
    angle: f64,
    /// The edge pixels within 1 px of the line.
    support: f64,
    /// The image's own borders, which are added by hand.
    border: bool,
}

/// Candidate quadrilaterals in work-image pixels, canonical order, at most 1024.
pub fn line_candidates(maps: &EdgeMaps) -> Result<Vec<Quad>, DetectError> {
    let (w, h) = (maps.sensitive.width(), maps.sensitive.height());
    if w < 3 || h < 3 {
        return Err(CoreError::OutOfRange { name: "work image", value: f64::from(w.min(h)) }.into());
    }
    let pixels = edge_pixels(&maps.sensitive);
    let image = gray_image(&maps.sensitive)?;
    // Step 1: the Hough transform.
    let vote_threshold = 25u32.max((0.08 * f64::from(w.min(h))).round() as u32);
    let options = LineDetectionOptions { vote_threshold, suppression_radius: 6 };
    let found = detect_lines(&image, options);
    // Step 2: rank the lines by the edge pixels near them.
    let mut lines: Vec<EdgeLine> = Vec::with_capacity(found.len());
    for polar in found {
        let line = polar_line(polar);
        let support = support_of(&line, &pixels);
        lines.push(EdgeLine { line, angle: angle_of(&line), support, border: false });
    }
    lines.sort_by(|a, b| b.support.partial_cmp(&a.support).unwrap_or(Ordering::Equal));
    lines.truncate(MAX_LINES);
    // Step 3: the image's own borders.
    lines.extend(borders(w, h));
    // Step 4: two groups of directions.
    let (first, second) = groups(&lines);
    // Step 5: the pairs of lines in each group, and the quadrilaterals they make.
    let mut candidates: Vec<(f64, Quad)> = Vec::new();
    for a in pairs(&first, w, h) {
        for b in pairs(&second, w, h) {
            let Some(quad) = quadrilateral(&a, &b, w, h) else { continue };
            // The borders carry no support of their own: a candidate is ranked by the edges
            // that back it.
            let support = [&a.0, &a.1, &b.0, &b.1].iter().filter(|l| !l.border).map(|l| l.support).sum();
            candidates.push((support, quad));
        }
    }
    // Step 6: keep the best 1024.
    candidates.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(Ordering::Equal));
    candidates.truncate(MAX_CANDIDATES);
    Ok(candidates.into_iter().map(|(_, quad)| quad).collect())
}

/// The line of a Hough result, in the photo's coordinates.
///
/// The accumulator counts the pixels by their indices, while the photo's coordinates put a
/// pixel's centre half a pixel further on, so the distance is corrected by half a pixel.
///
/// The Hough form is `x · cos θ + y · sin θ = r`: the vector (cos θ, sin θ) is the line's
/// normal, so the line itself runs along (−sin θ, cos θ).
fn polar_line(polar: PolarLine) -> Line {
    let angle = f64::from(polar.angle_in_degrees).to_radians();
    let (sin, cos) = angle.sin_cos();
    let r = f64::from(polar.r) + 0.5 * (cos + sin);
    Line { point: Point::new(r * cos, r * sin), direction: Point::new(-sin, cos) }
}

/// The image's four borders, as lines (A7 step 3).
fn borders(w: u32, h: u32) -> [EdgeLine; 4] {
    let (w, h) = (f64::from(w), f64::from(h));
    [
        EdgeLine { line: Line { point: Point::new(0.0, 0.0), direction: Point::new(0.0, 1.0) }, angle: 90.0, support: 0.0, border: true },
        EdgeLine { line: Line { point: Point::new(w, 0.0), direction: Point::new(0.0, 1.0) }, angle: 90.0, support: 0.0, border: true },
        EdgeLine { line: Line { point: Point::new(0.0, 0.0), direction: Point::new(1.0, 0.0) }, angle: 0.0, support: 0.0, border: true },
        EdgeLine { line: Line { point: Point::new(0.0, h), direction: Point::new(1.0, 0.0) }, angle: 0.0, support: 0.0, border: true },
    ]
}

/// The direction of a line in degrees, between 0 and 180.
fn angle_of(line: &Line) -> f64 {
    line.direction.y.atan2(line.direction.x).to_degrees().rem_euclid(180.0)
}

/// The edge pixels of a map, as points whose centres they are.
fn edge_pixels(map: &BitPlane) -> Vec<Point> {
    let mut out = Vec::new();
    for y in 0..map.height() {
        for x in 0..map.width() {
            if map.get(x, y) == Some(true) {
                out.push(Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
    }
    out
}

/// The edge map as the greyscale image `imageproc` reads.
fn gray_image(map: &BitPlane) -> Result<image::GrayImage, DetectError> {
    let (w, h) = (map.width(), map.height());
    let data: Vec<u8> = (0..w * h).map(|i| if map.get(i % w, i / w) == Some(true) { 255 } else { 0 }).collect();
    image::GrayImage::from_raw(w, h, data).ok_or(CoreError::BadBuffer { len: 0, expected: (w as usize) * (h as usize) }.into())
}

/// How many edge pixels lie within 1 px of the line (A7 step 2).
fn support_of(line: &Line, pixels: &[Point]) -> f64 {
    let d = line.direction;
    let mut count = 0.0;
    for p in pixels {
        let v = sub(*p, line.point);
        if (v.x * d.y - v.y * d.x).abs() <= 1.0 {
            count += 1.0;
        }
    }
    count
}

/// The lines of the two direction groups (A7 step 4).
///
/// The clustering is 2-means on the doubled-angle unit vectors, so that 0° and 179° are
/// neighbours. It starts from the best line's direction and its perpendicular.
fn groups(lines: &[EdgeLine]) -> (Vec<EdgeLine>, Vec<EdgeLine>) {
    if lines.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let best = lines.iter().max_by(|a, b| a.support.partial_cmp(&b.support).unwrap_or(Ordering::Equal)).unwrap_or(&lines[0]);
    let (s, c) = best.angle.to_radians().sin_cos();
    let mut first = Point::new(c * c - s * s, 2.0 * s * c);
    let mut second = Point::new(-first.x, -first.y);
    for _ in 0..CLUSTER_ROUNDS {
        let mut sums = [(0.0f64, 0.0f64, 0usize); 2];
        for line in lines {
            let (s, c) = line.angle.to_radians().sin_cos();
            let v = Point::new(c * c - s * s, 2.0 * s * c);
            let d0 = v.x * first.x + v.y * first.y;
            let d1 = v.x * second.x + v.y * second.y;
            let k = usize::from(d1 > d0);
            sums[k].0 += v.x;
            sums[k].1 += v.y;
            sums[k].2 += 1;
        }
        for (k, (sx, sy, n)) in sums.iter().enumerate() {
            if *n > 0 {
                let unit = normalised(Point::new(sx / *n as f64, sy / *n as f64));
                if k == 0 {
                    first = unit;
                } else {
                    second = unit;
                }
            }
        }
    }
    let mut a = Vec::new();
    let mut b = Vec::new();
    for line in lines {
        let (s, c) = line.angle.to_radians().sin_cos();
        let v = Point::new(c * c - s * s, 2.0 * s * c);
        let d0 = v.x * first.x + v.y * first.y;
        let d1 = v.x * second.x + v.y * second.y;
        if d0 >= d1 {
            a.push(line.clone());
        } else {
            b.push(line.clone());
        }
    }
    (a, b)
}

/// The pairs of lines in a group that could be two sides of a page (A7 step 5).
fn pairs(group: &[EdgeLine], w: u32, h: u32) -> Vec<(EdgeLine, EdgeLine)> {
    let mut out = Vec::new();
    for i in 0..group.len() {
        for j in (i + 1)..group.len() {
            let (Some(a), Some(b)) = (group.get(i), group.get(j)) else { continue };
            let difference = (a.angle - b.angle).abs();
            if difference > MAX_PAIR_ANGLE {
                continue;
            }
            if !separated(a, b, w, h) {
                continue;
            }
            out.push((a.clone(), b.clone()));
        }
    }
    out
}

/// Whether two lines lie at least 0.15 · min(w, h) apart where they cross the line through the
/// photo's middle (A7 step 5).
///
/// The distance is measured between the two points where the lines meet the line through the
/// photo's middle that runs across them, so that two nearly parallel lines are measured across
/// rather than along.
fn separated(a: &EdgeLine, b: &EdgeLine, w: u32, h: u32) -> bool {
    let centre = Point::new(f64::from(w) / 2.0, f64::from(h) / 2.0);
    let p = foot_from_centre(&a.line, centre);
    let q = foot_from_centre(&b.line, centre);
    p.distance(q) >= MIN_PAIR_SEPARATION * f64::from(w.min(h))
}

/// Where the line through the photo's middle that runs across `line` meets it: the foot of the
/// perpendicular from the middle.
fn foot_from_centre(line: &Line, centre: Point) -> Point {
    let v = sub(centre, line.point);
    let t = dot(v, line.direction);
    crate::shapes::add(line.point, line.direction.scaled(t))
}

/// The quadrilateral four lines make, in canonical order, when it is convex and in the allowed
/// area (A7 step 5).
fn quadrilateral(a: &(EdgeLine, EdgeLine), b: &(EdgeLine, EdgeLine), w: u32, h: u32) -> Option<Quad> {
    let (a1, a2) = a;
    let (b1, b2) = b;
    let corners = [
        line_intersection(&a1.line, &b1.line),
        line_intersection(&a1.line, &b2.line),
        line_intersection(&a2.line, &b2.line),
        line_intersection(&a2.line, &b1.line),
    ];
    let points: Vec<Point> = corners.into_iter().collect::<Option<Vec<_>>>()?;
    if points.len() != 4 || !points.iter().all(|p| p.is_finite()) {
        return None;
    }
    if !crate::shapes::is_convex(&points) {
        return None;
    }
    if !points.iter().all(|p| in_allowed_area(*p, w, h)) {
        return None;
    }
    Some(Quad::canonical([points[0], points[1], points[2], points[3]]))
}

#[cfg(test)]
mod tests {
    use super::{angle_of, line_candidates, polar_line};
    use crate::edges::colour_edges;
    use scancraft_core::image::{Plane, RgbF32};

    fn work(w: u32, h: u32, f: impl Fn(f64, f64) -> [f32; 3]) -> RgbF32 {
        let mut px: Vec<[f32; 3]> = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(f(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
        Plane::from_vec(w, h, px).unwrap()
    }

    #[test]
    fn a_page_is_found_as_lines() {
        let img = work(400, 300, |x, y| if x > 80.0 && x < 320.0 && y > 60.0 && y < 240.0 { [0.92; 3] } else { [0.18; 3] });
        let maps = colour_edges(&img).unwrap();
        let candidates = line_candidates(&maps).unwrap();
        assert!(!candidates.is_empty(), "no line candidate was found");
        let truth = [
            scancraft_core::geom::Point::new(80.0, 60.0),
            scancraft_core::geom::Point::new(320.0, 60.0),
            scancraft_core::geom::Point::new(320.0, 240.0),
            scancraft_core::geom::Point::new(80.0, 240.0),
        ];
        let best = candidates.iter().map(|q| q.0.iter().zip(truth.iter()).map(|(a, b)| a.distance(*b)).sum::<f64>()).fold(f64::INFINITY, f64::min);
        assert!(best < 12.0, "the closest candidate is {best:.2} px from the page's corners");
    }

    #[test]
    fn a_hough_line_is_read_in_the_photo_s_coordinates() {
        // A vertical line at x = 100: the Hough normal is (1, 0), so the angle is 0°.
        let polar = imageproc::hough::PolarLine { r: 100.0, angle_in_degrees: 0 };
        let line = polar_line(polar);
        // The line runs down the image at x = 100.5: the accumulator counts the pixels by their
        // indices, and a pixel's centre lies half a pixel further on.
        assert!((line.point.x - 100.5).abs() < 1e-6, "{:?}", line.point);
        assert!((angle_of(&line) - 90.0).abs() < 1e-6, "{:?}", line.direction);
    }
}
