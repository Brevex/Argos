//! Core error types for Argos forensic tool.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// I/O operation failed
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid file format or signature
    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    /// Offset is out of bounds for the source
    #[error("Offset {offset} is out of bounds (max: {max})")]
    OutOfBounds { offset: u64, max: u64 },

    /// Buffer size is invalid for the operation
    #[error("Invalid buffer size: expected {expected}, got {actual}")]
    InvalidBufferSize { expected: usize, actual: usize },

    /// Device or file not found
    #[error("Not found: {0}")]
    NotFound(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
