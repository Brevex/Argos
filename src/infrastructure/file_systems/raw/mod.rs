//! Raw parser (file carving only)
//!
//! This parser doesn't parse any filesystem metadata - it's used
//! for pure file carving on raw devices or formatted disks.

use crate::domain::repositories::{
    DeletedFileEntry, FileSystemError, FileSystemParser, FileSystemType,
};

/// Raw parser for file carving without filesystem metadata
///
/// This is used when:
/// - The filesystem is unknown or unsupported
/// - The device has been formatted (metadata destroyed)
/// - Pure file carving is preferred
pub struct RawParser {
    /// Device size in bytes (reserved for future use in size-based heuristics)
    #[allow(dead_code)]
    device_size: u64,
}

impl RawParser {
    /// Creates a new raw parser
    pub fn new(device_size: u64) -> Self {
        Self { device_size }
    }
}

impl FileSystemParser for RawParser {
    fn detect_type(&self) -> Result<FileSystemType, FileSystemError> {
        Ok(FileSystemType::Raw)
    }

    fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError> {
        // Raw parser doesn't have any metadata to parse
        // File carving will be done directly on the raw data
        Ok(Vec::new())
    }

    fn read_deleted_data(&self, _entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError> {
        Err(FileSystemError::Other(
            "Raw parser does not support metadata-based recovery".to_string(),
        ))
    }

    fn filesystem_type(&self) -> FileSystemType {
        FileSystemType::Raw
    }

    fn is_healthy(&self) -> bool {
        true // Raw is always "healthy"
    }
}
