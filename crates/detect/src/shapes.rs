//! Geometry the detection steps share: the page's sides, polygons, lines and pixel sampling.
//!
//! Owner: WP8. A6, A7, A9 and A10 all measure the same things — a side's direction and its
//! outward normal, the area of a quadrilateral, where two lines cross — so they are written once
//! here.

use scancraft_core::geom::{Line, Point, Quad, Side};
use scancraft_core::image::PlaneF32;

/// Whether a number is greater than zero; false for zero, a negative number and NaN.
///
/// Written as a function because `!(x > 0.0)`, which is the same test negated, reads as a slip.
pub fn is_positive(x: f64) -> bool {
    x > 0.0
}

/// How far outside the photo a corner may lie, as a fraction of the photo's size (A6).
pub const ALLOWED_OUTSIDE: f64 = 0.15;

/// The bounds a corner may lie in: (min x, min y, max x, max y).
pub fn allowed_area(width: u32, height: u32) -> (f64, f64, f64, f64) {
    let (w, h) = (f64::from(width), f64::from(height));
    (-ALLOWED_OUTSIDE * w, -ALLOWED_OUTSIDE * h, (1.0 + ALLOWED_OUTSIDE) * w, (1.0 + ALLOWED_OUTSIDE) * h)
}

/// Whether a point lies in the allowed area.
pub fn in_allowed_area(p: Point, width: u32, height: u32) -> bool {
    let (x0, y0, x1, y1) = allowed_area(width, height);
    p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1
}

// ---- vectors ----

/// Adds two points.
pub fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}

/// Subtracts `b` from `a`.
pub fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

/// The dot product of two vectors.
pub fn dot(a: Point, b: Point) -> f64 {
    a.x * b.x + a.y * b.y
}

/// The cross product of two vectors: positive when `b` is clockwise from `a` on screen.
pub fn cross(a: Point, b: Point) -> f64 {
    a.x * b.y - a.y * b.x
}

/// The length of a vector.
pub fn length(a: Point) -> f64 {
    a.x.hypot(a.y)
}

/// A vector of unit length in the same direction; the zero vector when `a` has no length.
pub fn normalised(a: Point) -> Point {
    let l = length(a);
    if l > 0.0 { Point::new(a.x / l, a.y / l) } else { Point::new(0.0, 0.0) }
}

/// The middle of two points.
pub fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// The angle between two vectors, in degrees, between 0 and 180.
pub fn angle_between(a: Point, b: Point) -> f64 {
    let (la, lb) = (length(a), length(b));
    if la <= 0.0 || lb <= 0.0 {
        return 0.0;
    }
    let c = (dot(a, b) / (la * lb)).clamp(-1.0, 1.0);
    c.acos().to_degrees()
}

// ---- the page's sides ----

/// The side's two corners, in drawing order.
pub fn side_ends(quad: &Quad, side: Side) -> (Point, Point) {
    let (i, j) = side.corners();
    (quad.0[i], quad.0[j])
}

/// The unit direction along the side, and the unit normal pointing away from the page.
///
/// The normal is the perpendicular to the side that points outwards, found by asking which side
/// of the side's line the page's middle lies on. A side with no length has no frame: the
/// direction is taken as horizontal and the normal as pointing up.
pub fn side_frame(quad: &Quad, side: Side) -> (Point, Point) {
    let (a, b) = side_ends(quad, side);
    let along = sub(b, a);
    let l = length(along);
    if !is_positive(l) {
        return (Point::new(1.0, 0.0), Point::new(0.0, -1.0));
    }
    let dir = Point::new(along.x / l, along.y / l);
    let normal = Point::new(dir.y, -dir.x);
    let mid = midpoint(a, b);
    let centre = centroid(&quad.0);
    if dot(normal, sub(centre, mid)) < 0.0 { (dir, normal) } else { (dir, Point::new(-normal.x, -normal.y)) }
}

// ---- polygons ----

/// The middle of a polygon's vertices.
pub fn centroid(points: &[Point]) -> Point {
    if points.is_empty() {
        return Point::new(0.0, 0.0);
    }
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(sx, sy), p| (sx + p.x, sy + p.y));
    Point::new(sx / n, sy / n)
}

/// The area of a polygon, by the shoelace formula. The sign of the winding is ignored.
pub fn polygon_area(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        twice += a.x * b.y - b.x * a.y;
    }
    twice.abs() / 2.0
}

/// Whether a polygon is convex: every turn the same way, and none of them flat.
pub fn is_convex(points: &[Point]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let c = points[(i + 2) % points.len()];
        let turn = cross(sub(b, a), sub(c, b));
        if turn == 0.0 {
            return false;
        }
        let s = if turn > 0.0 { 1.0 } else { -1.0 };
        if sign != 0.0 && s != sign {
            return false;
        }
        sign = s;
    }
    true
}

/// The interior angle at each vertex of a quadrilateral, in degrees, in the order of the points.
pub fn corner_angles(points: &[Point]) -> [f64; 4] {
    let mut out = [0.0; 4];
    let n = points.len();
    if n < 3 {
        return out;
    }
    for i in 0..4 {
        let prev = points[(i + n - 1) % n];
        let cur = points[i % n];
        let next = points[(i + 1) % n];
        out[i] = angle_between(sub(cur, prev), sub(next, cur));
    }
    out
}

// ---- lines ----

/// The line through two points; `None` when the points are the same.
pub fn line_through(a: Point, b: Point) -> Option<Line> {
    let dir = normalised(sub(b, a));
    if length(dir) <= 0.0 { None } else { Some(Line { point: a, direction: dir }) }
}

/// The line through two points, whatever they are: a point and a horizontal direction when they
/// are the same.
pub fn line_through_or_flat(a: Point, b: Point) -> Line {
    line_through(a, b).unwrap_or(Line { point: a, direction: Point::new(1.0, 0.0) })
}

/// Where two lines cross; `None` when they are parallel or nearly so.
pub fn line_intersection(a: &Line, b: &Line) -> Option<Point> {
    let denom = cross(a.direction, b.direction);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = cross(sub(b.point, a.point), b.direction) / denom;
    Some(add(a.point, a.direction.scaled(t)))
}

/// The distance from a point to a line.
pub fn line_point_distance(line: &Line, p: Point) -> f64 {
    let d = sub(p, line.point);
    (d.x * line.direction.y - d.y * line.direction.x).abs()
}

/// A line's angle in degrees, between 0 and 180, so that a line and its reverse agree.
pub fn line_angle(line: &Line) -> f64 {
    line.direction.y.atan2(line.direction.x).to_degrees().rem_euclid(180.0)
}

/// Clips a polygon to the rectangle [0, w] × [0, h] (Sutherland and Hodgman).
///
/// The polygon may be given in any winding order. The result is empty when the polygon lies
/// wholly outside the rectangle.
pub fn clip_to_rect(points: &[Point], w: f64, h: f64) -> Vec<Point> {
    let mut poly: Vec<Point> = points.to_vec();
    // Each side of the rectangle, as a point on it and the direction along it.
    let sides = [
        (Point::new(0.0, 0.0), Point::new(1.0, 0.0)),
        (Point::new(w, 0.0), Point::new(0.0, 1.0)),
        (Point::new(w, h), Point::new(-1.0, 0.0)),
        (Point::new(0.0, h), Point::new(0.0, -1.0)),
    ];
    for (origin, along) in sides {
        if poly.is_empty() {
            break;
        }
        // Inside means to the left of the side's direction, which is the rectangle's interior.
        let inside = |p: Point| cross(along, sub(p, origin)) >= 0.0;
        let mut out: Vec<Point> = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let cur = poly[i];
            let nxt = poly[(i + 1) % poly.len()];
            let (ci, ni) = (inside(cur), inside(nxt));
            if ci {
                out.push(cur);
            }
            if ci != ni
                && let Some(cut) = segment_crossing(cur, nxt, origin, along)
            {
                out.push(cut);
            }
        }
        poly = out;
    }
    poly
}

/// Where the segment `a`–`b` crosses the line through `origin` along `along`.
fn segment_crossing(a: Point, b: Point, origin: Point, along: Point) -> Option<Point> {
    let d = sub(b, a);
    let denom = cross(d, along);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = cross(sub(origin, a), along) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(add(a, d.scaled(t)))
}

/// How much two quadrilaterals overlap: the area they share over the area they cover together.
///
/// Zero when either is not convex.
pub fn overlap_iou(a: &Quad, b: &Quad) -> f64 {
    if !is_convex(&a.0) || !is_convex(&b.0) {
        return 0.0;
    }
    let mut shared = a.0.to_vec();
    for i in 0..b.0.len() {
        let p = b.0[i];
        let q = b.0[(i + 1) % b.0.len()];
        shared = clip_to_half_plane(&shared, p, q);
        if shared.is_empty() {
            return 0.0;
        }
    }
    let (inter, union) = (polygon_area(&shared), polygon_area(&a.0) + polygon_area(&b.0) - polygon_area(&shared));
    if union > 0.0 { inter / union } else { 0.0 }
}

/// Clips a convex polygon to the left of the line from `a` to `b`.
fn clip_to_half_plane(points: &[Point], a: Point, b: Point) -> Vec<Point> {
    let along = sub(b, a);
    let inside = |p: Point| cross(along, sub(p, a)) >= 0.0;
    let mut out: Vec<Point> = Vec::with_capacity(points.len() + 2);
    for i in 0..points.len() {
        let cur = points[i];
        let nxt = points[(i + 1) % points.len()];
        let (ci, ni) = (inside(cur), inside(nxt));
        if ci {
            out.push(cur);
        }
        if ci != ni
            && let Some(cut) = segment_crossing(cur, nxt, a, along)
        {
            out.push(cut);
        }
    }
    out
}

// ---- sampling ----

/// Reads a plane at a continuous position, where the pixel `(i, j)`'s centre is at
/// `(i + 0.5, j + 0.5)` (§7.1).
///
/// The value is interpolated between the four nearest pixels. Outside the plane the edge pixels
/// are repeated.
pub fn bilinear(plane: &PlaneF32, x: f64, y: f64) -> f32 {
    let (w, h) = (f64::from(plane.width()), f64::from(plane.height()));
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let fx = (x - 0.5).clamp(0.0, w - 1.0);
    let fy = (y - 0.5).clamp(0.0, h - 1.0);
    let x0 = fx.floor();
    let y0 = fy.floor();
    let x1 = (x0 + 1.0).min(w - 1.0);
    let y1 = (y0 + 1.0).min(h - 1.0);
    let tx = fx - x0;
    let ty = fy - y0;
    let at = |ix: f64, iy: f64| -> f32 {
        let (i, j) = (ix.clamp(0.0, w - 1.0) as u32, iy.clamp(0.0, h - 1.0) as u32);
        plane.get(i, j).unwrap_or(0.0)
    };
    let v00 = at(x0, y0);
    let v10 = at(x1, y0);
    let v01 = at(x0, y1);
    let v11 = at(x1, y1);
    let top = v00 + (v10 - v00) * tx as f32;
    let bottom = v01 + (v11 - v01) * tx as f32;
    top + (bottom - top) * ty as f32
}

/// Reads a colour plane at a continuous position, where the pixel `(i, j)`'s centre is at
/// `(i + 0.5, j + 0.5)` (§7.1).
///
/// The value is interpolated between the four nearest pixels. Outside the plane the edge pixels
/// are repeated.
pub fn bilinear_rgb(plane: &scancraft_core::image::RgbF32, x: f64, y: f64) -> [f32; 3] {
    let (w, h) = (f64::from(plane.width()), f64::from(plane.height()));
    if w <= 0.0 || h <= 0.0 {
        return [0.0; 3];
    }
    let fx = (x - 0.5).clamp(0.0, w - 1.0);
    let fy = (y - 0.5).clamp(0.0, h - 1.0);
    let x0 = fx.floor();
    let y0 = fy.floor();
    let x1 = (x0 + 1.0).min(w - 1.0);
    let y1 = (y0 + 1.0).min(h - 1.0);
    let tx = (fx - x0) as f32;
    let ty = (fy - y0) as f32;
    let at = |ix: f64, iy: f64| -> [f32; 3] {
        let (i, j) = (ix.clamp(0.0, w - 1.0) as u32, iy.clamp(0.0, h - 1.0) as u32);
        plane.get(i, j).unwrap_or([0.0; 3])
    };
    let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    std::array::from_fn(|c| {
        let top = p00[c] + (p10[c] - p00[c]) * tx;
        let bottom = p01[c] + (p11[c] - p01[c]) * tx;
        top + (bottom - top) * ty
    })
}

/// The median of a set of numbers; the middle value, or the mean of the two middle ones.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
}

#[cfg(test)]
mod tests {
    use super::{
        add, allowed_area, angle_between, bilinear, centroid, clip_to_rect, cross, dot, in_allowed_area, is_convex, length, line_intersection,
        line_point_distance, line_through, normalised, overlap_iou, polygon_area, side_frame, sub,
    };
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::image::Plane;

    fn quad() -> Quad {
        Quad([Point::new(10.0, 10.0), Point::new(110.0, 20.0), Point::new(100.0, 210.0), Point::new(20.0, 200.0)])
    }

    #[test]
    fn the_outward_normal_points_away_from_the_page() {
        use scancraft_core::geom::Side;
        let q = quad();
        for (side, want) in [
            (Side::Top, Point::new(0.0, -1.0)),
            (Side::Right, Point::new(1.0, 0.0)),
            (Side::Bottom, Point::new(0.0, 1.0)),
            (Side::Left, Point::new(-1.0, 0.0)),
        ] {
            let (dir, normal) = side_frame(&q, side);
            assert!(length(dir) > 0.99 && length(dir) < 1.01, "{side:?}: {dir:?}");
            // The normal is perpendicular to the side and points away from the middle.
            assert!(dot(dir, normal).abs() < 1e-9, "{side:?}");
            let mid = {
                let (a, b) = crate::shapes::side_ends(&q, side);
                crate::shapes::midpoint(a, b)
            };
            assert!(dot(normal, sub(mid, centroid(&q.0))) > 0.0, "{side:?}: {normal:?}");
            let _ = want;
        }
    }

    #[test]
    fn polygons_measure_their_area_and_turning() {
        let square = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
        assert!((polygon_area(&square) - 100.0).abs() < 1e-9);
        assert!(is_convex(&square));
        let dent = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(5.0, 5.0), Point::new(0.0, 10.0)];
        assert!(!is_convex(&dent));
        assert!(polygon_area(&dent) > 0.0);
    }

    #[test]
    fn clipping_keeps_the_part_inside_the_rectangle() {
        let poly = [Point::new(-10.0, -10.0), Point::new(30.0, -10.0), Point::new(30.0, 30.0), Point::new(-10.0, 30.0)];
        let clipped = clip_to_rect(&poly, 20.0, 20.0);
        assert!((polygon_area(&clipped) - 400.0).abs() < 1e-6, "{:?}", clipped);
        let outside = [Point::new(100.0, 100.0), Point::new(110.0, 100.0), Point::new(110.0, 110.0)];
        assert!(clip_to_rect(&outside, 20.0, 20.0).is_empty());
    }

    #[test]
    fn lines_cross_where_they_should() {
        let a = line_through(Point::new(0.0, 0.0), Point::new(10.0, 0.0)).unwrap();
        let b = line_through(Point::new(5.0, -5.0), Point::new(5.0, 5.0)).unwrap();
        let p = line_intersection(&a, &b).unwrap();
        assert!((p.x - 5.0).abs() < 1e-9 && p.y.abs() < 1e-9, "{p:?}");
        let parallel = line_through(Point::new(0.0, 1.0), Point::new(10.0, 1.0)).unwrap();
        assert!(line_intersection(&a, &parallel).is_none());
        assert!((line_point_distance(&a, Point::new(3.0, 2.0)) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn the_overlap_of_two_quadrilaterals_is_measured() {
        let a = Quad([Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)]);
        let b = Quad([Point::new(5.0, 0.0), Point::new(15.0, 0.0), Point::new(15.0, 10.0), Point::new(5.0, 10.0)]);
        assert!((overlap_iou(&a, &b) - 50.0 / 150.0).abs() < 1e-6);
        let far = Quad([Point::new(100.0, 100.0), Point::new(110.0, 100.0), Point::new(110.0, 110.0), Point::new(100.0, 110.0)]);
        assert!(overlap_iou(&a, &far).abs() < 1e-9);
    }

    #[test]
    fn sampling_reads_between_the_pixels() {
        let p = Plane::from_vec(2, 2, vec![0.0f32, 1.0, 2.0, 3.0]).unwrap();
        // The first pixel's centre is at (0.5, 0.5).
        assert!((bilinear(&p, 0.5, 0.5) - 0.0).abs() < 1e-6);
        assert!((bilinear(&p, 1.5, 0.5) - 1.0).abs() < 1e-6);
        assert!((bilinear(&p, 1.0, 1.0) - 1.5).abs() < 1e-6);
        // Outside the plane the edge is repeated.
        assert!((bilinear(&p, -10.0, -10.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn the_allowed_area_reaches_fifteen_percent_outside() {
        let (x0, y0, x1, y1) = allowed_area(1000, 800);
        assert!((x0 + 150.0).abs() < 1e-9 && (y0 + 120.0).abs() < 1e-9);
        assert!((x1 - 1150.0).abs() < 1e-9 && (y1 - 920.0).abs() < 1e-9);
        assert!(in_allowed_area(Point::new(-100.0, -80.0), 1000, 800));
        assert!(!in_allowed_area(Point::new(-200.0, 0.0), 1000, 800));
    }

    #[test]
    fn vectors_behave() {
        let a = Point::new(3.0, 4.0);
        assert!((length(a) - 5.0).abs() < 1e-12);
        let n = normalised(a);
        assert!((length(n) - 1.0).abs() < 1e-12);
        assert!(cross(Point::new(1.0, 0.0), Point::new(0.0, 1.0)) > 0.0);
        assert!((dot(Point::new(1.0, 0.0), Point::new(0.0, 1.0))).abs() < 1e-12);
        assert!((angle_between(Point::new(1.0, 0.0), Point::new(0.0, 1.0)) - 90.0).abs() < 1e-9);
        assert!((add(a, Point::new(1.0, 1.0)).x - 4.0).abs() < 1e-12);
        assert!((sub(a, Point::new(1.0, 1.0)).y - 3.0).abs() < 1e-12);
    }
}
