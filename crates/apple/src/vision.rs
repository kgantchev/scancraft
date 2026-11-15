//! The three Vision services. Stubs until WP5 lands.

use scancraft_core::geom::Quad;
use scancraft_core::image::{Gray8, Rgb8};
use scancraft_core::recipe::RecognizedText;
use scancraft_core::traits::{PageDetector, RecognizeOptions, Recognizer, ServiceError, UprightVote, UprightVoter};

fn unfinished(what: &str) -> ServiceError {
    ServiceError::Failed(format!("{what} is not implemented yet (WP5)"))
}

/// A8: `VNDetectDocumentSegmentationRequest`.
#[derive(Debug, Default)]
pub struct VisionPageDetector;

impl PageDetector for VisionPageDetector {
    fn detect(&self, work_image: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        let _ = work_image;
        Err(unfinished("A8"))
    }
}

/// A20: `VNRecognizeTextRequest`, Accurate.
#[derive(Debug, Default)]
pub struct VisionRecognizer;

impl Recognizer for VisionRecognizer {
    fn name(&self) -> &str {
        "vision"
    }
    fn recognize(&self, page: &Gray8, options: &RecognizeOptions) -> Result<RecognizedText, ServiceError> {
        let _ = (page, options);
        Err(unfinished("A20"))
    }
}

/// A15: `VNRecognizeTextRequest`, Fast, at four rotations.
#[derive(Debug, Default)]
pub struct VisionUprightVoter;

impl UprightVoter for VisionUprightVoter {
    fn vote(&self, page: &Gray8) -> Result<UprightVote, ServiceError> {
        let _ = page;
        Err(unfinished("A15"))
    }
}
