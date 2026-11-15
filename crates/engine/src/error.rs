//! The engine's error type: every failure a command, a job or the session can report.

use scancraft_core::recipe::{PageId, PhotoId};
use scancraft_core::{CoreError, Step};
use scancraft_detect::DetectError;
use scancraft_enhance::EnhanceError;
use scancraft_geom::GeomError;
use scancraft_io::IoError;
use scancraft_raster::RasterError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Detect(#[from] DetectError),
    #[error(transparent)]
    Geom(#[from] GeomError),
    #[error(transparent)]
    Raster(#[from] RasterError),
    #[error(transparent)]
    Enhance(#[from] EnhanceError),
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("there is no page {}", .0.0)]
    NoPage(PageId),
    #[error("there is no photo {}", .0.0)]
    NoPhoto(PhotoId),
    #[error("{0}")]
    Fs(String),
    /// Work that was cancelled because the recipe changed under it; never shown.
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Message(String),
}

impl EngineError {
    pub fn msg(text: impl Into<String>) -> EngineError {
        EngineError::Message(text.into())
    }

    /// The algorithm step that no work package has finished yet, when that is why this failed.
    pub fn unfinished_step(&self) -> Option<Step> {
        let core = match self {
            EngineError::Core(e) => e,
            EngineError::Detect(DetectError::Core(e)) => e,
            EngineError::Geom(GeomError::Core(e)) => e,
            EngineError::Raster(RasterError::Core(e)) => e,
            EngineError::Enhance(EnhanceError::Core(e)) => e,
            EngineError::Io(IoError::Core(e)) => e,
            _ => return None,
        };
        match core {
            CoreError::NotImplemented(step) => Some(*step),
            _ => None,
        }
    }

    /// True when the failure is a step still to come: the engine says so in a notice and carries on.
    pub fn is_unfinished(&self) -> bool {
        self.unfinished_step().is_some()
    }
}

/// An `io::Error` as the file-system error it is.
pub fn fs_error(what: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::Fs(format!("{what}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfinished_step_is_recognised_through_every_wrapper() {
        let core = CoreError::NotImplemented(Step::A9);
        for e in [
            EngineError::Core(core.clone()),
            EngineError::Detect(DetectError::Core(core.clone())),
            EngineError::Geom(GeomError::Core(core.clone())),
            EngineError::Raster(RasterError::Core(core.clone())),
            EngineError::Enhance(EnhanceError::Core(core.clone())),
            EngineError::Io(IoError::Core(core.clone())),
        ] {
            assert_eq!(e.unfinished_step(), Some(Step::A9));
            assert!(e.is_unfinished());
        }
        assert!(!EngineError::msg("x").is_unfinished());
        assert!(!EngineError::Core(CoreError::Scene("x".into())).is_unfinished());
    }
}
