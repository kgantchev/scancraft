//! The traits that plug platform services into the portable pipeline (specification §6.1, §6.7).
//! `scancraft-apple` implements them with Vision; tests use recordings or fakes.

use crate::geom::Quad;
use crate::image::{Gray8, Rgb8};
use crate::recipe::RecognizedText;

/// Why a platform service gave no answer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ServiceError {
    #[error("service unavailable on this system")]
    Unavailable,
    #[error("timed out after {0} ms")]
    TimedOut(u64),
    #[error("service failed: {0}")]
    Failed(String),
}

/// A8: finds a document's four corners in the work image (long side 1024 px). `Ok(None)` means
/// the service ran and found nothing.
pub trait PageDetector: Send + Sync {
    fn detect(&self, work_image: &Rgb8) -> Result<Option<Quad>, ServiceError>;
}

/// A15's vote for each of the four rotations, 0°, 90°, 180° and 270° clockwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UprightVote {
    /// The sum over results of confidence × character count (A15 step 2).
    pub score: [f32; 4],
    /// Characters read at each rotation.
    pub chars: [u32; 4],
}

/// A15: scores how readable a 1024 px Grayscale page is at each rotation.
pub trait UprightVoter: Send + Sync {
    fn vote(&self, page: &Gray8) -> Result<UprightVote, ServiceError>;
}

/// What A20 is asked to read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecognizeOptions {
    /// Empty for automatic language detection.
    pub languages: Vec<String>,
    pub language_correction: bool,
    /// Minimum text height as a fraction of the page's height (A20 step 2).
    pub minimum_text_height: f32,
}

/// A20: recognises the text of a page (the Grayscale look at default strength, output DPI).
pub trait Recognizer: Send + Sync {
    /// The engine's name, as stored in `RecognizedText::engine`.
    fn name(&self) -> &str;
    fn recognize(&self, page: &Gray8, options: &RecognizeOptions) -> Result<RecognizedText, ServiceError>;
}
