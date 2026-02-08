use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const SECTOR_SIZE: usize = 4096;
pub const BUFFER_SIZE: usize = 1024 * 1024;
pub const OVERLAP: usize = SECTOR_SIZE;

const BAD_SECTOR_BACKOFF_THRESHOLD: usize = 10;
const MAX_JUMP_SIZE: u64 = 16 * 1024 * 1024;

#[cfg(target_os = "linux")]
fn get_block_device_size(file: &File) -> io::Result<u64> {
    use std::os::unix::io::AsRawFd;

    const BLKGETSIZE64: libc::c_ulong = 0x80081272;

    let mut size: u64 = 0;
    let result = unsafe { libc::ioctl(file.as_raw_fd(), BLKGETSIZE64, &mut size) };

    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(size)
    }
}

#[cfg(not(target_os = "linux"))]
fn get_block_device_size(_file: &File) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Not supported on this platform",
    ))
}

pub struct AlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuffer {
    pub fn new() -> Self {
        let layout = Layout::from_size_align(BUFFER_SIZE, SECTOR_SIZE)
            .expect("Invalid layout for AlignedBuffer");

        let ptr = unsafe { alloc_zeroed(layout) };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        Self { ptr, layout }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, BUFFER_SIZE) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, BUFFER_SIZE) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        BUFFER_SIZE
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Default for AlignedBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr, self.layout);
        }
    }
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

pub struct DiskReader {
    file: File,
    size: u64,
}

impl DiskReader {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_options(path, true)
    }

    pub fn open_regular(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_options(path, false)
    }

    fn open_with_options(path: impl AsRef<Path>, direct_io: bool) -> io::Result<Self> {
        let path = path.as_ref();

        #[cfg(target_os = "linux")]
        let file = if direct_io {
            use std::os::unix::fs::OpenOptionsExt;
            match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECT)
                .open(path)
            {
                Ok(f) => f,
                Err(_) => OpenOptions::new().read(true).open(path)?,
            }
        } else {
            OpenOptions::new().read(true).open(path)?
        };

        #[cfg(target_os = "windows")]
        let file = if direct_io {
            use std::os::windows::fs::OpenOptionsExt;
            match OpenOptions::new()
                .read(true)
                .custom_flags(0x20000000)
                .open(path)
            {
                Ok(f) => f,
                Err(_) => OpenOptions::new().read(true).open(path)?,
            }
        } else {
            OpenOptions::new().read(true).open(path)?
        };

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let file = {
            let _ = direct_io;
            OpenOptions::new().read(true).open(path)?
        };

        let mut size = file.metadata()?.len();

        if size == 0 {
            if let Ok(device_size) = get_block_device_size(&file) {
                size = device_size;
            }
        }

        if size == 0 {
            let mut file_mut = file;
            if let Ok(end_pos) = file_mut.seek(SeekFrom::End(0)) {
                size = end_pos;
                let _ = file_mut.seek(SeekFrom::Start(0));
            }

            return Ok(Self {
                file: file_mut,
                size,
            });
        }

        Ok(Self { file, size })
    }

    pub fn read_at(&mut self, offset: u64, buffer: &mut AlignedBuffer) -> io::Result<usize> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read(buffer.as_mut_slice())
    }

    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }
}

pub struct DiskScanner {
    reader: DiskReader,
    current_offset: u64,
    buffer: AlignedBuffer,
    bad_sectors: Vec<u64>,
    jump_size: u64,
    first_read: bool,
}

impl DiskScanner {
    pub fn new(reader: DiskReader) -> Self {
        Self {
            reader,
            current_offset: 0,
            buffer: AlignedBuffer::new(),
            bad_sectors: Vec::new(),
            jump_size: BUFFER_SIZE as u64,
            first_read: true,
        }
    }

    pub fn next_block(&mut self) -> io::Result<Option<(u64, &[u8])>> {
        if self.current_offset >= self.reader.size() {
            return Ok(None);
        }

        let read_offset = if self.first_read {
            0
        } else {
            self.current_offset.saturating_sub(OVERLAP as u64)
        };

        match self.reader.read_at(read_offset, &mut self.buffer) {
            Ok(n) if n > 0 => {
                self.first_read = false;
                self.current_offset = read_offset + n as u64;
                Ok(Some((read_offset, &self.buffer.as_slice()[..n])))
            }
            Ok(_) => Ok(None),
            Err(e) => {
                let is_recoverable = e.kind() == io::ErrorKind::InvalidInput
                    || e.kind() == io::ErrorKind::Other
                    || matches!(e.raw_os_error(), Some(5) | Some(61));

                if is_recoverable {
                    self.bad_sectors.push(self.current_offset);
                    self.current_offset += self.jump_size;

                    if self.bad_sectors.len() > BAD_SECTOR_BACKOFF_THRESHOLD {
                        self.jump_size = (self.jump_size * 2).min(MAX_JUMP_SIZE);
                    }
                    self.next_block()
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn bad_sectors(&self) -> &[u64] {
        &self.bad_sectors
    }
}
