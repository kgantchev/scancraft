//! A15: upright, Vision's vote first, then the ink-profile fallback ported from PdfCraft's
//! `detect_orientation` (specification A15, §5.3).

use crate::EnhanceError;
use scancraft_core::image::Gray8;
use scancraft_core::traits::UprightVoter;
use scancraft_core::{Step, not_implemented};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UprightBy {
    Vision,
    Profiles,
    /// Unsure: the page stays as it is.
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upright {
    /// Clockwise quarter turns to apply.
    pub quarter_turns: u8,
    pub by: UprightBy,
}

/// `page` is a 1024 px Grayscale version of the page (A15 step 1).
pub fn upright(page: &Gray8, voter: Option<&dyn UprightVoter>) -> Result<Upright, EnhanceError> {
    let _ = (page, voter);
    not_implemented(Step::A15)
}
