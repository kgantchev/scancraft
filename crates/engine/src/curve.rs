//! The bent side's curve (specification §4.3): a centripetal Catmull–Rom spline (α = 0.5) through
//! the side's two corners and its three bend handles. A27 takes this curve as the side's evidence
//! and the interface draws it, so both use the same points.

use scancraft_core::geom::{Point, Quad, Side};

/// The samples drawn per span between control points.
const PER_SPAN: usize = 32;

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// The centripetal parameter step between two points: |b − a|^α with α = ½.
fn knot_step(a: Point, b: Point) -> f64 {
    a.distance(b).sqrt().max(1e-9)
}

/// One span of the spline between `p1` and `p2`, `p0` and `p3` being their neighbours
/// (Barry and Goldman's pyramid), at `steps + 1` points from `p1` to `p2` inclusive.
fn span(p0: Point, p1: Point, p2: Point, p3: Point, steps: usize) -> Vec<Point> {
    let (t0, t1) = (0.0, knot_step(p0, p1));
    let t2 = t1 + knot_step(p1, p2);
    let t3 = t2 + knot_step(p2, p3);
    (0..=steps)
        .map(|k| {
            let t = t1 + (t2 - t1) * k as f64 / steps as f64;
            let a1 = lerp(p0, p1, (t - t0) / (t1 - t0));
            let a2 = lerp(p1, p2, (t - t1) / (t2 - t1));
            let a3 = lerp(p2, p3, (t - t2) / (t3 - t2));
            let b1 = lerp(a1, a2, (t - t0) / (t2 - t0));
            let b2 = lerp(a2, a3, (t - t1) / (t3 - t1));
            lerp(b1, b2, (t - t1) / (t2 - t1))
        })
        .collect()
}

/// The spline through `points` (at least two), as a dense polyline of `PER_SPAN` points per span.
fn dense(points: &[Point]) -> Vec<Point> {
    if points.len() < 2 {
        return points.to_vec();
    }
    // Phantom end points, so the end spans have neighbours: the reflection of the next point.
    let first = lerp(points[1], points[0], 2.0);
    let n = points.len();
    let last = lerp(points[n - 2], points[n - 1], 2.0);
    let mut padded = vec![first];
    padded.extend_from_slice(points);
    padded.push(last);
    let mut out: Vec<Point> = Vec::with_capacity((n - 1) * PER_SPAN + 1);
    for i in 1..n {
        let piece = span(padded[i - 1], padded[i], padded[i + 1], padded[i + 2], PER_SPAN);
        out.extend(if i == 1 { &piece[..] } else { &piece[1..] });
    }
    out
}

/// The five control points of a side: its first corner, the three handles and its second corner.
fn controls(corners: &Quad, side: Side, handles: &[Point; 3]) -> [Point; 5] {
    let (a, b) = side.corners();
    [corners.0[a], handles[0], handles[1], handles[2], corners.0[b]]
}

/// The curve as a polyline for drawing, and for the check that it doesn't cross another side.
pub fn bent_side_points(corners: &Quad, side: Side, handles: &[Point; 3]) -> Vec<Point> {
    let c = controls(corners, side, handles);
    if !c.iter().all(|p| p.is_finite()) {
        return Vec::new();
    }
    dense(&c)
}

/// `n` points spread evenly by arc length along the curve, from the first corner to the second.
pub fn bent_side_samples(corners: &Quad, side: Side, handles: &[Point; 3], n: usize) -> Vec<Point> {
    let line = bent_side_points(corners, side, handles);
    if line.len() < 2 || n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![line[0]];
    }
    let mut cumulative = vec![0.0];
    for w in line.windows(2) {
        cumulative.push(cumulative[cumulative.len() - 1] + w[0].distance(w[1]));
    }
    let total = cumulative[cumulative.len() - 1];
    if total <= 0.0 {
        return vec![line[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 0;
    for k in 0..n {
        let target = total * k as f64 / (n - 1) as f64;
        while seg + 2 < cumulative.len() && cumulative[seg + 1] < target {
            seg += 1;
        }
        let (c0, c1) = (cumulative[seg], cumulative[seg + 1]);
        let t = if c1 > c0 { ((target - c0) / (c1 - c0)).clamp(0.0, 1.0) } else { 0.0 };
        out.push(lerp(line[seg], line[seg + 1], t));
    }
    out
}

/// The default bend handles of a straight side: at a quarter, half and three quarters of its length.
pub fn default_handles(corners: &Quad, side: Side) -> [Point; 3] {
    let (a, b) = side.corners();
    let (pa, pb) = (corners.0[a], corners.0[b]);
    [0.25, 0.5, 0.75].map(|t| lerp(pa, pb, t))
}

/// Whether two segments properly cross.
fn segments_cross(a: (Point, Point), b: (Point, Point)) -> bool {
    let cross = |o: Point, p: Point, q: Point| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    let (d1, d2) = (cross(b.0, b.1, a.0), cross(b.0, b.1, a.1));
    let (d3, d4) = (cross(a.0, a.1, b.0), cross(a.0, a.1, b.1));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Whether the polyline crosses the segment `other` anywhere but at their shared ends.
pub fn polyline_crosses(line: &[Point], other: (Point, Point)) -> bool {
    line.windows(2).any(|w| segments_cross((w[0], w[1]), other))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Quad {
        Quad([Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)])
    }

    fn bulge() -> [Point; 3] {
        [Point::new(25.0, -8.0), Point::new(50.0, -14.0), Point::new(75.0, -8.0)]
    }

    #[test]
    fn the_bent_side_curve_passes_through_its_five_points() {
        let line = bent_side_points(&square(), Side::Top, &bulge());
        let near = |p: Point| line.iter().any(|q| q.distance(p) < 1e-9);
        assert!(near(Point::new(0.0, 0.0)) && near(Point::new(100.0, 0.0)));
        for h in bulge() {
            assert!(near(h), "{h:?}");
        }
        // It runs from the first corner to the second.
        assert_eq!(line[0], Point::new(0.0, 0.0));
        assert_eq!(*line.last().unwrap(), Point::new(100.0, 0.0));
    }

    #[test]
    fn the_bent_side_samples_are_the_ones_the_sheet_fit_takes() {
        let samples = bent_side_samples(&square(), Side::Top, &bulge(), 64);
        assert_eq!(samples.len(), 64);
        assert_eq!(samples[0], Point::new(0.0, 0.0));
        assert!(samples[63].distance(Point::new(100.0, 0.0)) < 1e-9);
        // Spread evenly by arc length: the steps agree within a few per cent.
        let steps: Vec<f64> = samples.windows(2).map(|w| w[0].distance(w[1])).collect();
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        assert!(steps.iter().all(|s| (s - mean).abs() < 0.05 * mean), "{steps:?}");
        // The bulge reaches above the chord.
        assert!(samples.iter().any(|p| p.y < -10.0));
    }

    #[test]
    fn a_side_with_handles_on_its_chord_is_straight() {
        let c = square();
        let samples = bent_side_samples(&c, Side::Right, &default_handles(&c, Side::Right), 33);
        assert!(samples.iter().all(|p| (p.x - 100.0).abs() < 1e-9));
        // The bottom side runs from the bottom-left corner to the bottom-right (drawing order).
        let bottom = bent_side_samples(&c, Side::Bottom, &default_handles(&c, Side::Bottom), 5);
        assert_eq!((bottom[0], bottom[4]), (Point::new(0.0, 100.0), Point::new(100.0, 100.0)));
    }

    #[test]
    fn junk_points_give_no_curve_and_crossing_is_detected() {
        let nan = [Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(2.0, 2.0)];
        assert!(bent_side_points(&square(), Side::Top, &nan).is_empty());
        assert!(bent_side_samples(&square(), Side::Top, &nan, 10).is_empty());
        assert!(bent_side_samples(&square(), Side::Top, &bulge(), 0).is_empty());
        // Same point three times (a zero-length spline) doesn't panic.
        let same = [Point::new(5.0, 5.0); 3];
        let q = Quad([Point::new(5.0, 5.0); 4]);
        assert_eq!(bent_side_samples(&q, Side::Top, &same, 4).len(), 4);
        // A top side dragged down through the bottom side crosses it.
        let deep = [Point::new(25.0, 80.0), Point::new(50.0, 130.0), Point::new(75.0, 80.0)];
        let line = bent_side_points(&square(), Side::Top, &deep);
        assert!(polyline_crosses(&line, (Point::new(0.0, 100.0), Point::new(100.0, 100.0))));
        assert!(!polyline_crosses(&bent_side_points(&square(), Side::Top, &bulge()), (Point::new(0.0, 100.0), Point::new(100.0, 100.0))));
    }
}
