//! The pinhole camera of specification §7.8.

use crate::geom::Point;
use serde::{Deserialize, Serialize};

/// Where a photo's focal length came from (A11 step 3, A27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocalSource {
    /// EXIF's 35 mm-equivalent focal length.
    Known,
    /// Estimated from the corners (Zhang–He).
    Estimated,
    /// The typical 0.75 of a phone's main camera.
    Typical,
    /// Fitted with the page's sheet (A27).
    Fitted,
}

/// A pinhole camera: principal point at the photo's centre, focal length `f · L` pixels, with L
/// the photo's long side. Camera coordinates: x right, y down, z away from the camera.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Photo width in pixels.
    pub width: f64,
    /// Photo height in pixels.
    pub height: f64,
    /// Focal length in units of the long side.
    pub f: f64,
    pub source: FocalSource,
}

impl Camera {
    pub fn long_side(&self) -> f64 {
        self.width.max(self.height)
    }

    /// Focal length in pixels.
    pub fn focal_px(&self) -> f64 {
        self.f * self.long_side()
    }

    /// The photo position of a point in camera coordinates; `None` behind or on the camera's plane.
    pub fn project(&self, q: [f64; 3]) -> Option<Point> {
        if !q.iter().all(|v| v.is_finite()) || q[2] <= 0.0 {
            return None;
        }
        let fp = self.focal_px();
        Some(Point::new(self.width / 2.0 + fp * q[0] / q[2], self.height / 2.0 + fp * q[1] / q[2]))
    }

    /// The direction (z = 1) of the ray through photo position `p`.
    pub fn ray(&self, p: Point) -> [f64; 3] {
        let fp = self.focal_px();
        [(p.x - self.width / 2.0) / fp, (p.y - self.height / 2.0) / fp, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::{Camera, FocalSource};
    use crate::geom::Point;

    #[test]
    fn project_and_ray_agree() {
        let c = Camera { width: 4032.0, height: 3024.0, f: 0.75, source: FocalSource::Typical };
        let p = Point::new(1000.0, 2500.0);
        let r = c.ray(p);
        let q = c.project([r[0] * 3.0, r[1] * 3.0, 3.0]).unwrap();
        assert!(q.distance(p) < 1e-9);
        assert!(c.project([0.0, 0.0, -1.0]).is_none());
    }
}
