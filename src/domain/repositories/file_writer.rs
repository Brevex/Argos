//! File writer trait
//!
//! Defines the interface for writing recovered files to storage.

use crate::domain::entities::RecoveredFile;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when writing recovered files
#[derive(Error, Debug)]
pub enum FileWriterError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Directory not found: {0}")]
    DirectoryNotFound(String),

    #[error("File already exists: {0}")]
    FileExists(String),

    #[error("Disk full: {0}")]
    DiskFull(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Image conversion error: {0}")]
    ConversionError(String),

    #[error("Write error: {0}")]
    Other(String),
}

/// Options for writing recovered files
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Whether to overwrite existing files
    pub overwrite: bool,
    /// Whether to convert images to PNG format
    pub convert_to_png: bool,
    /// Whether to create subdirectories by file type
    pub organize_by_type: bool,
    /// Prefix for filenames
    pub filename_prefix: String,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            overwrite: false,
            convert_to_png: true, // User requested PNG output
            organize_by_type: true,
            filename_prefix: "recovered".to_string(),
        }
    }
}

/// Result of writing a single file
#[derive(Debug, Clone)]
pub struct WriteResult {
    /// Original file ID
    pub file_id: u64,
    /// Path where the file was saved
    pub saved_path: PathBuf,
    /// Size in bytes of the saved file
    pub saved_size: u64,
    /// Whether the file was converted (e.g., JPEG to PNG)
    pub was_converted: bool,
}

/// Trait for writing recovered files to storage
///
/// This trait abstracts the file writing process, allowing for
/// different output strategies (local files, network storage, etc.)
///
/// # Example
///
/// ```ignore
/// let writer = LocalFileWriter::new("/output/recovered")?;
/// let options = WriteOptions::default();
/// let result = writer.write(&recovered_file, &options)?;
/// println!("Saved to: {}", result.saved_path.display());
/// ```
pub trait RecoveredFileWriter: Send + Sync {
    /// Creates a new writer for the specified output directory
    fn new(output_dir: &Path) -> Result<Self, FileWriterError>
    where
        Self: Sized;

    /// Writes a recovered file to storage
    fn write(
        &self,
        file: &RecoveredFile,
        options: &WriteOptions,
    ) -> Result<WriteResult, FileWriterError>;

    /// Writes multiple recovered files
    fn write_batch(
        &self,
        files: &[RecoveredFile],
        options: &WriteOptions,
    ) -> Vec<Result<WriteResult, FileWriterError>> {
        files.iter().map(|f| self.write(f, options)).collect()
    }

    /// Returns the output directory
    fn output_dir(&self) -> &Path;

    /// Returns the number of files written so far
    fn files_written(&self) -> usize;

    /// Returns the total bytes written so far
    fn bytes_written(&self) -> u64;
}
