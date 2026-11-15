//! Pixel buffers (specification §6.1, §7.3, §7.6).
//!
//! A [`Plane`] is a row-major grid of one value type: `u8` grey, `[u8; 3]` RGB, `f32` or
//! `[f32; 3]` for processing. Values are sRGB-encoded unless a function says "linear" (§7.3).
//! Pixel loops split the rows into fixed bands of [`BAND_ROWS`] rows, so results don't depend on
//! the number of threads (§7.6).

use crate::CoreError;
use serde::{Deserialize, Serialize};

/// Rows per band for every parallel pixel loop (§7.6).
pub const BAND_ROWS: u32 = 64;
/// Largest side of any buffer (§9).
pub const MAX_SIDE: u32 = 32_768;
/// Largest pixel count of any buffer: the 100-megapixel input limit (§9).
pub const MAX_PIXELS: u64 = 100_000_000;
/// Largest warped page (§9).
pub const MAX_PAGE_PIXELS: u64 = 40_000_000;

/// Checks a buffer size against the limits and returns its pixel count.
pub fn checked_len(width: u32, height: u32) -> Result<usize, CoreError> {
    let px = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || px > MAX_PIXELS {
        return Err(CoreError::BadSize { width, height });
    }
    usize::try_from(px).map_err(|_| CoreError::BadSize { width, height })
}

/// A row-major grid of `T`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plane<T> {
    width: u32,
    height: u32,
    data: Vec<T>,
}

/// 8-bit grey (luma).
pub type Gray8 = Plane<u8>;
/// 8-bit sRGB.
pub type Rgb8 = Plane<[u8; 3]>;
/// One f32 channel, usually in [0, 1].
pub type PlaneF32 = Plane<f32>;
/// Three f32 channels, sRGB-encoded in [0, 1] unless stated.
pub type RgbF32 = Plane<[f32; 3]>;

impl<T: Copy> Plane<T> {
    /// A plane filled with `fill`, after checking the size limits.
    pub fn new(width: u32, height: u32, fill: T) -> Result<Self, CoreError> {
        let len = checked_len(width, height)?;
        Ok(Self { width, height, data: vec![fill; len] })
    }

    /// Wraps existing data, which must hold exactly `width × height` values.
    pub fn from_vec(width: u32, height: u32, data: Vec<T>) -> Result<Self, CoreError> {
        let len = checked_len(width, height)?;
        if data.len() != len {
            return Err(CoreError::BadBuffer { len: data.len(), expected: len });
        }
        Ok(Self { width, height, data })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn data(&self) -> &[T] {
        &self.data
    }
    pub fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
    pub fn into_vec(self) -> Vec<T> {
        self.data
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        usize::try_from(u64::from(y) * u64::from(self.width) + u64::from(x)).ok()
    }

    pub fn get(&self, x: u32, y: u32) -> Option<T> {
        self.index(x, y).and_then(|i| self.data.get(i).copied())
    }

    /// Sets one value; returns false when (x, y) is outside the plane.
    pub fn set(&mut self, x: u32, y: u32, value: T) -> bool {
        match self.index(x, y).and_then(|i| self.data.get_mut(i)) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    pub fn row(&self, y: u32) -> Option<&[T]> {
        let start = self.index(0, y)?;
        self.data.get(start..start + self.width as usize)
    }

    /// The rows split into bands of [`BAND_ROWS`] rows, for `rayon`'s `par_chunks_mut` or a plain
    /// loop. The last band may be shorter. Band `b` starts at row `b × BAND_ROWS`.
    pub fn band_len(&self) -> usize {
        self.width as usize * BAND_ROWS as usize
    }
}

/// The row ranges of the fixed bands for an image `height` rows tall (§7.6).
pub fn bands(height: u32) -> impl Iterator<Item = std::ops::Range<u32>> {
    (0..height.div_ceil(BAND_ROWS)).map(move |b| {
        let start = b * BAND_ROWS;
        start..(start + BAND_ROWS).min(height)
    })
}

/// A 1-bit image, rows packed most significant bit first, each row padded to whole bytes.
/// 1 means ink (specification A18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitPlane {
    width: u32,
    height: u32,
    stride: usize,
    data: Vec<u8>,
}

impl BitPlane {
    /// An all-paper (all 0) image.
    pub fn new(width: u32, height: u32) -> Result<Self, CoreError> {
        checked_len(width, height)?;
        let stride = (width as usize).div_ceil(8);
        let len = stride.checked_mul(height as usize).ok_or(CoreError::BadSize { width, height })?;
        Ok(Self { width, height, stride, data: vec![0; len] })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Bytes per row.
    pub fn stride(&self) -> usize {
        self.stride
    }
    /// The packed rows.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn locate(&self, x: u32, y: u32) -> Option<(usize, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((y as usize * self.stride + (x as usize / 8), 0x80u8 >> (x % 8)))
    }

    pub fn get(&self, x: u32, y: u32) -> Option<bool> {
        let (i, mask) = self.locate(x, y)?;
        self.data.get(i).map(|b| b & mask != 0)
    }

    /// Sets one bit; returns false when (x, y) is outside the image.
    pub fn set(&mut self, x: u32, y: u32, ink: bool) -> bool {
        let Some((i, mask)) = self.locate(x, y) else { return false };
        match self.data.get_mut(i) {
            Some(b) => {
                if ink {
                    *b |= mask;
                } else {
                    *b &= !mask;
                }
                true
            }
            None => false,
        }
    }
}

/// Luma, the single brightness channel, on sRGB-encoded values (§7.3).
pub fn luma(rgb: [f32; 3]) -> f32 {
    0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::{BAND_ROWS, BitPlane, Plane, bands};

    #[test]
    fn sizes_are_checked() {
        assert!(Plane::new(0, 10, 0u8).is_err());
        assert!(Plane::new(40_000, 10, 0u8).is_err());
        assert!(Plane::new(20_000, 20_000, 0u8).is_err());
        assert!(Plane::from_vec(2, 2, vec![0u8; 3]).is_err());
        assert!(Plane::new(10, 10, 0u8).is_ok());
    }

    #[test]
    fn get_and_set_stay_in_bounds() {
        let mut p = Plane::new(3, 2, 0u8).unwrap();
        assert!(p.set(2, 1, 7));
        assert_eq!(p.get(2, 1), Some(7));
        assert!(!p.set(3, 0, 1));
        assert_eq!(p.get(0, 2), None);
        assert_eq!(p.row(1), Some(&[0u8, 0, 7][..]));
    }

    #[test]
    fn bands_cover_every_row_once() {
        for h in [1u32, 63, 64, 65, 200] {
            let rows: Vec<u32> = bands(h).flatten().collect();
            assert_eq!(rows, (0..h).collect::<Vec<_>>());
            assert!(bands(h).all(|r| r.len() as u32 <= BAND_ROWS));
        }
    }

    #[test]
    fn bit_plane_packs_msb_first() {
        let mut b = BitPlane::new(10, 2).unwrap();
        assert_eq!(b.stride(), 2);
        assert!(b.set(0, 0, true));
        assert!(b.set(9, 1, true));
        assert_eq!(b.data(), &[0x80, 0x00, 0x00, 0x40]);
        assert_eq!(b.get(9, 1), Some(true));
        assert!(!b.set(10, 0, true));
    }
}
