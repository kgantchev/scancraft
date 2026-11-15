//! A11: true proportions and page size (specification A11). Owner: WP1.

use crate::GeomError;
use scancraft_core::camera::Camera;
use scancraft_core::geom::Quad;
use scancraft_core::recipe::{FocalInput, Paper, Proportions};
use scancraft_core::{Step, not_implemented};

/// What A11 needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProportionsInput {
    /// Refined corners in photo pixels.
    pub corners: Quad,
    /// Photo size in pixels.
    pub photo: (u32, u32),
    pub focal: FocalInput,
    /// Snap to standard paper sizes (Settings, on by default).
    pub snap: bool,
    /// The default paper of Settings, for unsnapped pages and the ISO A series.
    pub default_paper: Paper,
}

/// What A11 gives back.
#[derive(Debug, Clone, PartialEq)]
pub struct ProportionsOutput {
    pub proportions: Proportions,
    /// The camera A11 settled on (step 3), which A27 starts from.
    pub camera: Camera,
    /// Physical size in millimetres (step 6).
    pub page_size_mm: (f64, f64),
}

/// A11 steps 1–7.
pub fn true_proportions(input: &ProportionsInput) -> Result<ProportionsOutput, GeomError> {
    let _ = input;
    not_implemented(Step::A11)
}
