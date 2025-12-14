//! Block device reader trait
//!
//! Defines the interface for reading raw data from block devices.
//! This abstraction allows the domain to work with any storage medium.

use std::io;
use thiserror::Error;

/// Errors that can occur when reading from a block device
#[derive(Error, Debug)]
pub enum BlockDeviceError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Device is busy: {0}")]
    DeviceBusy(String),

    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    #[error("Invalid offset: {offset} exceeds device size {device_size}")]
    InvalidOffset { offset: u64, device_size: u64 },

    #[error("Read error at offset {offset}: {message}")]
    ReadError { offset: u64, message: String },

    #[error("Device error: {0}")]
    Other(String),
}

/// Information about a block device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Path to the device (e.g., /dev/sda)
    pub path: String,
    /// Total size in bytes
    pub size: u64,
    /// Block size in bytes (typically 512 or 4096)
    pub block_size: u32,
    /// Whether the device is read-only
    pub read_only: bool,
    /// Device model name if available
    pub model: Option<String>,
    /// Device serial number if available
    pub serial: Option<String>,
}

impl DeviceInfo {
    /// Returns the number of blocks
    pub fn block_count(&self) -> u64 {
        self.size / self.block_size as u64
    }
}

/// Trait for reading raw data from block devices
///
/// This trait abstracts the low-level device access, allowing the domain
/// to remain platform-agnostic. Implementations can target Linux /dev/sdX,
/// Windows PhysicalDrive, or even disk image files.
///
/// # Example
///
/// ```ignore
/// // Implementation will handle platform-specific details
/// let reader = LinuxBlockDevice::open("/dev/sda")?;
/// let info = reader.device_info()?;
/// let data = reader.read_at(0, 4096)?;
/// ```
pub trait BlockDeviceReader: Send + Sync {
    /// Opens the device for reading
    fn open(path: &str) -> Result<Self, BlockDeviceError>
    where
        Self: Sized;

    /// Returns information about the device
    fn device_info(&self) -> Result<DeviceInfo, BlockDeviceError>;

    /// Reads data at the specified byte offset
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset to start reading from
    /// * `length` - Number of bytes to read
    ///
    /// # Returns
    ///
    /// A vector containing the read bytes
    fn read_at(&self, offset: u64, length: usize) -> Result<Vec<u8>, BlockDeviceError>;

    /// Reads data in chunks, calling the callback for each chunk
    ///
    /// This is more memory-efficient for scanning large devices.
    ///
    /// # Arguments
    ///
    /// * `start_offset` - Starting byte offset
    /// * `chunk_size` - Size of each chunk to read
    /// * `callback` - Function called with (offset, data) for each chunk
    ///
    /// # Returns
    ///
    /// The total number of bytes read
    fn read_chunks<F>(
        &self,
        start_offset: u64,
        chunk_size: usize,
        callback: F,
    ) -> Result<u64, BlockDeviceError>
    where
        F: FnMut(u64, &[u8]) -> bool;

    /// Returns the device path
    fn path(&self) -> &str;

    /// Returns the total size in bytes
    fn size(&self) -> u64;
}
