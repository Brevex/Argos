use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::Path;

use super::buffer::AlignedBuffer;

#[cfg(target_os = "linux")]
const BLKGETSIZE64_IOCTL: libc::c_ulong = 0x80081272;

#[cfg(target_os = "linux")]
fn get_block_device_size(file: &File) -> io::Result<u64> {
    use std::os::unix::io::AsRawFd;

    let mut size: u64 = 0;
    let result = unsafe { libc::ioctl(file.as_raw_fd(), BLKGETSIZE64_IOCTL, &mut size) };

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

#[cfg(target_os = "linux")]
fn advise_sequential(file: &File) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL);
    }
}

#[cfg(not(target_os = "linux"))]
fn advise_sequential(_file: &File) {}

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
            const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
            match OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_NO_BUFFERING)
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

        if direct_io {
            advise_sequential(&file);
        }

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

    #[cfg(unix)]
    pub fn read_at(&self, offset: u64, buffer: &mut AlignedBuffer) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;
        self.file.read_at(buffer.as_mut_slice(), offset)
    }

    #[cfg(target_os = "windows")]
    pub fn read_at(&self, offset: u64, buffer: &mut AlignedBuffer) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;
        self.file.seek_read(buffer.as_mut_slice(), offset)
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    pub fn read_at(&self, offset: u64, buffer: &mut AlignedBuffer) -> io::Result<usize> {
        use std::io::{Read, Seek};
        use std::sync::Mutex;
        static FALLBACK_LOCK: Mutex<()> = Mutex::new(());
        let _guard = FALLBACK_LOCK
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "lock poisoned"))?;
        let file = &self.file;
        let mut file_ref = file;
        (&mut file_ref).seek(SeekFrom::Start(offset))?;
        (&mut file_ref).read(buffer.as_mut_slice())
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            size: self.size,
        })
    }

    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[cfg(target_os = "linux")]
    pub fn advise_willneed(&self, offset: u64, len: u64) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::posix_fadvise(
                self.file.as_raw_fd(),
                offset as i64,
                len as i64,
                libc::POSIX_FADV_WILLNEED,
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn advise_willneed(&self, _offset: u64, _len: u64) {}
}
