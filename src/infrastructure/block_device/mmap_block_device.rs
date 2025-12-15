//! Memory-mapped block device implementation
//!
//! Provides efficient read access to block devices using memory-mapped I/O.
//! This implementation is faster than standard file I/O for sequential reads
//! and allows concurrent access without mutex contention.

use crate::domain::repositories::{BlockDeviceError, BlockDeviceReader, DeviceInfo};
use memmap2::Mmap;
use std::fs::OpenOptions;
use std::path::Path;

/// Memory-mapped block device reader implementation
///
/// Uses memory-mapped I/O for efficient, zero-copy access to device data.
/// This is faster than traditional read() calls and allows the kernel
/// to optimize page caching.
///
/// # Example
///
/// ```ignore
/// let device = MmapBlockDevice::open("/dev/sda")?;
/// let data = device.read_at(0, 512)?;
/// ```
pub struct MmapBlockDevice {
    mmap: Mmap,
    path: String,
    size: u64,
    block_size: u32,
}

impl MmapBlockDevice {
    /// Attempts to detect the block size of a device
    fn detect_block_size(path: &Path) -> u32 {
        if path.starts_with("/dev/") {
            4096 // Modern devices typically use 4K blocks
        } else {
            512 // Image files use logical sector size
        }
    }

    /// Returns a slice of the memory-mapped data
    ///
    /// This is a zero-copy operation and very fast.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

    /// Returns a slice at the specified offset and length
    ///
    /// This is a zero-copy operation.
    #[inline]
    pub fn slice_at(&self, offset: u64, length: usize) -> Option<&[u8]> {
        let start = offset as usize;
        let end = start.checked_add(length)?;
        if end <= self.mmap.len() {
            Some(&self.mmap[start..end])
        } else {
            None
        }
    }
}

impl BlockDeviceReader for MmapBlockDevice {
    fn open(path: &str) -> Result<Self, BlockDeviceError> {
        let path_obj = Path::new(path);

        if !path_obj.exists() {
            return Err(BlockDeviceError::DeviceNotFound(path.to_string()));
        }

        let file = OpenOptions::new().read(true).open(path_obj).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                BlockDeviceError::PermissionDenied(format!("{} - try running with sudo", path))
            } else {
                BlockDeviceError::IoError(e)
            }
        })?;

        let metadata = file.metadata().map_err(BlockDeviceError::IoError)?;
        let size = metadata.len();

        if size == 0 {
            return Err(BlockDeviceError::Other(format!(
                "File {} has zero size",
                path
            )));
        }

        // Create memory mapping
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
            BlockDeviceError::Other(format!("Failed to memory-map file: {}", e))
        })?;

        let block_size = Self::detect_block_size(path_obj);

        Ok(Self {
            mmap,
            path: path.to_string(),
            size,
            block_size,
        })
    }

    fn device_info(&self) -> Result<DeviceInfo, BlockDeviceError> {
        Ok(DeviceInfo {
            path: self.path.clone(),
            size: self.size,
            block_size: self.block_size,
            read_only: true,
            model: None,
            serial: None,
        })
    }

    fn read_at(&self, offset: u64, length: usize) -> Result<Vec<u8>, BlockDeviceError> {
        if offset >= self.size {
            return Err(BlockDeviceError::InvalidOffset {
                offset,
                device_size: self.size,
            });
        }

        let available = (self.size - offset) as usize;
        let to_read = length.min(available);
        let start = offset as usize;
        let end = start + to_read;

        Ok(self.mmap[start..end].to_vec())
    }

    fn read_chunks<F>(
        &self,
        start_offset: u64,
        chunk_size: usize,
        mut callback: F,
    ) -> Result<u64, BlockDeviceError>
    where
        F: FnMut(u64, &[u8]) -> bool,
    {
        let mut offset = start_offset as usize;
        let mut total_read = 0u64;
        let data_len = self.mmap.len();

        while offset < data_len {
            let remaining = data_len - offset;
            let chunk_len = chunk_size.min(remaining);
            let chunk = &self.mmap[offset..offset + chunk_len];

            if !callback(offset as u64, chunk) {
                break;
            }

            total_read += chunk_len as u64;
            offset += chunk_len;
        }

        Ok(total_read)
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn size(&self) -> u64 {
        self.size
    }
}

// Mmap is Send + Sync when the underlying file is read-only
unsafe impl Send for MmapBlockDevice {}
unsafe impl Sync for MmapBlockDevice {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_mmap_open_nonexistent() {
        let result = MmapBlockDevice::open("/nonexistent/file");
        assert!(result.is_err());
    }

    #[test]
    fn test_mmap_read_at() {
        let mut file = NamedTempFile::new().unwrap();
        let data = b"Hello, memory-mapped world!";
        file.write_all(data).unwrap();
        file.flush().unwrap();

        let device = MmapBlockDevice::open(file.path().to_str().unwrap()).unwrap();
        let read_data = device.read_at(0, 5).unwrap();
        assert_eq!(&read_data, b"Hello");
    }

    #[test]
    fn test_mmap_slice_at() {
        let mut file = NamedTempFile::new().unwrap();
        let data = b"Zero-copy access!";
        file.write_all(data).unwrap();
        file.flush().unwrap();

        let device = MmapBlockDevice::open(file.path().to_str().unwrap()).unwrap();
        let slice = device.slice_at(5, 4).unwrap();
        assert_eq!(slice, b"copy");
    }

    #[test]
    fn test_mmap_read_chunks() {
        let mut file = NamedTempFile::new().unwrap();
        let data = vec![0u8; 1024];
        file.write_all(&data).unwrap();
        file.flush().unwrap();

        let device = MmapBlockDevice::open(file.path().to_str().unwrap()).unwrap();
        let mut chunks_read = 0;
        device
            .read_chunks(0, 256, |_, _| {
                chunks_read += 1;
                true
            })
            .unwrap();
        assert_eq!(chunks_read, 4);
    }
}
