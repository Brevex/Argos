use super::error::BlockDeviceError;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: String,
    pub size: u64,
    pub block_size: u32,
    pub read_only: bool,
    pub model: Option<String>,
    pub serial: Option<String>,
}

impl DeviceInfo {
    pub fn block_count(&self) -> u64 {
        self.size / self.block_size as u64
    }
}

pub trait BlockDeviceReader: Send + Sync {
    fn open(path: &str) -> Result<Self, BlockDeviceError>
    where
        Self: Sized;

    fn device_info(&self) -> Result<DeviceInfo, BlockDeviceError>;

    fn read_at(&self, offset: u64, length: usize) -> Result<Vec<u8>, BlockDeviceError>;

    fn read_chunks<F>(
        &self,
        start_offset: u64,
        chunk_size: usize,
        callback: F,
    ) -> Result<u64, BlockDeviceError>
    where
        F: FnMut(u64, &[u8]) -> bool;

    fn path(&self) -> &str;

    fn size(&self) -> u64;
}
