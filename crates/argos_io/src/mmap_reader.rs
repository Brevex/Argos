use argos_core::{BlockSource, CoreError, Result};
use memmap2::Mmap;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::Path;

pub struct MmapReader {
    mmap: Mmap,
    size: u64,
}

impl MmapReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;

        let size = file.seek(SeekFrom::End(0))?;

        if size == 0 {
            return Err(CoreError::InvalidFormat(
                "Cannot mmap empty file".to_string(),
            ));
        }

        let mmap =
            unsafe { Mmap::map(&file) }.map_err(|e| CoreError::Io(std::io::Error::other(e)))?;

        if mmap.len() == 0 {
            return Err(CoreError::InvalidFormat(
                "mmap returned empty mapping (block device not supported)".to_string(),
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

    fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

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

        let slice = reader.slice(7, 4).unwrap();
        assert_eq!(slice, b"Worl");
    }

    #[test]
    fn test_mmap_reader_block_source_trait() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Test data for BlockSource trait.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let mut reader = MmapReader::new(temp_file.path()).unwrap();

        let mut buffer = vec![0u8; 9];
        let bytes_read = reader.read_chunk(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 9);
        assert_eq!(&buffer, b"Test data");
    }

    #[test]
    fn test_mmap_reader_beyond_eof() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Short").unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        let slice = reader.slice(0, 100).unwrap();
        assert_eq!(slice.len(), 5);

        let slice = reader.slice(100, 10);
        assert!(slice.is_none());
    }

    #[test]
    fn test_mmap_reader_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let result = MmapReader::new(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_prefetch_does_not_panic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Data for prefetch test").unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        reader.prefetch(0, 1024);
        reader.prefetch(1000, 1024);
    }
}
