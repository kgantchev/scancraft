//! Errors shared by every crate, and the names of the specification's algorithm steps.

use std::fmt;

/// One algorithm of the specification, A1–A30 (§8). Used to say which step is unfinished or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Step {
    A1,
    A2,
    A3,
    A4,
    A5,
    A6,
    A7,
    A8,
    A9,
    A10,
    A11,
    A12,
    A13,
    A14,
    A15,
    A16,
    A17,
    A18,
    A19,
    A20,
    A21,
    A22,
    A23,
    A24,
    A25,
    A26,
    A27,
    A28,
    A29,
    A30,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Errors from the shared types themselves: sizes, buffers and parameters out of range.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    /// An algorithm step whose work package hasn't landed yet (build plan §2 rule 4).
    #[error("{0} is not implemented yet")]
    NotImplemented(Step),
    /// An image size of zero, above the limits of specification §9, or whose pixel count overflows.
    #[error("image size {width} × {height} is not allowed")]
    BadSize { width: u32, height: u32 },
    /// A pixel buffer whose length doesn't match its size.
    #[error("buffer holds {len} values, {expected} expected")]
    BadBuffer { len: usize, expected: usize },
    /// A parameter outside its allowed range, with the parameter's name.
    #[error("parameter {name} out of range: {value}")]
    OutOfRange { name: &'static str, value: f64 },
    /// A file in the scene or fixture format that couldn't be read.
    #[error("scene file: {0}")]
    Scene(String),
}

/// The body of every stub (build plan §2 rule 4): `return not_implemented(Step::A16);`.
pub fn not_implemented<T, E: From<CoreError>>(step: Step) -> Result<T, E> {
    Err(E::from(CoreError::NotImplemented(step)))
}
