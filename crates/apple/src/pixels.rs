//! The two pixel jobs A15 needs before it calls Vision: reduce the page to 1024 px on its long
//! side, and turn it by whole quarter turns without resampling.
//!
//! The reduction is A3 from `scancraft-raster`. The raster crate has no rotation, so the quarter
//! turn is kept here; if it gains one, this one can go.

use scancraft_core::image::{Gray8, Plane};
use scancraft_core::traits::ServiceError;
use scancraft_raster::colour::to_u8_plane;
use scancraft_raster::downscale::{area_downscale, fit_long_side};

/// The long side of the page A15 reads (A15 step 1).
pub(crate) const VOTE_LONG_SIDE: u32 = 1024;

/// A3's area downscale of a grey page so that its long side is at most `long_side`. A page that
/// is already small enough is copied as it is.
pub(crate) fn fit_gray(page: &Gray8, long_side: u32) -> Result<Gray8, ServiceError> {
    let (w, h) = fit_long_side(page.width(), page.height(), long_side);
    if (w, h) == (page.width(), page.height()) {
        return Ok(page.clone());
    }
    let unit: Vec<f32> = page.data().iter().map(|&v| f32::from(v) / 255.0).collect();
    let plane = Plane::from_vec(page.width(), page.height(), unit).map_err(failed)?;
    let small = area_downscale(&plane, w, h).map_err(failed)?;
    to_u8_plane(&small).map_err(failed)
}

fn failed(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Failed(format!("preparing the page failed: {e}"))
}

/// Turns the page clockwise by `quarters` quarter turns (0 to 3, taken modulo 4), exactly: every
/// output pixel is one input pixel.
pub(crate) fn rotate_quarter_turns(page: &Gray8, quarters: u8) -> Result<Gray8, ServiceError> {
    let quarters = quarters % 4;
    let (w, h) = (page.width(), page.height());
    let (out_w, out_h) = if quarters.is_multiple_of(2) { (w, h) } else { (h, w) };
    let mut data = Vec::with_capacity(page.data().len());
    for y in 0..out_h {
        for x in 0..out_w {
            // The input pixel that lands on output pixel (x, y).
            let (sx, sy) = match quarters {
                0 => (x, y),
                // 90° clockwise sends input (x, y) to (h − 1 − y, x).
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            data.push(page.get(sx, sy).ok_or_else(|| ServiceError::Failed("rotation read outside the page".to_owned()))?);
        }
    }
    Plane::from_vec(out_w, out_h, data).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::{fit_gray, rotate_quarter_turns};
    use scancraft_core::image::Plane;

    /// 2 wide, 3 tall:  1 2 / 3 4 / 5 6
    fn small() -> Plane<u8> {
        Plane::from_vec(2, 3, vec![1, 2, 3, 4, 5, 6]).unwrap()
    }

    #[test]
    fn rotate_quarter_turn_rotates_clockwise() {
        // A quarter turn clockwise makes the left column the top row, read bottom to top:
        // 1 2      5 3 1
        // 3 4  =>  6 4 2
        // 5 6
        let turned = rotate_quarter_turns(&small(), 1).unwrap();
        assert_eq!((turned.width(), turned.height()), (3, 2));
        assert_eq!(turned.data(), &[5, 3, 1, 6, 4, 2]);
        // Half a turn reverses the pixel order, three quarters is a quarter turn back.
        assert_eq!(rotate_quarter_turns(&small(), 2).unwrap().data(), &[6, 5, 4, 3, 2, 1]);
        let back = rotate_quarter_turns(&small(), 3).unwrap();
        assert_eq!((back.width(), back.height()), (3, 2));
        assert_eq!(back.data(), &[2, 4, 6, 1, 3, 5]);
    }

    #[test]
    fn rotate_quarter_turn_four_times_is_the_identity() {
        let mut page = small();
        for _ in 0..4 {
            page = rotate_quarter_turns(&page, 1).unwrap();
        }
        assert_eq!(page, small());
        assert_eq!(rotate_quarter_turns(&small(), 0).unwrap(), small());
        // The count is taken modulo four.
        assert_eq!(rotate_quarter_turns(&small(), 5).unwrap(), rotate_quarter_turns(&small(), 1).unwrap());
    }

    #[test]
    fn fit_gray_reduces_the_long_side_and_leaves_a_small_page_alone() {
        let big = Plane::from_vec(400, 200, (0..400u32 * 200).map(|i| (i % 251) as u8).collect()).unwrap();
        let fitted = fit_gray(&big, 100).unwrap();
        assert_eq!((fitted.width(), fitted.height()), (100, 50));
        assert_eq!(fit_gray(&big, 400).unwrap(), big);
        assert_eq!(fit_gray(&big, 1024).unwrap(), big);
        // A flat page stays flat: the area average of equal values is that value.
        let flat = Plane::from_vec(40, 30, vec![200u8; 1200]).unwrap();
        assert!(fit_gray(&flat, 10).unwrap().data().iter().all(|&v| v == 200));
    }
}
