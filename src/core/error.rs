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
