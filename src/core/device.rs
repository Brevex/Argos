use super::error::BlockDeviceError;
use super::io::{BlockDeviceReader, DeviceInfo};
use memmap2::Mmap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// Linux block device reader implementation
pub struct LinuxBlockDevice {
    file: Mutex<File>,
    path: String,
    size: u64,
    block_size: u32,
}

impl LinuxBlockDevice {
    fn detect_block_size(path: &Path) -> u32 {
        if path.starts_with("/dev/") {
            4096
        } else {
            512
        }
    }

    fn get_size(file: &File, path: &Path) -> Result<u64, BlockDeviceError> {
        let metadata = file.metadata().map_err(BlockDeviceError::IoError)?;
        if metadata.is_file() {
            Ok(metadata.len())
        } else {
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

        let mut file = self
            .file
            .lock()
            .map_err(|_| BlockDeviceError::Other("Failed to acquire lock".to_string()))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(BlockDeviceError::IoError)?;

        let available = (self.size - offset) as usize;
        let to_read = length.min(available);
        let mut buffer = vec![0u8; to_read];
        file.read_exact(&mut buffer).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
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

/// Memory-mapped block device reader implementation
pub struct MmapBlockDevice {
    mmap: Mmap,
    path: String,
    size: u64,
    block_size: u32,
}

impl MmapBlockDevice {
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

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

        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| BlockDeviceError::Other(format!("Failed to memory-map file: {}", e)))?;
        let block_size = if path.starts_with("/dev/") { 4096 } else { 512 };

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
