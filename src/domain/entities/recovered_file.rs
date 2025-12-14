//! Recovered file entity
//!
//! Represents a file that has been successfully recovered from storage.

use super::file_signature::FileType;
use std::path::PathBuf;

/// Represents a recovered file with its metadata and content
#[derive(Debug, Clone)]
pub struct RecoveredFile {
    /// Unique identifier for this recovered file
    id: u64,
    /// The type of file
    file_type: FileType,
    /// Original offset in the source device
    source_offset: u64,
    /// Size of the recovered data in bytes
    size: u64,
    /// The raw recovered data
    data: Vec<u8>,
    /// Path where the file was saved (if saved)
    saved_path: Option<PathBuf>,
    /// Recovery confidence (0.0 - 1.0)
    confidence: f32,
    /// Whether the file appears to be corrupted
    is_corrupted: bool,
}

impl RecoveredFile {
    /// Creates a new recovered file
    pub fn new(
        id: u64,
        file_type: FileType,
        source_offset: u64,
        data: Vec<u8>,
        confidence: f32,
    ) -> Self {
        let size = data.len() as u64;
        Self {
            id,
            file_type,
            source_offset,
            size,
            data,
            saved_path: None,
            confidence: confidence.clamp(0.0, 1.0),
            is_corrupted: false,
        }
    }

    /// Returns the unique ID
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the file type
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the source offset
    pub fn source_offset(&self) -> u64 {
        self.source_offset
    }

    /// Returns the size in bytes
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the raw data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consumes self and returns the owned data
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Returns the saved path if the file was saved
    pub fn saved_path(&self) -> Option<&PathBuf> {
        self.saved_path.as_ref()
    }

    /// Sets the saved path
    pub fn set_saved_path(&mut self, path: PathBuf) {
        self.saved_path = Some(path);
    }

    /// Returns the confidence level
    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    /// Returns whether the file is corrupted
    pub fn is_corrupted(&self) -> bool {
        self.is_corrupted
    }

    /// Marks the file as corrupted
    pub fn mark_corrupted(&mut self) {
        self.is_corrupted = true;
    }

    /// Generates a suggested filename based on ID and type
    pub fn suggested_filename(&self) -> String {
        format!("recovered_{:06}.{}", self.id, self.file_type.extension())
    }

    /// Returns a human-readable size string
    pub fn size_human(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if self.size >= GB {
            format!("{:.2} GB", self.size as f64 / GB as f64)
        } else if self.size >= MB {
            format!("{:.2} MB", self.size as f64 / MB as f64)
        } else if self.size >= KB {
            format!("{:.2} KB", self.size as f64 / KB as f64)
        } else {
            format!("{} bytes", self.size)
        }
    }
}
