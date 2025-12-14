//! File carver trait
//!
//! Defines the interface for carving files from raw data.

use crate::domain::entities::{FileType, RecoveredFile, SignatureMatch};
use thiserror::Error;

/// Errors that can occur during file carving
#[derive(Error, Debug)]
pub enum CarverError {
    #[error("Invalid file data: {0}")]
    InvalidData(String),

    #[error("File appears corrupted: {0}")]
    CorruptedFile(String),

    #[error("Unsupported file type: {0}")]
    UnsupportedType(String),

    #[error("Carving error: {0}")]
    Other(String),
}

/// Trait for carving files from raw data
///
/// File carvers extract complete files from raw byte data using
/// file signatures and format-specific parsing.
///
/// # Example
///
/// ```ignore
/// let carver = ImageCarver::new();
/// let recovered = carver.carve(&raw_data, &signature_match)?;
/// println!("Recovered {} bytes", recovered.size());
/// ```
pub trait FileCarver: Send + Sync {
    /// Returns the file types this carver supports
    fn supported_types(&self) -> &[FileType];

    /// Checks if this carver supports the given file type
    fn supports(&self, file_type: FileType) -> bool {
        self.supported_types().contains(&file_type)
    }

    /// Carves a file from raw data based on a signature match
    ///
    /// # Arguments
    ///
    /// * `data` - Raw data starting at the file's beginning
    /// * `match_info` - Information about the signature match
    /// * `file_id` - Unique ID to assign to the recovered file
    ///
    /// # Returns
    ///
    /// The recovered file, or an error if carving failed
    fn carve(
        &self,
        data: &[u8],
        match_info: &SignatureMatch,
        file_id: u64,
    ) -> Result<RecoveredFile, CarverError>;

    /// Attempts to determine the exact file size
    ///
    /// Some formats (like PNG, JPEG) have markers or headers that
    /// indicate the file size. This method attempts to find the
    /// exact size without carving the entire file.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw data starting at the file's beginning
    /// * `file_type` - The type of file to analyze
    ///
    /// # Returns
    ///
    /// The exact file size, or None if it cannot be determined
    fn determine_file_size(&self, data: &[u8], file_type: FileType) -> Option<u64>;

    /// Validates a carved file
    ///
    /// Performs format-specific validation to check if the recovered
    /// file is complete and valid.
    ///
    /// # Arguments
    ///
    /// * `data` - The carved file data
    /// * `file_type` - The expected file type
    ///
    /// # Returns
    ///
    /// True if the file appears valid
    fn validate(&self, data: &[u8], file_type: FileType) -> bool;
}
