use std::alloc::{Layout, alloc, dealloc};
use std::fmt;
use std::path::Path;
use std::slice;

use rustix::fs::{Mode, OFlags, SeekFrom, fstat, open, seek};
use rustix::io::{Errno, pread};

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
        let fd = open(path, flags, Mode::from_raw_mode(0)).map_err(ArgosError::from)?;
        let sector_size = 4096;
        Ok(Self { fd, sector_size })
    }

    pub fn sector_size(&self) -> usize {
        self.sector_size
    }

    pub fn size(&self) -> Result<u64, ArgosError> {
        let stat = fstat(&self.fd)?;
        if stat.st_size > 0 {
            return Ok(stat.st_size as u64);
        }
        Ok(seek(&self.fd, SeekFrom::End(0))?)
    }

    fn read_at(&self, buf: &mut AlignedBuf, offset: u64) -> Result<usize, ArgosError> {
        let n = pread(&self.fd, buf.as_mut_slice(), offset).map_err(ArgosError::from)?;
        Ok(n)
    }

    pub fn read_range(&self, buf: &mut [u8], offset: u64) -> Result<usize, ArgosError> {
        let n = pread(&self.fd, buf, offset).map_err(ArgosError::from)?;
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

pub struct BlockReader<'a> {
    device: &'a SourceDevice,
    buf: AlignedBuf,
    offset: u64,
    end: u64,
    sector_size: usize,
    bad_sectors: Vec<(u64, u64)>,
}

impl<'a> BlockReader<'a> {
    pub fn new(device: &'a SourceDevice, buf: AlignedBuf, end: u64) -> Self {
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

impl fmt::Debug for BlockReader<'_> {
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

#[cfg(target_os = "linux")]
pub fn detect_device_class(path: &Path) -> crate::carve::DeviceClass {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let sys_path = format!("/sys/block/{name}/queue/rotational");
        if let Ok(content) = std::fs::read_to_string(&sys_path) {
            if content.trim() == "1" {
                return crate::carve::DeviceClass::Hdd;
            } else if content.trim() == "0" {
                return crate::carve::DeviceClass::Ssd;
            }
        }
    }
    crate::carve::DeviceClass::Hdd
}

#[cfg(target_os = "windows")]
pub fn detect_device_class(_path: &Path) -> crate::carve::DeviceClass {
    crate::carve::DeviceClass::Hdd
}
