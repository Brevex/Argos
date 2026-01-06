//! Memory-mapped block source for zero-copy disk access.
//!
//! Uses mmap for optimal I/O performance on sequential scans.
//! Falls back to `DiskReader` for block devices that don't support mmap.

use argos_core::{BlockSource, CoreError, Result};
use memmap2::Mmap;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::Path;

/// Zero-copy block source using memory mapping.
///
/// Eliminates user-kernel copy overhead by mapping the file directly
/// into the process address space. The kernel handles prefetching
/// and page management automatically.
///
/// # When to Use
///
/// - Disk image files: Always works, best performance
/// - Block devices: May fail; caller should fall back to `DiskReader`
///
/// # Safety
///
/// Uses `memmap2::Mmap` which is safe when the underlying file is not
/// modified during the mapping lifetime. For forensic read-only analysis,
/// this invariant is guaranteed.
///
/// # Example
///
/// ```ignore
/// use argos_io::MmapReader;
/// use argos_core::BlockSource;
///
/// // Try mmap first, fall back to DiskReader if it fails
/// let reader = MmapReader::new("/path/to/disk.img")?;
/// let slice = reader.slice(0, 4096).unwrap();
/// ```
pub struct MmapReader {
    mmap: Mmap,
    size: u64,
}

impl MmapReader {
    /// Creates a new memory-mapped reader.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the disk image or device
    ///
    /// # Returns
    ///
    /// `Err` if mmap fails (e.g., block device doesn't support it).
    /// In that case, caller should fall back to `DiskReader`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;

        // Get file size by seeking to end
        let size = file.seek(SeekFrom::End(0))?;

        if size == 0 {
            return Err(CoreError::InvalidFormat(
                "Cannot mmap empty file".to_string(),
            ));
        }

        // SAFETY: We only read from the mmap, never write.
        // The file should not be modified during forensic analysis.
        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| CoreError::Io(std::io::Error::other(e)))?;

        // CRITICAL: Validate mmap actually worked!
        // Block devices often "succeed" at mmap but return empty mapping.
        // This check ensures we fall back to DiskReader for such cases.
        if mmap.len() == 0 {
            return Err(CoreError::InvalidFormat(
                "mmap returned empty mapping (block device not supported)".to_string(),
            ));
        }

        // Advise kernel about sequential access pattern
        #[cfg(target_os = "linux")]
        {
            use memmap2::Advice;
            let _ = mmap.advise(Advice::Sequential);
            let _ = mmap.advise(Advice::WillNeed);
        }

        Ok(Self { mmap, size })
    }

    /// Returns a zero-copy slice of the memory-mapped region.
    ///
    /// This is the key optimization: **no data copy occurs**.
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset into the file
    /// * `len` - Maximum number of bytes to return
    ///
    /// # Returns
    ///
    /// `Some(&[u8])` with up to `len` bytes, or `None` if offset is past EOF.
    #[inline]
    pub fn slice(&self, offset: u64, len: usize) -> Option<&[u8]> {
        let start = offset as usize;
        if start >= self.mmap.len() {
            return None;
        }
        let end = start.saturating_add(len).min(self.mmap.len());
        Some(&self.mmap[start..end])
    }

    /// Prefetch a region to reduce page fault latency.
    ///
    /// Hints to the kernel that we'll access this region soon.
    /// Non-blocking and best-effort.
    #[cfg(target_os = "linux")]
    pub fn prefetch(&self, offset: u64, len: usize) {
        let start = offset as usize;
        if start >= self.mmap.len() {
            return;
        }
        let end = start.saturating_add(len).min(self.mmap.len());
        let _ = self
            .mmap
            .advise_range(memmap2::Advice::WillNeed, start, end - start);
    }

    /// Non-Linux stub for prefetch (no-op).
    #[cfg(not(target_os = "linux"))]
    pub fn prefetch(&self, _offset: u64, _len: usize) {}
}

impl BlockSource for MmapReader {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(slice) = self.slice(offset, buffer.len()) {
            let len = slice.len();
            buffer[..len].copy_from_slice(slice);
            Ok(len)
        } else {
            Ok(0)
        }
    }

    fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_mmap_reader_basic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, World! This is test data for MmapReader.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        assert_eq!(reader.size(), test_data.len() as u64);

        // Test zero-copy slice
        let slice = reader.slice(0, 13).unwrap();
        assert_eq!(slice, b"Hello, World!");

        let slice = reader.slice(7, 4).unwrap();
        assert_eq!(slice, b"Worl");
    }

    #[test]
    fn test_mmap_reader_block_source_trait() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Test data for BlockSource trait.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let mut reader = MmapReader::new(temp_file.path()).unwrap();

        let mut buffer = vec![0u8; 9];
        let bytes_read = reader.read_chunk(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 9);
        assert_eq!(&buffer, b"Test data");
    }

    #[test]
    fn test_mmap_reader_beyond_eof() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Short").unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        // Slice at valid offset but requesting more than available
        let slice = reader.slice(0, 100).unwrap();
        assert_eq!(slice.len(), 5);

        // Slice completely past EOF
        let slice = reader.slice(100, 10);
        assert!(slice.is_none());
    }

    #[test]
    fn test_mmap_reader_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let result = MmapReader::new(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_prefetch_does_not_panic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Data for prefetch test").unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        // Should not panic even with out-of-bounds offset
        reader.prefetch(0, 1024);
        reader.prefetch(1000, 1024);
    }
}
