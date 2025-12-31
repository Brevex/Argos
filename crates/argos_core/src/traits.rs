//! Core traits defining the interfaces for block sources and file scanners.
//!
//! These traits follow the Ports & Adapters pattern to ensure decoupling
//! between the core domain logic and the infrastructure implementations.

use crate::error::Result;
use crate::types::FileType;

/// A source of raw block data, typically a disk or image file.
///
/// This trait abstracts away the underlying storage medium, allowing
/// the same scanning logic to work on physical disks, disk images,
/// or any other block-based data source.
///
/// # Example
///
/// ```ignore
/// struct DiskDevice { /* ... */ }
///
/// impl BlockSource for DiskDevice {
///     fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
///         // Read from physical disk at offset
///     }
///     
///     fn size(&self) -> u64 {
///         // Return total size in bytes
///     }
/// }
/// ```
pub trait BlockSource {
    /// Reads a chunk of data from the source at the specified offset.
    ///
    /// # Arguments
    ///
    /// * `offset` - The byte offset to start reading from
    /// * `buffer` - The buffer to read data into
    ///
    /// # Returns
    ///
    /// The number of bytes actually read, which may be less than `buffer.len()`
    /// if the end of the source is reached.
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    /// Returns the total size of the source in bytes.
    fn size(&self) -> u64;
}

/// File signature detection for forensic recovery.
///
/// Implementations of this trait scan byte buffers to find file headers
/// (magic bytes) and footers for specific file formats like JPEG, PNG, etc.
///
/// This trait is designed for high-performance scanning using SIMD-accelerated
/// pattern matching (via `memchr`).
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to support parallel scanning.
///
/// # Example
///
/// ```ignore
/// use argos_core::{FileScanner, FileType};
///
/// struct MyScanner;
///
/// impl FileScanner for MyScanner {
///     fn scan_headers(&self, buffer: &[u8]) -> Vec<usize> {
///         // Use memchr to find all header offsets
///         vec![]
///     }
///     
///     fn scan_footers(&self, buffer: &[u8]) -> Vec<usize> {
///         // Use memchr to find all footer offsets
///         vec![]
///     }
///     
///     fn file_type(&self) -> FileType {
///         FileType::Unknown
///     }
/// }
/// ```
pub trait FileScanner: Send + Sync {
    /// Scans the buffer and returns all offsets where START signatures were found.
    ///
    /// # Arguments
    ///
    /// * `buffer` - A slice of bytes to scan for header signatures
    ///
    /// # Returns
    ///
    /// A vector of byte offsets where headers begin within the buffer.
    fn scan_headers(&self, buffer: &[u8]) -> Vec<usize>;

    /// Scans the buffer and returns all offsets where END signatures were found.
    ///
    /// # Arguments
    ///
    /// * `buffer` - A slice of bytes to scan for footer signatures
    ///
    /// # Returns
    ///
    /// A vector of byte offsets where footers begin within the buffer.
    fn scan_footers(&self, buffer: &[u8]) -> Vec<usize>;

    /// Returns the file type this scanner detects.
    ///
    /// # Returns
    ///
    /// A `FileType` enum variant like `FileType::Jpeg`, `FileType::Png`, etc.
    fn file_type(&self) -> FileType;
}
