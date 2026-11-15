//! A12: Heckbert's closed-form mapping from the unit square to a quadrilateral (specification
//! A12; Heckbert 1989, §2.2.3). Written by WP0 because six packages use it.

use crate::GeomError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::map::{PageMap, ScanOrder};

/// The 3 × 3 mapping x = (a·u + b·v + c)/(g·u + h·v + 1), y = (d·u + e·v + f)/(g·u + h·v + 1),
/// with (0, 0) → TL, (1, 0) → TR, (1, 1) → BR and (0, 1) → BL.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    /// a, b, c, d, e, f, g, h; the ninth coefficient is 1.
    pub m: [f64; 8],
    /// True when the parallelogram branch was taken.
    pub affine: bool,
}

impl Homography {
    /// The mapping for `quad`; `long_side` is the photo's long side L, which scales the affine test.
    pub fn from_unit_square(quad: &Quad, long_side: f64) -> Result<Homography, GeomError> {
        if !quad.is_finite() || !long_side.is_finite() || long_side <= 0.0 {
            return Err(GeomError::Degenerate("non-finite corners or size"));
        }
        let [p0, p1, p2, p3] = quad.0;
        let (x0, y0, x1, y1, x2, y2, x3, y3) = (p0.x, p0.y, p1.x, p1.y, p2.x, p2.y, p3.x, p3.y);
        let sx = x0 - x1 + x2 - x3;
        let sy = y0 - y1 + y2 - y3;
        if sx.abs() < 1e-9 * long_side && sy.abs() < 1e-9 * long_side {
            return Ok(Homography { m: [x1 - x0, x2 - x1, x0, y1 - y0, y2 - y1, y0, 0.0, 0.0], affine: true });
        }
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        if den == 0.0 || !den.is_finite() {
            return Err(GeomError::Degenerate("quadrilateral has collinear corners"));
        }
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        let m = [x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h];
        if m.iter().all(|v| v.is_finite()) { Ok(Homography { m, affine: false }) } else { Err(GeomError::Degenerate("mapping is not finite")) }
    }

    /// The same mapping for a page turned by `quarter_turns` clockwise (A12): new corner k is old
    /// corner (k − q) mod 4. The output's width and height swap when q is odd.
    pub fn from_unit_square_turned(quad: &Quad, long_side: f64, quarter_turns: u8) -> Result<Homography, GeomError> {
        let q = usize::from(quarter_turns % 4);
        let old = quad.0;
        let turned = Quad(std::array::from_fn(|k| old[(k + 4 - q) % 4]));
        Homography::from_unit_square(&turned, long_side)
    }

    /// The photo position of page point (u, v); `None` on the line where the denominator vanishes.
    pub fn apply(&self, u: f64, v: f64) -> Option<Point> {
        let [a, b, c, d, e, f, g, h] = self.m;
        let w = g * u + h * v + 1.0;
        if w == 0.0 || !w.is_finite() {
            return None;
        }
        let p = Point::new((a * u + b * v + c) / w, (d * u + e * v + f) / w);
        p.is_finite().then_some(p)
    }

    /// The inverse mapping, photo → page, from the adjugate of the 3 × 3 matrix.
    pub fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let [a, b, c, d, e, f, g, h] = self.m;
        // Adjugate of [[a, b, c], [d, e, f], [g, h, 1]].
        let (i00, i01, i02) = (e - f * h, c * h - b, b * f - c * e);
        let (i10, i11, i12) = (f * g - d, a - c * g, c * d - a * f);
        let (i20, i21, i22) = (d * h - e * g, b * g - a * h, a * e - b * d);
        let w = i20 * x + i21 * y + i22;
        if w == 0.0 || !w.is_finite() {
            return None;
        }
        let u = (i00 * x + i01 * y + i02) / w;
        let v = (i10 * x + i11 * y + i12) / w;
        (u.is_finite() && v.is_finite()).then_some((u, v))
    }
}

impl PageMap for Homography {
    fn map(&self, u: f64, v: f64) -> Option<Point> {
        self.apply(u, v)
    }

    fn scan_order(&self) -> ScanOrder {
        ScanOrder::Rows
    }

    /// Along a row the numerators and denominator are linear in u, so they advance by constant
    /// steps (A13).
    fn map_span(&self, order: ScanOrder, fixed: f64, out: &mut [Option<Point>]) {
        let n = out.len().max(1) as f64;
        let [a, b, c, d, e, f, g, h] = self.m;
        let (mut nx, mut ny, mut w, sx, sy, sw) = match order {
            ScanOrder::Rows => {
                let u0 = 0.5 / n;
                (a * u0 + b * fixed + c, d * u0 + e * fixed + f, g * u0 + h * fixed + 1.0, a / n, d / n, g / n)
            }
            ScanOrder::Columns => {
                let v0 = 0.5 / n;
                (a * fixed + b * v0 + c, d * fixed + e * v0 + f, g * fixed + h * v0 + 1.0, b / n, e / n, h / n)
            }
        };
        for slot in out.iter_mut() {
            *slot = if w != 0.0 {
                let p = Point::new(nx / w, ny / w);
                p.is_finite().then_some(p)
            } else {
                None
            };
            nx += sx;
            ny += sy;
            w += sw;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Homography;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::map::{PageMap, ScanOrder};

    fn quad() -> Quad {
        Quad([Point::new(412.0, 380.0), Point::new(3610.2, 455.7), Point::new(3301.9, 2840.4), Point::new(611.3, 2702.8)])
    }

    #[test]
    fn corners_map_exactly() {
        let h = Homography::from_unit_square(&quad(), 4032.0).unwrap();
        assert!(!h.affine);
        for ((u, v), p) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().zip(quad().0) {
            assert!(h.apply(u, v).unwrap().distance(p) < 1e-9);
        }
    }

    #[test]
    fn parallelogram_takes_the_affine_branch() {
        let q = Quad([Point::new(0.0, 0.0), Point::new(2.0, 0.0), Point::new(3.0, 1.0), Point::new(1.0, 1.0)]);
        let h = Homography::from_unit_square(&q, 4032.0).unwrap();
        assert!(h.affine);
        assert!(h.apply(1.0, 1.0).unwrap().distance(Point::new(3.0, 1.0)) < 1e-12);
        assert!(h.apply(0.5, 0.5).unwrap().distance(Point::new(1.5, 0.5)) < 1e-12);
    }

    #[test]
    fn inverse_round_trips() {
        let h = Homography::from_unit_square(&quad(), 4032.0).unwrap();
        for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.93, 0.07), (0.0, 1.0)] {
            let p = h.apply(u, v).unwrap();
            let (u2, v2) = h.invert(p.x, p.y).unwrap();
            assert!((u - u2).abs() < 1e-9 && (v - v2).abs() < 1e-9);
        }
    }

    #[test]
    fn folding_a_quarter_turn_matches_rotating_the_output() {
        let h0 = Homography::from_unit_square(&quad(), 4032.0).unwrap();
        for q in 1u8..4 {
            let hq = Homography::from_unit_square_turned(&quad(), 4032.0, q).unwrap();
            for (u, v) in [(0.2, 0.3), (0.7, 0.1), (0.5, 0.9)] {
                // Turning the output clockwise q times: output (u, v) shows page point r^q(u, v),
                // with r(u, v) = (v, 1 − u).
                let (mut uu, mut vv) = (u, v);
                for _ in 0..q {
                    (uu, vv) = (vv, 1.0 - uu);
                }
                assert!(hq.apply(u, v).unwrap().distance(h0.apply(uu, vv).unwrap()) < 1e-7, "q={q}");
            }
        }
    }

    #[test]
    fn incremental_rows_match_direct_evaluation() {
        let h = Homography::from_unit_square(&quad(), 4032.0).unwrap();
        let mut out = vec![None; 97];
        h.map_span(ScanOrder::Rows, 0.37, &mut out);
        for (k, p) in out.iter().enumerate() {
            let direct = h.apply((k as f64 + 0.5) / 97.0, 0.37).unwrap();
            assert!(p.unwrap().distance(direct) < 1e-6);
        }
    }

    #[test]
    fn degenerate_input_is_an_error() {
        let p = Point::new(1.0, 1.0);
        assert!(Homography::from_unit_square(&Quad([p, Point::new(2.0, 2.0), Point::new(3.0, 3.0), Point::new(4.0, 4.0)]), 4032.0).is_err());
        assert!(Homography::from_unit_square(&Quad([Point::new(f64::NAN, 0.0), p, p, p]), 4032.0).is_err());
    }
}
