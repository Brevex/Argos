//! Block device implementations

mod linux_block_device;
mod mmap_block_device;

pub use linux_block_device::LinuxBlockDevice;
pub use mmap_block_device::MmapBlockDevice;

