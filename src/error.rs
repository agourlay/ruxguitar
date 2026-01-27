//! Error types for the ruxguitar library

use std::io;

/// Library error type for ruxguitar operations
#[derive(Debug, thiserror::Error)]
pub enum RuxError {
    /// Parsing error when reading Guitar Pro files
    #[error("parsing error: {0}")]
    ParsingError(String),

    /// Configuration error
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// Audio-related error
    #[error("audio error: {0}")]
    AudioError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(String),
}

impl From<io::Error> for RuxError {
    fn from(error: io::Error) -> Self {
        Self::IoError(error.to_string())
    }
}
