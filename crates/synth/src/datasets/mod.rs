//! The data sets the evaluation reads (specification §10.2): SmartDoc 2015, the UVDoc benchmark,
//! the ScanCraft reference set and the synthetic scenes `synth` writes. Each loader reads the
//! published on-disk layout and gives a `Dataset` of `Sample`s; none loads pixels, so a set of
//! thousands of frames costs only its ground truth.

pub mod reference;
pub mod smartdoc;
pub mod synthetic;
pub mod uvdoc;

use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use std::path::{Path, PathBuf};

/// The most samples a loader keeps and the most skipped-row messages it records.
pub const MAX_SAMPLES: usize = 200_000;
pub const MAX_MESSAGES: usize = 100;
/// The most bytes of one metadata or grid file a loader reads (after decompressing).
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// One photo and what is known about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// A stable name: the frame's path, the file's stem, or the manifest's file name.
    pub id: String,
    pub image: PathBuf,
    pub category: Option<String>,
    /// The true page corners, TL, TR, BR, BL.
    pub corners: Option<Quad>,
    /// True width ÷ height.
    pub ratio: Option<f64>,
    /// Where each point of the flat page lies in the photo: `grid[j][i]` is page point
    /// (i / (cols − 1), j / (rows − 1)). Rows and columns may differ.
    pub grid: Option<Vec<Vec<Point>>>,
    /// A spread's spine, top and bottom.
    pub spine: Option<[Point; 2]>,
    /// The page's true text, for the character error rate.
    pub text: Option<String>,
    /// The page's true ink mask (synthetic scenes).
    pub ink: Option<PathBuf>,
}

impl Sample {
    pub fn new(id: impl Into<String>, image: PathBuf) -> Sample {
        Sample { id: id.into(), image, category: None, corners: None, ratio: None, grid: None, spine: None, text: None, ink: None }
    }

    /// The photo's size in pixels, from its header: a PNG's IHDR, else whatever `craft-image-codecs`
    /// can probe (JPEG, HEIC, TIFF, WebP and more).
    pub fn size(&self) -> Result<(u32, u32), CoreError> {
        use std::io::Read;
        let fail = |e: &dyn std::fmt::Display| scene(format!("{}: {e}", self.image.display()));
        let mut head = [0u8; 24];
        let mut file = std::fs::File::open(&self.image).map_err(|e| fail(&e))?;
        let n = file.read(&mut head).map_err(|e| fail(&e))?;
        if n >= 24 && head.starts_with(b"\x89PNG\r\n\x1a\n") && &head[12..16] == b"IHDR" {
            let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
            let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
            return if w > 0 && h > 0 { Ok((w, h)) } else { Err(fail(&"the PNG has no pixels")) };
        }
        let bytes = read_capped(&self.image)?;
        craft_image_codecs::probe_size(&bytes).map_err(|e| fail(&e))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub name: String,
    pub samples: Vec<Sample>,
    /// Rows or files the loader could not use, each with the reason (at most `MAX_MESSAGES`).
    pub skipped: Vec<String>,
    /// How many were skipped in all.
    pub skipped_count: usize,
}

impl Dataset {
    pub fn new(name: &str) -> Dataset {
        Dataset { name: name.to_string(), samples: Vec::new(), skipped: Vec::new(), skipped_count: 0 }
    }

    pub fn skip(&mut self, why: String) {
        self.skipped_count += 1;
        if self.skipped.len() < MAX_MESSAGES {
            self.skipped.push(why);
        }
    }
}

pub(crate) fn scene(msg: impl Into<String>) -> CoreError {
    CoreError::Scene(msg.into())
}

/// Reads a file whole, but refuses one larger than `MAX_FILE_BYTES`.
pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, CoreError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| scene(format!("{}: {e}", path.display())))?;
    let mut out = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut out).map_err(|e| scene(format!("{}: {e}", path.display())))?;
    if out.len() as u64 > MAX_FILE_BYTES {
        return Err(scene(format!("{} is larger than {MAX_FILE_BYTES} bytes", path.display())));
    }
    Ok(out)
}
