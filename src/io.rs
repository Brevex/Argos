use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

pub const SECTOR_SIZE: usize = 4096;
pub const BUFFER_SIZE: usize = 4 * 1024 * 1024;
pub const OVERLAP: usize = SECTOR_SIZE;
pub const ALIGNMENT_MASK: u64 = !(SECTOR_SIZE as u64 - 1);

const BAD_SECTOR_BACKOFF_THRESHOLD: u64 = 10;
const MAX_JUMP_SIZE: u64 = 16 * 1024 * 1024;
const MAX_CONSECUTIVE_FAILURES: u64 = 1000;
const READ_TIMEOUT: Duration = Duration::from_secs(30);

static ZERO_SECTOR: [u8; SECTOR_SIZE] = [0u8; SECTOR_SIZE];

#[cfg(target_os = "linux")]
const BLKGETSIZE64_IOCTL: libc::c_ulong = 0x80081272;

pub fn is_recoverable_io_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Other
        || matches!(e.raw_os_error(), Some(libc::EIO) | Some(libc::ENODATA))
}

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

pub fn zero_sector() -> &'static [u8] {
    &ZERO_SECTOR
}

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
}

enum ScanMessage {
    Block {
        offset: u64,
        buffer: AlignedBuffer,
        bytes_read: usize,
    },
    FatalError(io::Error),
    Done,
}

struct IoThreadResult {
    reader: DiskReader,
    bad_sectors: Vec<u64>,
}

pub struct OwnedBlock {
    pub offset: u64,
    pub buffer: AlignedBuffer,
    pub bytes_read: usize,
}

impl OwnedBlock {
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.buffer.as_slice()[..self.bytes_read]
    }
}

pub struct DiskScanner {
    receiver: Option<crossbeam_channel::Receiver<ScanMessage>>,
    recycle_tx: Option<crossbeam_channel::Sender<AlignedBuffer>>,
    io_thread: Option<std::thread::JoinHandle<IoThreadResult>>,
    current_buffer: Option<AlignedBuffer>,
    finished_result: Option<IoThreadResult>,
}

impl DiskScanner {
    pub fn new(reader: DiskReader) -> Self {
        let (data_tx, data_rx) = crossbeam_channel::bounded(4);
        let (recycle_tx, recycle_rx) = crossbeam_channel::bounded(8);

        for _ in 0..8 {
            let _ = recycle_tx.send(AlignedBuffer::new());
        }

        let io_thread = std::thread::spawn(move || io_producer(reader, data_tx, recycle_rx));

        Self {
            receiver: Some(data_rx),
            recycle_tx: Some(recycle_tx),
            io_thread: Some(io_thread),
            current_buffer: None,
            finished_result: None,
        }
    }

    pub fn next_block(&mut self) -> io::Result<Option<(u64, &[u8])>> {
        if let Some(buf) = self.current_buffer.take() {
            if let Some(tx) = &self.recycle_tx {
                let _ = tx.send(buf);
            }
        }

        let receiver = match &self.receiver {
            Some(r) => r,
            None => return Ok(None),
        };

        match receiver.recv() {
            Ok(ScanMessage::Block {
                offset,
                buffer,
                bytes_read,
            }) => {
                self.current_buffer = Some(buffer);
                let slice = &self.current_buffer.as_ref().unwrap().as_slice()[..bytes_read];
                Ok(Some((offset, slice)))
            }
            Ok(ScanMessage::FatalError(e)) => {
                self.finish();
                Err(e)
            }
            Ok(ScanMessage::Done) => {
                self.finish();
                Ok(None)
            }
            Err(_) => {
                self.finish();
                Ok(None)
            }
        }
    }

    fn finish(&mut self) {
        self.receiver = None;
        self.recycle_tx = None;
        if let Some(handle) = self.io_thread.take() {
            if let Ok(result) = handle.join() {
                self.finished_result = Some(result);
            }
        }
    }

    pub fn bad_sectors(&self) -> &[u64] {
        match &self.finished_result {
            Some(r) => &r.bad_sectors,
            None => &[],
        }
    }

    pub fn into_reader(mut self) -> DiskReader {
        if self.finished_result.is_none() {
            self.finish();
        }
        self.finished_result
            .expect("DiskScanner: IO thread did not return")
            .reader
    }

    pub fn next_owned_block(&mut self) -> io::Result<Option<OwnedBlock>> {
        let receiver = match &self.receiver {
            Some(r) => r,
            None => return Ok(None),
        };

        match receiver.recv() {
            Ok(ScanMessage::Block {
                offset,
                buffer,
                bytes_read,
            }) => Ok(Some(OwnedBlock {
                offset,
                buffer,
                bytes_read,
            })),
            Ok(ScanMessage::FatalError(e)) => {
                self.finish();
                Err(e)
            }
            Ok(ScanMessage::Done) => {
                self.finish();
                Ok(None)
            }
            Err(_) => {
                self.finish();
                Ok(None)
            }
        }
    }

    pub fn recycle_buffer(&self, buffer: AlignedBuffer) {
        if let Some(tx) = &self.recycle_tx {
            let _ = tx.send(buffer);
        }
    }
}

fn io_producer(
    reader: DiskReader,
    data_tx: crossbeam_channel::Sender<ScanMessage>,
    recycle_rx: crossbeam_channel::Receiver<AlignedBuffer>,
) -> IoThreadResult {
    let mut current_offset: u64 = 0;
    let mut bad_sectors = Vec::new();
    let mut jump_size = BUFFER_SIZE as u64;
    let mut first_read = true;
    let mut consecutive_failures: u64 = 0;
    let disk_size = reader.size();
    let max_total_bad_sectors = disk_size / MAX_JUMP_SIZE;

    // Spawn a persistent reader worker so we can timeout stuck pread() calls
    // on damaged HDD sectors without blocking the entire pipeline.
    let (mut req_tx, req_rx) = crossbeam_channel::bounded::<ReadRequest>(0);
    let (resp_tx, mut resp_rx) = crossbeam_channel::bounded::<ReadResponse>(0);
    let worker_reader = reader
        .try_clone()
        .expect("Failed to clone reader for I/O worker");
    std::thread::Builder::new()
        .name("io-reader".into())
        .spawn(move || reader_worker(worker_reader, req_rx, resp_tx))
        .expect("Failed to spawn I/O reader thread");

    let mut spare_buffer: Option<AlignedBuffer> = None;

    loop {
        if current_offset >= disk_size {
            let _ = data_tx.send(ScanMessage::Done);
            break;
        }

        let read_offset = if first_read {
            0
        } else {
            current_offset.saturating_sub(OVERLAP as u64)
        };

        let buffer = if let Some(buf) = spare_buffer.take() {
            buf
        } else {
            match recycle_rx.recv() {
                Ok(buf) => buf,
                Err(_) => break,
            }
        };

        if req_tx.send(ReadRequest { offset: read_offset, buffer }).is_err() {
            break;
        }

        match resp_rx.recv_timeout(READ_TIMEOUT) {
            Ok(ReadResponse { buffer, result: Ok(n) }) if n > 0 => {
                first_read = false;
                if consecutive_failures > 0 {
                    consecutive_failures = 0;
                    jump_size = BUFFER_SIZE as u64;
                }
                current_offset = read_offset + n as u64;
                if data_tx
                    .send(ScanMessage::Block {
                        offset: read_offset,
                        buffer,
                        bytes_read: n,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(ReadResponse { buffer: _, result: Ok(_) }) => {
                let _ = data_tx.send(ScanMessage::Done);
                break;
            }
            Ok(ReadResponse { buffer, result: Err(e) }) => {
                if !is_recoverable_io_error(&e) {
                    let _ = data_tx.send(ScanMessage::FatalError(e));
                    break;
                }

                bad_sectors.push(current_offset);
                consecutive_failures += 1;
                current_offset += jump_size;

                if consecutive_failures > BAD_SECTOR_BACKOFF_THRESHOLD {
                    jump_size = (jump_size * 2).min(MAX_JUMP_SIZE);
                }

                if consecutive_failures > MAX_CONSECUTIVE_FAILURES
                    || bad_sectors.len() as u64 > max_total_bad_sectors
                {
                    let _ = data_tx.send(ScanMessage::Done);
                    break;
                }

                spare_buffer = Some(buffer);
            }
            Err(_timeout) => {
                // Worker is stuck on a damaged sector — detach it and spawn a fresh one.
                // The stuck thread will eventually exit when the kernel SCSI/ATA
                // command timeout fires (~30s), freeing its buffer and file descriptor.
                let (new_req_tx, new_req_rx) = crossbeam_channel::bounded::<ReadRequest>(0);
                let (new_resp_tx, new_resp_rx) = crossbeam_channel::bounded::<ReadResponse>(0);
                match reader.try_clone() {
                    Ok(new_reader) => {
                        let _ = std::thread::Builder::new()
                            .name("io-reader".into())
                            .spawn(move || reader_worker(new_reader, new_req_rx, new_resp_tx));
                    }
                    Err(_) => {
                        let _ = data_tx.send(ScanMessage::Done);
                        break;
                    }
                }
                req_tx = new_req_tx;
                resp_rx = new_resp_rx;

                bad_sectors.push(current_offset);
                consecutive_failures += 1;
                current_offset += jump_size;

                if consecutive_failures > BAD_SECTOR_BACKOFF_THRESHOLD {
                    jump_size = (jump_size * 2).min(MAX_JUMP_SIZE);
                }

                if consecutive_failures > MAX_CONSECUTIVE_FAILURES
                    || bad_sectors.len() as u64 > max_total_bad_sectors
                {
                    let _ = data_tx.send(ScanMessage::Done);
                    break;
                }

                // Buffer is lost with the stuck thread — allocate a replacement
                spare_buffer = Some(AlignedBuffer::new());
            }
        }
    }

    IoThreadResult {
        reader,
        bad_sectors,
    }
}

struct ReadRequest {
    offset: u64,
    buffer: AlignedBuffer,
}

struct ReadResponse {
    buffer: AlignedBuffer,
    result: io::Result<usize>,
}

fn reader_worker(
    reader: DiskReader,
    req_rx: crossbeam_channel::Receiver<ReadRequest>,
    resp_tx: crossbeam_channel::Sender<ReadResponse>,
) {
    while let Ok(mut req) = req_rx.recv() {
        let result = reader.read_at(req.offset, &mut req.buffer);
        if resp_tx
            .send(ReadResponse {
                buffer: req.buffer,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}
