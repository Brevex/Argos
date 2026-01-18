mod disk;
mod mmap;

pub use disk::DiskReader;
pub use mmap::MmapReader;

use crate::{BlockSource, Result};
use std::path::Path;

pub enum Reader {
    Mmap(MmapReader),
    Disk(DiskReader),
}

impl Reader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path_ref = path.as_ref();
        match MmapReader::new(path_ref) {
            Ok(r) => Ok(Reader::Mmap(r)),
            Err(_) => Ok(Reader::Disk(DiskReader::new(path_ref)?)),
        }
    }

    #[inline]
    pub fn is_mmap(&self) -> bool {
        matches!(self, Reader::Mmap(_))
    }
}

impl BlockSource for Reader {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        match self {
            Reader::Mmap(r) => r.read_chunk(offset, buffer),
            Reader::Disk(r) => r.read_chunk(offset, buffer),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Reader::Mmap(r) => r.size(),
            Reader::Disk(r) => r.size(),
        }
    }
}
