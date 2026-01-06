//! # Argos I/O
//!
//! I/O infrastructure implementations for the Argos forensic image recovery tool.
//!
//! This crate provides concrete implementations of the `BlockSource` trait
//! defined in `argos_core`, allowing the forensic scanner to read raw block data
//! from physical disks, disk images, and other block devices.
//!
//! ## Key Components
//!
//! - **DiskReader**: Read-only block source using standard file I/O
//! - **MmapReader**: Zero-copy block source using memory mapping (preferred)
//!
//! ## Recommended Usage
//!
//! Use [`create_reader`] to automatically select the best reader:
//!
//! ```ignore
//! use argos_io::create_reader;
//!
//! // Automatically tries mmap, falls back to DiskReader
//! let mut reader = create_reader("/dev/sda")?;
//! let mut buffer = vec![0u8; 512];
//! let bytes_read = reader.read_chunk(0, &mut buffer)?;
//! ```

mod mmap_reader;
mod reader;

pub use mmap_reader::MmapReader;
pub use reader::DiskReader;

use argos_core::{BlockSource, Result};
use std::path::Path;

/// Creates the optimal reader for the given path.
///
/// Attempts to use memory-mapped I/O first for zero-copy performance.
/// Falls back to standard file I/O if mmap fails (e.g., on block devices
/// that don't support mmap, or on systems with limited virtual address space).
///
/// # Arguments
///
/// * `path` - Path to the disk device or image file
///
/// # Returns
///
/// A boxed `BlockSource` using either `MmapReader` or `DiskReader`.
///
/// # Example
///
/// ```ignore
/// use argos_io::create_reader;
/// use argos_core::BlockSource;
///
/// let mut reader = create_reader("/path/to/disk.img")?;
/// println!("Source size: {} bytes", reader.size());
/// ```
pub fn create_reader(path: impl AsRef<Path>) -> Result<Box<dyn BlockSource>> {
    let path_ref = path.as_ref();

    // Try mmap first for best performance
    match MmapReader::new(path_ref) {
        Ok(r) => Ok(Box::new(r)),
        Err(_) => {
            // Fall back to standard I/O for block devices or when mmap fails
            Ok(Box::new(DiskReader::new(path_ref)?))
        }
    }
}

/// Creates a reader, returning specific type information about which was used.
///
/// Useful when you need to access `MmapReader`-specific methods like `slice()`.
pub enum Reader {
    Mmap(MmapReader),
    Disk(DiskReader),
}

impl Reader {
    /// Creates a Reader, preferring mmap when available.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path_ref = path.as_ref();
        match MmapReader::new(path_ref) {
            Ok(r) => Ok(Reader::Mmap(r)),
            Err(_) => Ok(Reader::Disk(DiskReader::new(path_ref)?)),
        }
    }

    /// Returns true if this is a memory-mapped reader.
    #[inline]
    pub fn is_mmap(&self) -> bool {
        matches!(self, Reader::Mmap(_))
    }
}

impl BlockSource for Reader {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        match self {
            Reader::Mmap(r) => r.read_chunk(offset, buffer),
            Reader::Disk(r) => r.read_chunk(offset, buffer),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Reader::Mmap(r) => r.size(),
            Reader::Disk(r) => r.size(),
        }
    }
}
