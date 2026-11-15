//! The generator's own camera (specification §7.8, written again here so the scenes are not made
//! with the pipeline's code): x right, y down, z away from the camera; the principal point is the
//! photo's centre and the focal length is `f` long sides.
//!
//! **Tilt** is the angle between the page's normal and the optical axis, exactly: the page is
//! turned by `tilt` about an axis in the image plane at angle `azimuth`, then rolled about the
//! optical axis. The tables of A27 and A29 are stated in these terms.

use crate::geom::{Mat3, mul, mul_vec, rotation_about, rotation_z};
use scancraft_core::geom::Point;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCamera {
    pub width: f64,
    pub height: f64,
    /// Focal length in units of the photo's long side.
    pub f: f64,
}

impl SceneCamera {
    pub fn focal_px(&self) -> f64 {
        self.f * self.width.max(self.height)
    }

    /// The photo position of a camera-space point, or `None` behind the camera.
    pub fn project(&self, q: [f64; 3]) -> Option<Point> {
        if !q.iter().all(|v| v.is_finite()) || q[2] <= 1e-9 {
            return None;
        }
        let fp = self.focal_px();
        Some(Point::new(self.width / 2.0 + fp * q[0] / q[2], self.height / 2.0 + fp * q[1] / q[2]))
    }
}

/// Where the sheet sits in front of the camera: camera point = R · (sheet point) + T.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub r: Mat3,
    pub t: [f64; 3],
}

impl Pose {
    /// The rotation for a tilt, its azimuth and a roll (all in radians).
    pub fn rotation(tilt: f64, azimuth: f64, roll: f64) -> Mat3 {
        let axis = [azimuth.cos(), azimuth.sin(), 0.0];
        mul(&rotation_z(roll), &rotation_about(axis, tilt))
    }

    /// The pose that puts the sheet point `centre` at camera-space position `at`.
    pub fn new(r: Mat3, centre: [f64; 3], at: [f64; 3]) -> Pose {
        let rc = mul_vec(&r, centre);
        Pose { r, t: [at[0] - rc[0], at[1] - rc[1], at[2] - rc[2]] }
    }

    pub fn to_camera(&self, p: [f64; 3]) -> [f64; 3] {
        let q = mul_vec(&self.r, p);
        [q[0] + self.t[0], q[1] + self.t[1], q[2] + self.t[2]]
    }

    pub fn project(&self, cam: &SceneCamera, p: [f64; 3]) -> Option<Point> {
        cam.project(self.to_camera(p))
    }
}

/// How the page is to sit in the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub tilt: f64,
    pub azimuth: f64,
    pub roll: f64,
    /// How much of the photo the page's bounding box fills, along the tighter axis (0.55–0.95).
    pub coverage: f64,
    /// Where the page's middle goes within the room the frame leaves, each −1 … 1.
    pub shift: (f64, f64),
}

/// The share of the photo's width and height a page may hang outside it (§10.1).
pub const MAX_OUTSIDE: f64 = 0.05;

/// The bounding box of the projected sheet, from a grid of its points.
fn bounds(pose: &Pose, cam: &SceneCamera, surface: &dyn Fn(f64, f64) -> [f64; 3]) -> Option<(Point, Point)> {
    let (mut lo, mut hi) = (Point::new(f64::INFINITY, f64::INFINITY), Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    for j in 0..=12 {
        for i in 0..=12 {
            let p = pose.project(cam, surface(f64::from(i) / 12.0, f64::from(j) / 12.0))?;
            lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
        }
    }
    Some((lo, hi))
}

/// Places `surface` (page fractions to sheet points, in points) in front of `cam`.
///
/// The distance is found so the page's bounding box fills `coverage` of the photo, then the page
/// is shifted within the frame, by up to the room that is left plus the 5% it may hang outside.
pub fn place(cam: &SceneCamera, p: &Placement, surface: &dyn Fn(f64, f64) -> [f64; 3]) -> Option<Pose> {
    let r = Pose::rotation(p.tilt, p.azimuth, p.roll);
    let centre = surface(0.5, 0.5);
    let fp = cam.focal_px();
    let (mut ox, mut oy, mut d) = (0.0, 0.0, 1.0f64);
    // Start from the page's size and the focal length, then correct by what the projection says.
    let extent = (0..4).map(|k| surface(f64::from(k & 1), f64::from(k >> 1))).fold(0.0f64, |m, q| m.max(q[0].hypot(q[1])));
    d = d.max(fp * 2.0 * extent / (p.coverage * cam.width.min(cam.height)));
    for _ in 0..6 {
        let pose = Pose::new(r, centre, [ox, oy, d]);
        let (lo, hi) = bounds(&pose, cam, surface)?;
        let ratio = ((hi.x - lo.x) / cam.width).max((hi.y - lo.y) / cam.height);
        if ratio.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        d *= ratio / p.coverage;
    }
    // Shift the bounding box's middle to the chosen spot, a few times since depth varies.
    let pose = Pose::new(r, centre, [0.0, 0.0, d]);
    let (lo, hi) = bounds(&pose, cam, surface)?;
    let (w, h) = (hi.x - lo.x, hi.y - lo.y);
    let room = ((cam.width - w) / 2.0 + MAX_OUTSIDE * cam.width * 0.9, (cam.height - h) / 2.0 + MAX_OUTSIDE * cam.height * 0.9);
    let target =
        Point::new(cam.width / 2.0 + p.shift.0.clamp(-1.0, 1.0) * room.0.max(0.0), cam.height / 2.0 + p.shift.1.clamp(-1.0, 1.0) * room.1.max(0.0));
    for _ in 0..4 {
        let pose = Pose::new(r, centre, [ox, oy, d]);
        let (lo, hi) = bounds(&pose, cam, surface)?;
        let now = Point::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
        ox += (target.x - now.x) * d / fp;
        oy += (target.y - now.y) * d / fp;
    }
    let pose = Pose::new(r, centre, [ox, oy, d]);
    // Every point of the sheet must be in front of the camera.
    bounds(&pose, cam, surface).map(|_| pose)
}

/// How far the projected bounding box hangs outside the photo, as a share of the photo's width
/// and height, each at most; 0 when it is inside.
pub fn overhang(pose: &Pose, cam: &SceneCamera, surface: &dyn Fn(f64, f64) -> [f64; 3]) -> Option<f64> {
    let (lo, hi) = bounds(pose, cam, surface)?;
    let over = [(-lo.x) / cam.width, (hi.x - cam.width) / cam.width, (-lo.y) / cam.height, (hi.y - cam.height) / cam.height];
    Some(over.into_iter().fold(0.0, f64::max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_a4(u: f64, v: f64) -> [f64; 3] {
        [u * 595.28, v * 841.89, 0.0]
    }

    #[test]
    fn the_tilt_is_the_angle_between_the_normal_and_the_optical_axis() {
        for tilt in [0.0f64, 0.3, 0.87] {
            for (azimuth, roll) in [(0.0, 0.0), (1.1, 2.0), (4.0, -1.0)] {
                let r = Pose::rotation(tilt, azimuth, roll);
                let normal = mul_vec(&r, [0.0, 0.0, 1.0]);
                let angle = normal[2].clamp(-1.0, 1.0).acos();
                assert!((angle - tilt).abs() < 1e-9, "{tilt} {azimuth} {roll}: {angle}");
            }
        }
    }

    #[test]
    fn a_placed_page_fills_the_chosen_share_of_the_frame() {
        let cam = SceneCamera { width: 1024.0, height: 768.0, f: 0.8 };
        for coverage in [0.55, 0.75, 0.95] {
            let p = Placement { tilt: 0.4, azimuth: 1.0, roll: 0.3, coverage, shift: (0.0, 0.0) };
            let pose = place(&cam, &p, &flat_a4).unwrap();
            let (lo, hi) = bounds(&pose, &cam, &flat_a4).unwrap();
            let got = ((hi.x - lo.x) / cam.width).max((hi.y - lo.y) / cam.height);
            assert!((got - coverage).abs() < 0.03, "{coverage}: {got}");
            // Centred, as asked.
            assert!(((lo.x + hi.x) / 2.0 - 512.0).abs() < 10.0 && ((lo.y + hi.y) / 2.0 - 384.0).abs() < 10.0);
            assert!(overhang(&pose, &cam, &flat_a4).unwrap() < 0.01);
        }
    }

    #[test]
    fn a_shifted_page_hangs_outside_by_at_most_five_percent() {
        let cam = SceneCamera { width: 1024.0, height: 768.0, f: 1.0 };
        for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 0.0)] {
            let p = Placement { tilt: 0.2, azimuth: 0.0, roll: 1.2, coverage: 0.9, shift: (sx, sy) };
            let pose = place(&cam, &p, &flat_a4).unwrap();
            let over = overhang(&pose, &cam, &flat_a4).unwrap();
            assert!(over <= MAX_OUTSIDE + 0.01, "{sx},{sy}: {over}");
        }
    }

    #[test]
    fn a_point_behind_the_camera_does_not_project() {
        let cam = SceneCamera { width: 100.0, height: 100.0, f: 1.0 };
        assert!(cam.project([0.0, 0.0, -1.0]).is_none());
        assert!(cam.project([f64::NAN, 0.0, 1.0]).is_none());
        assert!(cam.project([0.0, 0.0, 2.0]).unwrap().distance(Point::new(50.0, 50.0)) < 1e-12);
    }
}
