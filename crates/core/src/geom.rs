//! Geometry primitives (specification §7.1–§7.2): photo pixels with x to the right and y down,
//! pixel (i, j) centred at (i + ½, j + ½).

use serde::{Deserialize, Serialize};

/// A point in pixels, in f64.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn distance(self, other: Point) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn scaled(self, k: f64) -> Point {
        Point::new(self.x * k, self.y * k)
    }
}

/// The four sides of a page outline, in the order the recipe stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

    /// Index into `[top, right, bottom, left]` arrays.
    pub const fn index(self) -> usize {
        match self {
            Side::Top => 0,
            Side::Right => 1,
            Side::Bottom => 2,
            Side::Left => 3,
        }
    }

    /// The side's two corners as indices into a [`Quad`]'s TL, TR, BR, BL order, in drawing order.
    pub const fn corners(self) -> (usize, usize) {
        match self {
            Side::Top => (0, 1),
            Side::Right => (1, 2),
            Side::Bottom => (3, 2),
            Side::Left => (0, 3),
        }
    }
}

/// A quadrilateral in the canonical order TL, TR, BR, BL (clockwise on screen).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Quad(pub [Point; 4]);

impl Quad {
    pub fn tl(&self) -> Point {
        self.0[0]
    }
    pub fn tr(&self) -> Point {
        self.0[1]
    }
    pub fn br(&self) -> Point {
        self.0[2]
    }
    pub fn bl(&self) -> Point {
        self.0[3]
    }

    pub fn is_finite(&self) -> bool {
        self.0.iter().all(|p| p.is_finite())
    }

    /// Puts any four points in canonical order (specification §7.2): sorted by
    /// `atan2(y − cy, x − cx)` around their centroid, ascending, then rotated to start at the
    /// point with the smallest x + y. Ties in angle keep input order.
    pub fn canonical(points: [Point; 4]) -> Quad {
        let cx = points.iter().map(|p| p.x).sum::<f64>() / 4.0;
        let cy = points.iter().map(|p| p.y).sum::<f64>() / 4.0;
        let mut order = [0usize, 1, 2, 3];
        let angle = |i: usize| -> f64 {
            let p = points.get(i).copied().unwrap_or_default();
            (p.y - cy).atan2(p.x - cx)
        };
        order.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)));
        let sum = |i: usize| -> f64 {
            let p = points.get(i).copied().unwrap_or_default();
            p.x + p.y
        };
        let mut start = 0usize;
        for k in 1..4 {
            if sum(order[k]) < sum(order[start]) {
                start = k;
            }
        }
        let pick = |k: usize| points.get(order[(start + k) % 4]).copied().unwrap_or_default();
        Quad([pick(0), pick(1), pick(2), pick(3)])
    }

    /// The same quadrilateral after scaling every coordinate by `k` (§7.1).
    pub fn scaled(&self, k: f64) -> Quad {
        Quad(self.0.map(|p| p.scaled(k)))
    }
}

/// A straight line through `point` with unit `direction`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub point: Point,
    pub direction: Point,
}

#[cfg(test)]
mod tests {
    use super::{Point, Quad};

    #[test]
    fn canonical_order_from_any_input_order() {
        let tl = Point::new(10.0, 12.0);
        let tr = Point::new(90.0, 8.0);
        let br = Point::new(95.0, 70.0);
        let bl = Point::new(5.0, 80.0);
        let want = Quad([tl, tr, br, bl]);
        for perm in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]] {
            let pts = [tl, tr, br, bl];
            let input = perm.map(|i| pts[i]);
            assert_eq!(Quad::canonical(input), want);
        }
    }
}
