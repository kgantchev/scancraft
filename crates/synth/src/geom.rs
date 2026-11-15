//! The small geometry the generator and the metrics need, written here so that `scancraft-synth`
//! shares no code with the pipeline it checks (it may depend on `scancraft-core` only).

use scancraft_core::geom::{Point, Quad};

pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

pub fn mul_vec(a: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2])
}

/// Rotation about the z axis (the camera's optical axis), counter-clockwise by `angle`.
pub fn rotation_z(angle: f64) -> Mat3 {
    let (c, s) = (angle.cos(), angle.sin());
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Rotation about the unit vector `axis` by `angle` (Rodrigues' formula).
pub fn rotation_about(axis: [f64; 3], angle: f64) -> Mat3 {
    let (c, s) = (angle.cos(), angle.sin());
    let [x, y, z] = axis;
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

/// The inverse of a 3 × 3 matrix, or `None` when it is singular.
pub fn invert3(m: &Mat3) -> Option<Mat3> {
    let c = [
        [m[1][1] * m[2][2] - m[1][2] * m[2][1], m[0][2] * m[2][1] - m[0][1] * m[2][2], m[0][1] * m[1][2] - m[0][2] * m[1][1]],
        [m[1][2] * m[2][0] - m[1][0] * m[2][2], m[0][0] * m[2][2] - m[0][2] * m[2][0], m[0][2] * m[1][0] - m[0][0] * m[1][2]],
        [m[1][0] * m[2][1] - m[1][1] * m[2][0], m[0][1] * m[2][0] - m[0][0] * m[2][1], m[0][0] * m[1][1] - m[0][1] * m[1][0]],
    ];
    let det = m[0][0] * c[0][0] + m[0][1] * c[1][0] + m[0][2] * c[2][0];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inv = c.map(|row| row.map(|v| v / det));
    inv.iter().flatten().all(|v| v.is_finite()).then_some(inv)
}

/// Applies a 3 × 3 matrix to a 2-D point, dividing by the third component.
pub fn apply_h(m: &Mat3, x: f64, y: f64) -> Option<Point> {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if w == 0.0 || !w.is_finite() {
        return None;
    }
    let p = Point::new((m[0][0] * x + m[0][1] * y + m[0][2]) / w, (m[1][0] * x + m[1][1] * y + m[1][2]) / w);
    p.is_finite().then_some(p)
}

/// The homography that maps the unit square onto `q`, with (0, 0) → TL, (1, 0) → TR, (1, 1) → BR
/// and (0, 1) → BL (Heckbert 1989, §2.2.3). `long_side` scales the test for a parallelogram.
pub fn homography_from_unit_square(q: &Quad, long_side: f64) -> Option<Mat3> {
    if !q.is_finite() || !long_side.is_finite() || long_side <= 0.0 {
        return None;
    }
    let [p0, p1, p2, p3] = q.0;
    let (sx, sy) = (p0.x - p1.x + p2.x - p3.x, p0.y - p1.y + p2.y - p3.y);
    if sx.abs() < 1e-9 * long_side && sy.abs() < 1e-9 * long_side {
        return Some([[p1.x - p0.x, p2.x - p1.x, p0.x], [p1.y - p0.y, p2.y - p1.y, p0.y], [0.0, 0.0, 1.0]]);
    }
    let (dx1, dx2, dy1, dy2) = (p1.x - p2.x, p3.x - p2.x, p1.y - p2.y, p3.y - p2.y);
    let den = dx1 * dy2 - dx2 * dy1;
    if den == 0.0 || !den.is_finite() {
        return None;
    }
    let g = (sx * dy2 - dx2 * sy) / den;
    let h = (dx1 * sy - sx * dy1) / den;
    let m = [[p1.x - p0.x + g * p1.x, p3.x - p0.x + h * p3.x, p0.x], [p1.y - p0.y + g * p1.y, p3.y - p0.y + h * p3.y, p0.y], [g, h, 1.0]];
    m.iter().flatten().all(|v| v.is_finite()).then_some(m)
}

/// The area of a polygon, by the shoelace formula (always non-negative).
pub fn polygon_area(p: &[Point]) -> f64 {
    if p.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum.abs() / 2.0
}

/// The signed area, positive when the points run counter-clockwise (y up).
fn signed_area(p: &[Point]) -> f64 {
    let mut sum = 0.0;
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum / 2.0
}

/// `subject` clipped to the half-plane left of the directed edge `a → b` (Sutherland–Hodgman).
fn clip_edge(subject: &[Point], a: Point, b: Point) -> Vec<Point> {
    let side = |p: Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let mut out = Vec::with_capacity(subject.len() + 2);
    for (i, &p) in subject.iter().enumerate() {
        let q = subject[(i + 1) % subject.len()];
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            let t = sp / (sp - sq);
            out.push(Point::new(p.x + t * (q.x - p.x), p.y + t * (q.y - p.y)));
        }
    }
    out
}

/// The area of the intersection of two polygons, `clip` being convex.
pub fn polygon_intersection_area(subject: &[Point], clip: &[Point]) -> f64 {
    if subject.len() < 3 || clip.len() < 3 {
        return 0.0;
    }
    // The clip polygon's edges must run counter-clockwise for "left of the edge" to mean inside.
    let mut clip: Vec<Point> = clip.to_vec();
    if signed_area(&clip) < 0.0 {
        clip.reverse();
    }
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        if out.len() < 3 {
            return 0.0;
        }
        out = clip_edge(&out, clip[i], clip[(i + 1) % clip.len()]);
    }
    polygon_area(&out)
}

/// Whether the quadrilateral is convex (all turns the same way).
pub fn is_convex(q: &Quad) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let (a, b, c) = (q.0[i], q.0[(i + 1) % 4], q.0[(i + 2) % 4]);
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign.signum() {
                return false;
            }
            sign = cross;
        }
    }
    sign != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(p: [(f64, f64); 4]) -> Quad {
        Quad(p.map(|(x, y)| Point::new(x, y)))
    }

    #[test]
    fn the_unit_square_maps_onto_the_quad() {
        let q = quad([(412.0, 380.0), (3610.2, 455.7), (3301.9, 2840.4), (611.3, 2702.8)]);
        let h = homography_from_unit_square(&q, 4032.0).unwrap();
        for ((u, v), want) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().zip(q.0) {
            assert!(apply_h(&h, u, v).unwrap().distance(want) < 1e-9);
        }
        let back = apply_h(&invert3(&h).unwrap(), q.0[2].x, q.0[2].y).unwrap();
        assert!((back.x - 1.0).abs() < 1e-9 && (back.y - 1.0).abs() < 1e-9);
        // A parallelogram takes the affine branch.
        let p = quad([(0.0, 0.0), (2.0, 0.0), (3.0, 1.0), (1.0, 1.0)]);
        let h = homography_from_unit_square(&p, 100.0).unwrap();
        assert_eq!(h[2], [0.0, 0.0, 1.0]);
        assert!(homography_from_unit_square(&quad([(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]), 10.0).is_none());
    }

    #[test]
    fn areas_and_intersections() {
        let a = quad([(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]);
        let b = quad([(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]);
        assert!((polygon_area(&a.0) - 4.0).abs() < 1e-12);
        assert!((polygon_intersection_area(&a.0, &b.0) - 1.0).abs() < 1e-12);
        // Either winding order gives the same answer.
        let mut reversed = b.0;
        reversed.reverse();
        assert!((polygon_intersection_area(&a.0, &reversed) - 1.0).abs() < 1e-12);
        assert_eq!(polygon_intersection_area(&a.0, &quad([(5.0, 5.0), (6.0, 5.0), (6.0, 6.0), (5.0, 6.0)]).0), 0.0);
        assert!(is_convex(&a));
        assert!(!is_convex(&quad([(0.0, 0.0), (4.0, 0.0), (1.0, 1.0), (0.0, 4.0)])));
    }

    #[test]
    fn rotations_are_rotations() {
        let r = rotation_about([0.0, 0.6, 0.8], 0.7);
        let rt = [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]];
        let id = mul(&r, &rt);
        for (i, row) in id.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - IDENTITY[i][j]).abs() < 1e-12);
            }
        }
        assert!(invert3(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [1.0, 0.0, 1.0]]).is_none());
    }
}
