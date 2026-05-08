use std::alloc::{alloc, dealloc, Layout};
use std::fmt;
use std::path::Path;
use std::slice;

use rustix::fs::{fstat, open, Mode, OFlags};
use rustix::io::{pread, Errno};

use crate::error::ArgosError;

pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
    cap: usize,
    align: usize,
}

impl AlignedBuf {
    pub fn with_capacity(cap: usize, align: usize) -> Result<Self, ArgosError> {
        if align == 0 || (align & (align - 1)) != 0 {
            return Err(ArgosError::Allocation { size: cap, align });
        }
        let layout = Layout::from_size_align(cap, align)
            .map_err(|_| ArgosError::Allocation { size: cap, align })?;
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err(ArgosError::Allocation { size: cap, align });
        }
        Ok(Self {
            ptr,
            len: 0,
            cap,
            align,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    pub fn set_len(&mut self, len: usize) {
        assert!(len <= self.cap);
        self.len = len;
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.cap, self.align).unwrap();
        unsafe { dealloc(self.ptr, layout) }
    }
}

unsafe impl Send for AlignedBuf {}

impl fmt::Debug for AlignedBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlignedBuf")
            .field("len", &self.len)
            .field("cap", &self.cap)
            .field("align", &self.align)
            .finish_non_exhaustive()
    }
}

pub struct SourceDevice {
    fd: std::os::fd::OwnedFd,
    sector_size: usize,
}

impl SourceDevice {
    pub fn open(path: &Path) -> Result<Self, ArgosError> {
        let flags = OFlags::RDONLY | OFlags::DIRECT | OFlags::NOATIME;
        let fd = open(path, flags, Mode::from_raw_mode(0))
            .map_err(ArgosError::from)?;
        let sector_size = 4096;
        Ok(Self { fd, sector_size })
    }

    pub fn sector_size(&self) -> usize {
        self.sector_size
    }

    pub fn size(&self) -> Result<u64, ArgosError> {
        let stat = fstat(&self.fd)?;
        Ok(stat.st_size as u64)
    }

    fn read_at(&self, buf: &mut AlignedBuf, offset: u64) -> Result<usize, ArgosError> {
        let n = pread(&self.fd, buf.as_mut_slice(), offset)
            .map_err(ArgosError::from)?;
        Ok(n)
    }

    pub fn read_range(&self, buf: &mut [u8], offset: u64) -> Result<usize, ArgosError> {
        let n = pread(&self.fd, buf, offset)
            .map_err(ArgosError::from)?;
        Ok(n)
    }
}

impl fmt::Debug for SourceDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceDevice")
            .field("sector_size", &self.sector_size)
            .finish_non_exhaustive()
    }
}

pub struct OutputSink {
    base_dir: std::path::PathBuf,
}

impl OutputSink {
    pub fn create(base_dir: &Path) -> Result<Self, ArgosError> {
        std::fs::create_dir_all(base_dir)?;
        Ok(Self {
            base_dir: base_dir.to_path_buf(),
        })
    }

    pub fn create_file(&self, name: &str) -> Result<std::io::BufWriter<std::fs::File>, ArgosError> {
        let path = self.base_dir.join(name);
        let file = std::fs::File::create(&path)?;
        let blksize = Self::blksize(&path)?;
        Ok(std::io::BufWriter::with_capacity(blksize, file))
    }

    #[cfg(unix)]
    fn blksize(path: &Path) -> Result<usize, ArgosError> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path)?;
        Ok(meta.blksize() as usize)
    }

    #[cfg(not(unix))]
    fn blksize(_path: &Path) -> Result<usize, ArgosError> {
        Ok(64 * 1024)
    }
}

impl fmt::Debug for OutputSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputSink").finish_non_exhaustive()
    }
}

pub struct BlockReader {
    device: SourceDevice,
    buf: AlignedBuf,
    offset: u64,
    end: u64,
    sector_size: usize,
    bad_sectors: Vec<(u64, u64)>,
}

impl BlockReader {
    pub fn new(device: SourceDevice, buf: AlignedBuf, end: u64) -> Self {
        let sector_size = device.sector_size();
        Self {
            device,
            buf,
            offset: 0,
            end,
            sector_size,
            bad_sectors: Vec::new(),
        }
    }

    pub fn bad_sectors(&self) -> &[(u64, u64)] {
        &self.bad_sectors
    }

    pub fn try_next(&mut self) -> Result<Option<&[u8]>, ArgosError> {
        while self.offset < self.end {
            let remaining = (self.end - self.offset) as usize;
            let to_read = self.buf.capacity().min(remaining);
            let to_read = align_down(to_read, self.sector_size);
            if to_read == 0 {
                return Ok(None);
            }
            self.buf.set_len(to_read);
            match self.device.read_at(&mut self.buf, self.offset) {
                Ok(n) => {
                    self.buf.set_len(n);
                    self.offset += n as u64;
                    return Ok(Some(self.buf.as_slice()));
                }
                Err(ArgosError::Io(ref e)) if is_bad_sector_error(e) => {
                    self.bad_sectors.push((self.offset, to_read as u64));
                    self.offset += to_read as u64;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

impl fmt::Debug for BlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockReader")
            .field("offset", &self.offset)
            .field("end", &self.end)
            .field("sector_size", &self.sector_size)
            .field("bad_sector_count", &self.bad_sectors.len())
            .finish_non_exhaustive()
    }
}

fn align_down(n: usize, align: usize) -> usize {
    n & !(align - 1)
}

fn is_bad_sector_error(e: &std::io::Error) -> bool {
    let expected: std::io::Error = Errno::IO.into();
    e.raw_os_error() == expected.raw_os_error()
}

#[cfg(test)]
impl SourceDevice {
    fn from_file(file: std::fs::File, sector_size: usize) -> Self {
        let fd: std::os::fd::OwnedFd = file.into();
        Self { fd, sector_size }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_test_file(path: &Path, data: &[u8]) {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(data).unwrap();
    }

    fn is_inval_error(e: &std::io::Error) -> bool {
        let expected: std::io::Error = Errno::INVAL.into();
        e.raw_os_error() == expected.raw_os_error()
    }

    #[test]
    fn aligned_buf_allocates_and_slices() -> Result<(), ArgosError> {
        let mut buf = AlignedBuf::with_capacity(4096, 4096)?;
        assert_eq!(buf.capacity(), 4096);
        assert_eq!(buf.as_slice().len(), 0);
        buf.set_len(4096);
        assert_eq!(buf.as_slice().len(), 4096);
        buf.as_mut_slice().fill(0xAB);
        assert!(buf.as_slice().iter().all(|&b| b == 0xAB));
        Ok(())
    }

    #[test]
    fn source_device_opens_regular_file() -> Result<(), ArgosError> {
        let name = format!(".test_device_{}", std::process::id());
        let path = std::env::current_dir()?.join(&name);
        write_test_file(&path, &[0u8; 4096]);
        match SourceDevice::open(&path) {
            Ok(dev) => {
                std::fs::remove_file(&path).ok();
                assert_eq!(dev.sector_size(), 4096);
                Ok(())
            }
            Err(ArgosError::Io(ref e)) if is_inval_error(e) => {
                std::fs::remove_file(&path).ok();
                Ok(())
            }
            Err(e) => {
                std::fs::remove_file(&path).ok();
                Err(e)
            }
        }
    }

    #[test]
    fn block_reader_reads_aligned_blocks() -> Result<(), ArgosError> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.bin");
        let data = vec![0xCDu8; 8192];
        write_test_file(&path, &data);

        let file = std::fs::File::open(&path).unwrap();
        let dev = SourceDevice::from_file(file, 4096);

        let buf = AlignedBuf::with_capacity(4096, 4096)?;
        let mut reader = BlockReader::new(dev, buf, 8192);

        let b1 = reader.try_next()?.unwrap();
        assert_eq!(b1.len(), 4096);
        assert!(b1.iter().all(|&b| b == 0xCD));

        let b2 = reader.try_next()?.unwrap();
        assert_eq!(b2.len(), 4096);
        assert!(b2.iter().all(|&b| b == 0xCD));

        assert!(reader.try_next()?.is_none());
        assert!(reader.bad_sectors().is_empty());
        Ok(())
    }

    #[test]
    fn output_sink_creates_and_writes() -> Result<(), ArgosError> {
        let dir = tempdir().unwrap();
        let sink = OutputSink::create(dir.path())?;
        let mut writer = sink.create_file("test.out")?;
        use std::io::Write;
        writer.write_all(b"hello").unwrap();
        drop(writer);
        let read_back = std::fs::read(dir.path().join("test.out")).unwrap();
        assert_eq!(read_back, b"hello");
        Ok(())
    }
}
