//! Linux block device implementation
//!
//! Provides raw read access to block devices on Linux systems.
//! Supports both actual devices (/dev/sdX) and image files.

use crate::domain::repositories::{BlockDeviceError, BlockDeviceReader, DeviceInfo};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// Linux block device reader implementation
///
/// Provides read access to block devices and disk images on Linux.
/// This implementation uses standard file I/O for broad compatibility.
///
/// # Example
///
/// ```ignore
/// let device = LinuxBlockDevice::open("/dev/sda")?;
/// let data = device.read_at(0, 512)?;
/// ```
pub struct LinuxBlockDevice {
    file: Mutex<File>,
    path: String,
    size: u64,
    block_size: u32,
}

impl LinuxBlockDevice {
    /// Attempts to detect the block size of a device
    fn detect_block_size(path: &Path) -> u32 {
        // For regular files, use 512 bytes (common sector size)
        // In a full implementation, we'd use ioctl for real block devices
        if path.starts_with("/dev/") {
            4096 // Modern devices typically use 4K blocks
        } else {
            512 // Image files use logical sector size
        }
    }

    /// Gets the device/file size
    fn get_size(file: &File, path: &Path) -> Result<u64, BlockDeviceError> {
        let metadata = file.metadata().map_err(BlockDeviceError::IoError)?;

        if metadata.is_file() {
            Ok(metadata.len())
        } else {
            // For block devices, we need to seek to the end
            // This is a simplified approach; production code would use ioctl
            let mut f = file.try_clone().map_err(BlockDeviceError::IoError)?;
            let size = f
                .seek(SeekFrom::End(0))
                .map_err(BlockDeviceError::IoError)?;
            f.seek(SeekFrom::Start(0))
                .map_err(BlockDeviceError::IoError)?;

            if size == 0 {
                Err(BlockDeviceError::Other(format!(
                    "Could not determine size of {}",
                    path.display()
                )))
            } else {
                Ok(size)
            }
        }
    }
}

impl BlockDeviceReader for LinuxBlockDevice {
    fn open(path: &str) -> Result<Self, BlockDeviceError> {
        let path_obj = Path::new(path);

        // Check if path exists
        if !path_obj.exists() {
            return Err(BlockDeviceError::DeviceNotFound(path.to_string()));
        }

        // Open file for reading
        let file = OpenOptions::new().read(true).open(path_obj).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                BlockDeviceError::PermissionDenied(format!("{} - try running with sudo", path))
            } else {
                BlockDeviceError::IoError(e)
            }
        })?;

        let size = Self::get_size(&file, path_obj)?;
        let block_size = Self::detect_block_size(path_obj);

        Ok(Self {
            file: Mutex::new(file),
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
            read_only: true, // We only open for reading
            model: None,     // Would require ioctl for real devices
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

        let mut file = self
            .file
            .lock()
            .map_err(|_| BlockDeviceError::Other("Failed to acquire lock".to_string()))?;

        file.seek(SeekFrom::Start(offset))
            .map_err(BlockDeviceError::IoError)?;

        // Limit read to available data
        let available = (self.size - offset) as usize;
        let to_read = length.min(available);

        let mut buffer = vec![0u8; to_read];
        file.read_exact(&mut buffer).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                // Return partial data if we hit EOF
                BlockDeviceError::ReadError {
                    offset,
                    message: "Unexpected end of device".to_string(),
                }
            } else {
                BlockDeviceError::IoError(e)
            }
        })?;

        Ok(buffer)
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
        let mut offset = start_offset;
        let mut total_read = 0u64;

        while offset < self.size {
            let data = self.read_at(offset, chunk_size)?;
            let bytes_read = data.len() as u64;

            if bytes_read == 0 {
                break;
            }

            // Call the callback; if it returns false, stop reading
            if !callback(offset, &data) {
                break;
            }

            total_read += bytes_read;
            offset += bytes_read;
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

// Ensure LinuxBlockDevice is Send + Sync as required by the trait
unsafe impl Send for LinuxBlockDevice {}
unsafe impl Sync for LinuxBlockDevice {}
