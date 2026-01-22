use crate::{CoreError, Result};
use memmap2::Mmap;

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::path::Path;

pub trait ZeroCopySource: Send + Sync {
    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    fn size(&self) -> u64;
}

pub const PAGE_SIZE: usize = 4096;

#[cfg(unix)]
pub fn allocate_aligned_buffer(size: usize) -> Vec<u8> {
    use std::alloc::{alloc_zeroed, Layout};

    let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let layout = Layout::from_size_align(aligned_size, PAGE_SIZE)
        .expect("Invalid layout for aligned allocation");

    unsafe {
        let ptr = alloc_zeroed(layout);
        if ptr.is_null() {
            return vec![0u8; size];
        }
        Vec::from_raw_parts(ptr, size, aligned_size)
    }
}

#[cfg(not(unix))]
pub fn allocate_aligned_buffer(size: usize) -> Vec<u8> {
    vec![0u8; size]
}

#[cfg(target_os = "linux")]
fn is_block_device(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path)
        .map(|m| m.file_type().is_block_device())
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn is_block_device(_path: &Path) -> bool {
    false
}

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

    #[inline]
    pub fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            let bytes_read = self.file.read_at(buffer, offset)?;
            Ok(bytes_read)
        }
        #[cfg(not(unix))]
        {
            todo!("Windows support not implemented")
        }
    }
}

impl ZeroCopySource for DiskReader {
    #[inline]
    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        DiskReader::read_into(self, offset, buffer)
    }

    #[inline]
    fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(target_os = "linux")]
pub struct DirectReader {
    fd: std::os::unix::io::RawFd,
    size: u64,
}

#[cfg(target_os = "linux")]
impl DirectReader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path_cstr = CString::new(path.as_ref().as_os_str().as_bytes())
            .map_err(|_| CoreError::InvalidFormat("Invalid path".into()))?;

        let fd = unsafe {
            libc::open(
                path_cstr.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECT | libc::O_NOATIME,
            )
        };

        if fd < 0 {
            let fd = unsafe { libc::open(path_cstr.as_ptr(), libc::O_RDONLY | libc::O_DIRECT) };
            if fd < 0 {
                return Err(CoreError::Io(std::io::Error::last_os_error()));
            }
        }

        let size = unsafe { libc::lseek(fd, 0, libc::SEEK_END) };
        if size < 0 {
            unsafe { libc::close(fd) };
            return Err(CoreError::Io(std::io::Error::last_os_error()));
        }

        Ok(Self {
            fd,
            size: size as u64,
        })
    }

    #[inline]
    pub fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let aligned_offset = offset & !511;
        let offset_adjustment = (offset - aligned_offset) as usize;

        let ptr = buffer.as_ptr() as usize;
        if ptr % PAGE_SIZE != 0 {
            return Err(CoreError::InvalidFormat(
                "Buffer not page-aligned for O_DIRECT".into(),
            ));
        }

        let bytes_read = unsafe {
            libc::pread(
                self.fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
                aligned_offset as libc::off_t,
            )
        };

        if bytes_read < 0 {
            return Err(CoreError::Io(std::io::Error::last_os_error()));
        }

        let actual_bytes = (bytes_read as usize).saturating_sub(offset_adjustment);
        Ok(actual_bytes)
    }
}

#[cfg(target_os = "linux")]
impl Drop for DirectReader {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[cfg(target_os = "linux")]
impl ZeroCopySource for DirectReader {
    #[inline]
    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        DirectReader::read_into(self, offset, buffer)
    }

    #[inline]
    fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(target_os = "linux")]
unsafe impl Send for DirectReader {}
#[cfg(target_os = "linux")]
unsafe impl Sync for DirectReader {}

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

impl ZeroCopySource for MmapReader {
    #[inline]
    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(slice) = self.slice(offset, buffer.len()) {
            let len = slice.len().min(buffer.len());
            buffer[..len].copy_from_slice(&slice[..len]);
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
    #[cfg(target_os = "linux")]
    Direct(DirectReader),
}

impl Reader {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path_ref = path.as_ref();

        match MmapReader::new(path_ref) {
            Ok(r) => return Ok(Reader::Mmap(r)),
            Err(_) => {}
        }

        #[cfg(target_os = "linux")]
        if is_block_device(path_ref) {
            if let Ok(r) = DirectReader::new(path_ref) {
                eprintln!("[Reader] Using O_DIRECT for block device (bypassing page cache)");
                return Ok(Reader::Direct(r));
            }
        }

        Ok(Reader::Disk(DiskReader::new(path_ref)?))
    }

    #[inline]
    pub fn is_mmap(&self) -> bool {
        matches!(self, Reader::Mmap(_))
    }

    #[cfg(target_os = "linux")]
    #[inline]
    pub fn is_direct(&self) -> bool {
        matches!(self, Reader::Direct(_))
    }

    #[cfg(not(target_os = "linux"))]
    #[inline]
    pub fn is_direct(&self) -> bool {
        false
    }
}

impl ZeroCopySource for Reader {
    fn read_into(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        match self {
            Reader::Mmap(r) => r.read_into(offset, buffer),
            Reader::Disk(r) => r.read_into(offset, buffer),
            #[cfg(target_os = "linux")]
            Reader::Direct(r) => r.read_into(offset, buffer),
        }
    }

    #[inline]
    fn size(&self) -> u64 {
        match self {
            Reader::Mmap(r) => ZeroCopySource::size(r),
            Reader::Disk(r) => r.size,
            #[cfg(target_os = "linux")]
            Reader::Direct(r) => r.size,
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
        let reader = DiskReader::new(temp_file.path()).unwrap();

        assert_eq!(ZeroCopySource::size(&reader), test_data.len() as u64);

        let mut buffer = [0u8; 13];
        let bytes_read = reader.read_into(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 13);
        assert_eq!(&buffer, b"Hello, World!");
    }

    #[test]
    fn test_disk_reader_read_into() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, World! This is test data for DiskReader.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();
        let reader = DiskReader::new(temp_file.path()).unwrap();

        let mut buffer = [0u8; 13];
        let bytes_read = reader.read_into(0, &mut buffer).unwrap();
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
        assert_eq!(ZeroCopySource::size(&reader), test_data.len() as u64);

        let slice = reader.slice(0, 13).unwrap();
        assert_eq!(slice, b"Hello, World!");

        let mut buffer = [0u8; 13];
        let bytes_read = reader.read_into(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 13);
        assert_eq!(&buffer, b"Hello, World!");
    }

    #[test]
    fn test_mmap_reader_read_into() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let test_data = b"Hello, World! This is test data for MmapReader.";
        temp_file.write_all(test_data).unwrap();
        temp_file.flush().unwrap();

        let reader = MmapReader::new(temp_file.path()).unwrap();

        let mut buffer = [0u8; 13];
        let bytes_read = reader.read_into(0, &mut buffer).unwrap();
        assert_eq!(bytes_read, 13);
        assert_eq!(&buffer, b"Hello, World!");
    }

    #[test]
    fn test_mmap_reader_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let result = MmapReader::new(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_aligned_buffer_allocation() {
        let buffer = allocate_aligned_buffer(8192);
        assert!(buffer.len() >= 8192);
        // Check alignment
        let ptr = buffer.as_ptr() as usize;
        assert_eq!(ptr % PAGE_SIZE, 0, "Buffer not page-aligned");
    }

    #[test]
    fn test_zero_copy_source_trait() {
        fn assert_zero_copy<T: ZeroCopySource>() {}
        assert_zero_copy::<DiskReader>();
        assert_zero_copy::<MmapReader>();
        assert_zero_copy::<Reader>();
    }
}
