//! Block reader implementation for physical disks and image files.

use argos_core::{BlockSource, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// A read-only block source that reads from physical disks or disk image files.
///
/// `DiskReader` implements the `BlockSource` trait to provide random-access
/// block reading from any file-like source including:
/// - Physical disk devices (`/dev/sda`, `/dev/nvme0n1`, etc.)
/// - Partition devices (`/dev/sda1`, etc.)
/// - Disk image files (`.img`, `.raw`, etc.)
///
/// # Safety
///
/// This implementation only opens files in read-only mode and uses only safe Rust.
/// No `unsafe` code is used.
///
/// # Example
///
/// ```ignore
/// use argos_io::DiskReader;
/// use argos_core::BlockSource;
///
/// // Open a disk device or image file
/// let mut reader = DiskReader::new("/dev/sda")?;
///
/// // Read the first sector (512 bytes)
/// let mut buffer = vec![0u8; 512];
/// let bytes_read = reader.read_chunk(0, &mut buffer)?;
/// ```
pub struct DiskReader {
    file: File,
    size: u64,
}

impl DiskReader {
    /// Creates a new `DiskReader` for the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the disk device or image file to read from
    ///
    /// # Returns
    ///
    /// A `Result` containing the `DiskReader` on success, or an error if:
    /// - The file/device does not exist
    /// - Permission is denied
    /// - The file size cannot be determined
    ///
    /// # Example
    ///
    /// ```ignore
    /// let reader = DiskReader::new("/dev/sda")?;
    /// ```
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
        let bytes_read = self.file.read(buffer)?;

        Ok(bytes_read)
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

        let mut buffer = vec![0u8; 4];
        let bytes_read = reader.read_chunk(7, &mut buffer).unwrap();
        assert_eq!(bytes_read, 4);
        assert_eq!(&buffer, b"Worl");
    }

    #[test]
    fn test_disk_reader_read_beyond_end() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"Short").unwrap();
        temp_file.flush().unwrap();

        let mut reader = DiskReader::new(temp_file.path()).unwrap();

        let mut buffer = vec![0u8; 100];
        let bytes_read = reader.read_chunk(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 5);
    }
}
