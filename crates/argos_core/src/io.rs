use crate::{BlockSource, CoreError, Result};
use memmap2::Mmap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub struct DiskReader {
    file: File,
    size: u64,
}

impl DiskReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(path.as_ref())?;

        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{fadvise, Advice};
            let _ = fadvise(&file, 0, None, Advice::Sequential);
            let _ = fadvise(&file, 0, None, Advice::NoReuse);
        }

        let size = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        Ok(Self { file, size })
    }
}

impl BlockSource for DiskReader {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(self.file.read(buffer)?)
    }

    #[inline]
    fn size(&self) -> u64 {
        self.size
    }
}

pub struct MmapReader {
    mmap: Mmap,
    size: u64,
}

impl MmapReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;
        let size = file.seek(SeekFrom::End(0))?;

        if size == 0 {
            return Err(CoreError::InvalidFormat("Cannot mmap empty file".into()));
        }

        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| CoreError::Io(std::io::Error::other(e)))?;

        if mmap.is_empty() {
            return Err(CoreError::InvalidFormat(
                "mmap returned empty mapping (block device not supported)".into(),
            ));
        }

        #[cfg(target_os = "linux")]
        {
            use memmap2::Advice;
            let _ = mmap.advise(Advice::Sequential);
            let _ = mmap.advise(Advice::WillNeed);
        }

        Ok(Self { mmap, size })
    }

    #[inline]
    pub fn slice(&self, offset: u64, len: usize) -> Option<&[u8]> {
        let start = offset as usize;
        if start >= self.mmap.len() {
            return None;
        }
        let end = start.saturating_add(len).min(self.mmap.len());
        Some(&self.mmap[start..end])
    }

    #[cfg(target_os = "linux")]
    pub fn prefetch(&self, offset: u64, len: usize) {
        let start = offset as usize;
        if start >= self.mmap.len() {
            return;
        }
        let end = start.saturating_add(len).min(self.mmap.len());
        let _ = self
            .mmap
            .advise_range(memmap2::Advice::WillNeed, start, end - start);
    }

    #[cfg(not(target_os = "linux"))]
    pub fn prefetch(&self, _offset: u64, _len: usize) {}
}

impl BlockSource for MmapReader {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(slice) = self.slice(offset, buffer.len()) {
            let len = slice.len();
            buffer[..len].copy_from_slice(slice);
            Ok(len)
        } else {
            Ok(0)
        }
    }

    #[inline]
    fn size(&self) -> u64 {
        self.size
    }
}

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

    #[inline]
    fn size(&self) -> u64 {
        match self {
            Reader::Mmap(r) => r.size(),
            Reader::Disk(r) => r.size(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_disk_reader_basic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, World! This is test data for DiskReader.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();
        let mut reader = DiskReader::new(temp_file.path()).unwrap();

        assert_eq!(reader.size(), test_data.len() as u64);

        let mut buffer = vec![0u8; 13];
        let bytes_read = reader.read_chunk(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 13);
        assert_eq!(&buffer, b"Hello, World!");
    }

    #[test]
    fn test_mmap_reader_basic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, World! This is test data for MmapReader.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();
        assert_eq!(reader.size(), test_data.len() as u64);

        let slice = reader.slice(0, 13).unwrap();
        assert_eq!(slice, b"Hello, World!");
    }

    #[test]
    fn test_mmap_reader_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let result = MmapReader::new(temp_file.path());
        assert!(result.is_err());
    }
}
