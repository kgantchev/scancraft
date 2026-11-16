//! The whole detection of one photo: A3 work image, A5–A9, A10, and A25 on the result.
//! Owner: WP8 (A25's call is added by WP11).

use crate::DetectError;
use crate::contours::contour_candidates;
use crate::edges::colour_edges;
use crate::lines::line_candidates;
use crate::refine::{Refined, build_pyramid};
use crate::select::{Prepared, select};
use rayon::prelude::*;
use scancraft_core::geom::Quad;
use scancraft_core::image::{BAND_ROWS, Plane, PlaneF32, Rgb8, luma};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{CornerOrigin, Detector, SideShape};
use scancraft_core::traits::PageDetector;
use scancraft_raster::colour::to_f32;
use scancraft_raster::downscale::{area_downscale_rgb8, fit_long_side};

/// The long side of the work image (A3).
pub const WORK_LONG_SIDE: u32 = 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// Full-resolution corners and A10's side information.
    pub refined: Refined,
    pub origin: CornerOrigin,
    pub sides: [SideShape; 4],
    /// Kept for A25, A26 and A30, which sample from it.
    pub pyramid: LumaPyramid,
}

/// The work image: the photo with its long side at most 1024 px (A3).
pub fn work_image(photo: &Rgb8) -> Result<Rgb8, DetectError> {
    let (w, h) = fit_long_side(photo.width(), photo.height(), WORK_LONG_SIDE);
    if (w, h) == (photo.width(), photo.height()) {
        return Ok(photo.clone());
    }
    Ok(area_downscale_rgb8(photo, w, h)?)
}

/// The luma of the photo, the one brightness channel (§7.3), made in fixed 64-row bands.
pub fn photo_luma(photo: &Rgb8) -> Result<PlaneF32, DetectError> {
    let w = photo.width() as usize;
    let mut out = Plane::new(photo.width(), photo.height(), 0.0f32)?;
    let src = photo.data();
    out.data_mut().par_chunks_mut(w * BAND_ROWS as usize).enumerate().for_each(|(band, chunk)| {
        let start = band * BAND_ROWS as usize * w;
        for (i, slot) in chunk.iter_mut().enumerate() {
            if let Some(p) = src.get(start + i) {
                *slot = luma([f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0]);
            }
        }
    });
    Ok(out)
}

/// Detects the page in a full-resolution photo; `apple` is A8 when the platform has it.
///
/// A service that fails, times out or finds nothing gives no candidate; it never fails the
/// detection. When no candidate is good enough, the corners start 3% inside the photo's edges and
/// the origin says so.
pub fn detect_page(photo: &Rgb8, apple: Option<&dyn PageDetector>, seed: u64) -> Result<Detection, DetectError> {
    // A3: the work image, as 8 bits for A8 and as floats for A5.
    let work8 = work_image(photo)?;
    let work = to_f32(&work8)?;
    // A5 to A7: the classical candidates.
    let maps = colour_edges(&work)?;
    let mut classical: Vec<(Detector, Quad)> = Vec::new();
    for quad in contour_candidates(&maps)? {
        classical.push((Detector::Contours, quad));
    }
    for quad in line_candidates(&maps)? {
        classical.push((Detector::Lines, quad));
    }
    // A8: the platform's candidate, if it has one. Its failure is not an error here.
    let apple_quad = apple.and_then(|service| service.detect(&work8).ok().flatten()).filter(|q| q.is_finite());
    // A10's pyramid, from the full-resolution luma.
    let pyramid = build_pyramid(&photo_luma(photo)?)?;
    // A9: score, refine, choose.
    let prepared = Prepared::new(maps)?;
    let selection = select(&prepared, &classical, apple_quad.as_ref(), &pyramid, seed)?;
    let origin = match selection.by {
        Some(by) => CornerOrigin::Detected { by, score: selection.score as f32 },
        None => CornerOrigin::NotFound,
    };
    Ok(Detection { refined: selection.refined, origin, sides: selection.sides, pyramid })
}
