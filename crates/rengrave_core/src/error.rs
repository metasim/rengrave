//! The error type returned by engine operations.

use std::path::PathBuf;

/// Everything that can stop a job.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file involved.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// A font could not be loaded.
    #[error("font error: {0}")]
    Font(String),
    /// An image or drawing could not be imported.
    #[error("import error: {0}")]
    Import(String),
    /// The settings failed validation; each entry describes one problem.
    #[error("invalid settings: {}", .0.join("; "))]
    InvalidSettings(Vec<String>),
    /// There is nothing to cut with the current input.
    #[error("{0}")]
    NothingToCut(String),
    /// The operation was canceled through its [`crate::Progress`] handle.
    #[error("calculation canceled")]
    Canceled,
}

/// Result alias for engine operations.
pub type Result<T> = std::result::Result<T, Error>;
