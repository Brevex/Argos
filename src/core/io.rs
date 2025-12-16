use super::error::BlockDeviceError;

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
pub trait BlockDeviceReader: Send + Sync {
    /// Opens the device for reading
    fn open(path: &str) -> Result<Self, BlockDeviceError>
    where
        Self: Sized;

    /// Returns information about the device
    fn device_info(&self) -> Result<DeviceInfo, BlockDeviceError>;

    /// Reads data at the specified byte offset
    fn read_at(&self, offset: u64, length: usize) -> Result<Vec<u8>, BlockDeviceError>;

    /// Reads data in chunks, calling the callback for each chunk
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
