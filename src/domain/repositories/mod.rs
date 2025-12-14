//! Repository traits (interfaces)
//!
//! These traits define the contracts for external dependencies.
//! They follow the Dependency Inversion Principle (DIP) from SOLID.

mod block_device;
mod file_system;
mod file_writer;

pub use block_device::{BlockDeviceError, BlockDeviceReader, DeviceInfo};
pub use file_system::{DeletedFileEntry, FileSystemError, FileSystemParser, FileSystemType};
pub use file_writer::{FileWriterError, RecoveredFileWriter, WriteOptions, WriteResult};
