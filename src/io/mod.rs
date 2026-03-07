pub mod buffer;
pub mod reader;
pub mod scanner;

use std::io;

pub use buffer::AlignedBuffer;
pub use reader::DiskReader;
pub use scanner::{DiskScanner, OwnedBlock, PollResult};

pub const SECTOR_SIZE: usize = 4096;
pub const BUFFER_SIZE: usize = 4 * 1024 * 1024;
pub const OVERLAP: usize = SECTOR_SIZE;
pub const ALIGNMENT_MASK: u64 = !(SECTOR_SIZE as u64 - 1);

static ZERO_SECTOR: [u8; SECTOR_SIZE] = [0u8; SECTOR_SIZE];

pub fn is_recoverable_io_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Other
        || matches!(e.raw_os_error(), Some(libc::EIO) | Some(libc::ENODATA))
}

pub fn zero_sector() -> &'static [u8] {
    &ZERO_SECTOR
}
