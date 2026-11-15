//! The geometry of Screen 2's edits (§4.3): where a corner may go, how a side slides along its
//! neighbours, the fallback outlines, and when an outline is invalid.

use crate::curve::{bent_side_points, polyline_crosses};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::recipe::SideShape;

/// A corner may go this far outside the photo, as a share of its width or height (§4.3).
pub const CORNER_MARGIN: f64 = 0.15;
/// The not-found fallback puts the corners this far inside the photo's edges (§4.3).
pub const NOT_FOUND_INSET: f64 = 0.03;
/// A corner angle outside this range makes the outline invalid (§4.3), in degrees.
pub const MIN_ANGLE: f64 = 20.0;
pub const MAX_ANGLE: f64 = 160.0;
/// Below this sine of the angle between two sides they count as parallel when sliding a corner.
pub const PARALLEL_EPS: f64 = 1e-3;

/// Where a corner dragged to `p` ends up: at most 15% of the photo's width or height outside it.
/// A photo of unknown size (0 × 0, before it is decoded) clamps nothing.
pub fn clamp_corner(p: Point, photo: (u32, u32)) -> Point {
    if photo.0 == 0 || photo.1 == 0 {
        return p;
    }
    let (w, h) = (f64::from(photo.0), f64::from(photo.1));
    Point::new(p.x.clamp(-CORNER_MARGIN * w, (1.0 + CORNER_MARGIN) * w), p.y.clamp(-CORNER_MARGIN * h, (1.0 + CORNER_MARGIN) * h))
}

/// The photo's own corners, TL, TR, BR, BL.
pub fn full_image_corners(photo: (u32, u32)) -> Quad {
    let (w, h) = (f64::from(photo.0), f64::from(photo.1));
    Quad([Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)])
}

/// Corners 3% inside the photo's edges: where they start when no page was found.
pub fn not_found_corners(photo: (u32, u32)) -> Quad {
    let (w, h) = (f64::from(photo.0), f64::from(photo.1));
    let (dx, dy) = (NOT_FOUND_INSET * w, NOT_FOUND_INSET * h);
    Quad([Point::new(dx, dy), Point::new(w - dx, dy), Point::new(w - dx, h - dy), Point::new(dx, h - dy)])
}

fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}

fn cross(a: Point, b: Point) -> f64 {
    a.x * b.y - a.y * b.x
}

/// The point where the line through `p` along `d` meets the line through `q` along `e`.
fn meet(p: Point, d: Point, q: Point, e: Point) -> Option<Point> {
    let denominator = cross(d, e);
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let t = cross(sub(q, p), e) / denominator;
    let hit = Point::new(p.x + t * d.x, p.y + t * d.y);
    hit.is_finite().then_some(hit)
}

/// The corners of `side` in the quad's cyclic order (top 0→1, right 1→2, bottom 2→3, left 3→0).
fn cyclic(side: Side) -> (usize, usize) {
    match side {
        Side::Top => (0, 1),
        Side::Right => (1, 2),
        Side::Bottom => (2, 3),
        Side::Left => (3, 0),
    }
}

/// Moves `side` by `offset` photo pixels along its outward normal (positive is away from the page's
/// middle). Its two corners slide along the neighbouring sides, so those keep their angles; where a
/// neighbour is nearly parallel to the side, both corners move by the drag instead. `None` if the
/// numbers are not usable.
pub fn move_side(corners: &Quad, side: Side, offset: f64) -> Option<Quad> {
    if !corners.is_finite() || !offset.is_finite() {
        return None;
    }
    let (i, j) = cyclic(side);
    let (c0, c1) = (corners.0[i], corners.0[j]);
    let d = sub(c1, c0);
    let length = d.x.hypot(d.y);
    if length < 1e-9 {
        return None;
    }
    // The normal turns the direction a quarter; flip it to point away from the page's middle.
    let mut n = Point::new(d.y / length, -d.x / length);
    let centre = Point::new(corners.0.iter().map(|p| p.x).sum::<f64>() / 4.0, corners.0.iter().map(|p| p.y).sum::<f64>() / 4.0);
    let middle = Point::new((c0.x + c1.x) / 2.0, (c0.y + c1.y) / 2.0);
    if n.x * (middle.x - centre.x) + n.y * (middle.y - centre.y) < 0.0 {
        n = Point::new(-n.x, -n.y);
    }
    let shift = Point::new(n.x * offset, n.y * offset);
    let (prev, next) = (corners.0[(i + 3) % 4], corners.0[(j + 1) % 4]);
    let slide = |corner: Point, other_end: Point| -> Point {
        let e = sub(corner, other_end);
        let sine = cross(d, e).abs() / (length * e.x.hypot(e.y)).max(1e-12);
        if sine < PARALLEL_EPS {
            return Point::new(corner.x + shift.x, corner.y + shift.y);
        }
        meet(Point::new(c0.x + shift.x, c0.y + shift.y), d, corner, e).unwrap_or(Point::new(corner.x + shift.x, corner.y + shift.y))
    };
    let mut out = *corners;
    out.0[i] = slide(c0, prev);
    out.0[j] = slide(c1, next);
    out.is_finite().then_some(out)
}

/// The interior angle at corner `k` in degrees.
fn angle_at(q: &Quad, k: usize) -> f64 {
    let (a, b, c) = (q.0[(k + 3) % 4], q.0[k], q.0[(k + 1) % 4]);
    let (u, v) = (sub(a, b), sub(c, b));
    let norm = u.x.hypot(u.y) * v.x.hypot(v.y);
    if norm < 1e-12 {
        return 0.0;
    }
    ((u.x * v.x + u.y * v.y) / norm).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Why the outline is invalid (§4.3), or `None`: the four corners must stay convex with every angle
/// from 20° to 160°, and a bent side must not cross another side. A curved side may dip inward; the
/// move itself is never blocked.
pub fn invalid_reason(corners: &Quad, sides: &[SideShape; 4]) -> Option<String> {
    if !corners.is_finite() {
        return Some("a corner is not a usable position".to_string());
    }
    let turns: Vec<f64> =
        (0..4).map(|k| cross(sub(corners.0[(k + 1) % 4], corners.0[k]), sub(corners.0[(k + 2) % 4], corners.0[(k + 1) % 4]))).collect();
    if !(turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0)) {
        return Some("the corners no longer form a convex shape".to_string());
    }
    for k in 0..4 {
        let a = angle_at(corners, k);
        if !(MIN_ANGLE..=MAX_ANGLE).contains(&a) {
            return Some(format!("a corner angle of {a:.0}° is outside {MIN_ANGLE:.0}°–{MAX_ANGLE:.0}°"));
        }
    }
    for (index, shape) in sides.iter().enumerate() {
        let SideShape::Bent { handles } = shape else { continue };
        let side = Side::ALL[index];
        let line = bent_side_points(corners, side, handles);
        if line.is_empty() {
            return Some("a bend handle is not a usable position".to_string());
        }
        for (other_index, other) in Side::ALL.iter().enumerate().filter(|(k, _)| *k != index) {
            // A neighbour shares a corner with the curve, so it may touch there; only a proper
            // crossing of the middle counts. Bent neighbours are compared by their own curves.
            let (oa, ob) = other.corners();
            let segment = (corners.0[oa], corners.0[ob]);
            let crossed = match sides.get(other_index) {
                Some(SideShape::Bent { handles: h }) => {
                    let other_line = bent_side_points(corners, *other, h);
                    other_line.windows(2).any(|w| polyline_crosses(&line, (w[0], w[1])))
                }
                _ => polyline_crosses(&line, segment),
            };
            if crossed {
                return Some("a bent side crosses another side".to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Quad {
        Quad([Point::new(100.0, 100.0), Point::new(500.0, 100.0), Point::new(500.0, 400.0), Point::new(100.0, 400.0)])
    }

    fn straight() -> [SideShape; 4] {
        [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight]
    }

    #[test]
    fn a_corner_may_leave_the_photo_by_fifteen_percent() {
        let photo = (1000, 800);
        let clamped = clamp_corner(Point::new(-500.0, 1100.0), photo);
        assert!((clamped.x + 150.0).abs() < 1e-9 && (clamped.y - 920.0).abs() < 1e-9, "{clamped:?}");
        assert_eq!(clamp_corner(Point::new(5.0, 900.0), photo), Point::new(5.0, 900.0));
        let clamped = clamp_corner(Point::new(1500.0, -300.0), photo);
        assert!((clamped.x - 1150.0).abs() < 1e-9 && (clamped.y + 120.0).abs() < 1e-9, "{clamped:?}");
        assert_eq!(clamp_corner(Point::new(10.0, 20.0), photo), Point::new(10.0, 20.0));
        // A photo of unknown size clamps nothing.
        assert_eq!(clamp_corner(Point::new(-9e9, 9e9), (0, 0)), Point::new(-9e9, 9e9));
    }

    #[test]
    fn the_fallback_outlines_sit_on_and_inside_the_photo() {
        assert_eq!(full_image_corners((200, 100)).br(), Point::new(200.0, 100.0));
        let nf = not_found_corners((1000, 500));
        assert_eq!((nf.tl(), nf.br()), (Point::new(30.0, 15.0), Point::new(970.0, 485.0)));
    }

    #[test]
    fn moving_a_side_slides_its_corners_along_the_neighbours() {
        // The top side of a rectangle dragged outward goes up and its corners stay on the sides.
        let q = move_side(&rect(), Side::Top, 30.0).unwrap();
        assert_eq!((q.tl(), q.tr()), (Point::new(100.0, 70.0), Point::new(500.0, 70.0)));
        assert_eq!((q.bl(), q.br()), (rect().bl(), rect().br()));
        // Every side's positive offset is outward.
        assert_eq!(move_side(&rect(), Side::Left, 30.0).unwrap().tl(), Point::new(70.0, 100.0));
        assert_eq!(move_side(&rect(), Side::Right, 30.0).unwrap().tr(), Point::new(530.0, 100.0));
        assert_eq!(move_side(&rect(), Side::Bottom, 30.0).unwrap().bl(), Point::new(100.0, 430.0));
        // Negative is inward.
        assert_eq!(move_side(&rect(), Side::Top, -50.0).unwrap().tl(), Point::new(100.0, 150.0));
    }

    #[test]
    fn a_slid_corner_keeps_the_neighbours_direction() {
        // A trapezoid: the sides lean. Moving the top out slides the corners along the leaning sides.
        let trap = Quad([Point::new(150.0, 100.0), Point::new(450.0, 100.0), Point::new(500.0, 400.0), Point::new(100.0, 400.0)]);
        let q = move_side(&trap, Side::Top, 40.0).unwrap();
        assert!((q.tl().y - 60.0).abs() < 1e-9 && (q.tr().y - 60.0).abs() < 1e-9);
        // The left side's direction (from BL up to TL) is unchanged.
        let before = (trap.tl().x - trap.bl().x) / (trap.tl().y - trap.bl().y);
        let after = (q.tl().x - q.bl().x) / (q.tl().y - q.bl().y);
        assert!((before - after).abs() < 1e-9);
    }

    #[test]
    fn with_a_nearly_parallel_neighbour_both_corners_move_by_the_drag() {
        // The left side leans by well under the threshold: it is parallel to the top's corner slide.
        let q = Quad([Point::new(100.0, 100.0), Point::new(500.0, 100.0), Point::new(500.0, 400.0), Point::new(100.0, 400.0)]);
        // Make the top side nearly parallel to the left one by squashing the quad into a sliver.
        let sliver = Quad([Point::new(0.0, 0.0), Point::new(1000.0, 0.0), Point::new(1000.0, 0.5), Point::new(0.0, 0.5)]);
        let moved = move_side(&sliver, Side::Left, 10.0).unwrap();
        // The left side moves left by 10 whatever the sliver does.
        assert!((moved.tl().x + 10.0).abs() < 1e-6 && (moved.bl().x + 10.0).abs() < 1e-6, "{moved:?}");
        let _ = q;
        assert!(move_side(&rect(), Side::Top, f64::NAN).is_none());
        assert!(move_side(&Quad([Point::new(1.0, 1.0); 4]), Side::Top, 5.0).is_none());
    }

    #[test]
    fn an_outline_is_invalid_when_it_stops_being_convex_or_its_angles_leave_the_range() {
        assert_eq!(invalid_reason(&rect(), &straight()), None);
        // A corner pulled inside the shape makes it concave.
        let mut concave = rect();
        concave.0[2] = Point::new(200.0, 150.0);
        assert!(invalid_reason(&concave, &straight()).unwrap().contains("convex"));
        // A very sharp corner.
        // A slanted sliver: the corner at the top left is 14°.
        let sharp = Quad([Point::new(0.0, 0.0), Point::new(800.0, 0.0), Point::new(900.0, 25.0), Point::new(100.0, 25.0)]);
        assert!(invalid_reason(&sharp, &straight()).unwrap().contains("angle"));
        assert!(
            invalid_reason(&Quad([Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)]), &straight())
                .is_some()
        );
    }

    #[test]
    fn a_bent_side_may_dip_inward_but_not_cross_another_side() {
        let mut sides = straight();
        // The top bows outward: fine. Dipping inward by a little: fine too.
        sides[0] = SideShape::Bent { handles: [Point::new(200.0, 80.0), Point::new(300.0, 70.0), Point::new(400.0, 80.0)] };
        assert_eq!(invalid_reason(&rect(), &sides), None);
        sides[0] = SideShape::Bent { handles: [Point::new(200.0, 130.0), Point::new(300.0, 150.0), Point::new(400.0, 130.0)] };
        assert_eq!(invalid_reason(&rect(), &sides), None);
        // Dragged down through the bottom side it crosses it.
        sides[0] = SideShape::Bent { handles: [Point::new(200.0, 380.0), Point::new(300.0, 450.0), Point::new(400.0, 380.0)] };
        assert!(invalid_reason(&rect(), &sides).unwrap().contains("crosses"));
    }
}
