use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::buffer::AlignedBuffer;
use super::reader::DiskReader;
use super::{is_recoverable_io_error, BUFFER_SIZE, OVERLAP};

const BAD_SECTOR_BACKOFF_THRESHOLD: u64 = 3;
const MAX_JUMP_SIZE: u64 = 16 * 1024 * 1024;
const MAX_CONSECUTIVE_FAILURES: u64 = 100;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_TIMEOUT: Duration = Duration::from_millis(500);

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

pub enum PollResult {
    Block(OwnedBlock),
    Pending,
    Done,
}

pub struct DiskScanner {
    receiver: Option<crossbeam_channel::Receiver<ScanMessage>>,
    recycle_tx: Option<crossbeam_channel::Sender<AlignedBuffer>>,
    io_thread: Option<std::thread::JoinHandle<IoThreadResult>>,
    current_buffer: Option<AlignedBuffer>,
    finished_result: Option<IoThreadResult>,
    progress: Arc<AtomicU64>,
}

impl DiskScanner {
    pub fn new(reader: DiskReader) -> Self {
        let (data_tx, data_rx) = crossbeam_channel::bounded(4);
        let (recycle_tx, recycle_rx) = crossbeam_channel::bounded(8);

        for _ in 0..8 {
            let _ = recycle_tx.send(AlignedBuffer::new());
        }

        let progress = Arc::new(AtomicU64::new(0));
        let progress_clone = progress.clone();
        let io_thread =
            std::thread::spawn(move || io_producer(reader, data_tx, recycle_rx, progress_clone));

        Self {
            receiver: Some(data_rx),
            recycle_tx: Some(recycle_tx),
            io_thread: Some(io_thread),
            current_buffer: None,
            finished_result: None,
            progress,
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

    pub fn recycle_buffer(&self, buffer: AlignedBuffer) {
        if let Some(tx) = &self.recycle_tx {
            let _ = tx.send(buffer);
        }
    }

    pub fn disk_position(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }

    pub fn poll_block(&mut self) -> io::Result<PollResult> {
        let receiver = match &self.receiver {
            Some(r) => r,
            None => return Ok(PollResult::Done),
        };

        match receiver.recv_timeout(POLL_TIMEOUT) {
            Ok(ScanMessage::Block {
                offset,
                buffer,
                bytes_read,
            }) => Ok(PollResult::Block(OwnedBlock {
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
                Ok(PollResult::Done)
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(PollResult::Pending),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                self.finish();
                Ok(PollResult::Done)
            }
        }
    }
}

fn io_producer(
    reader: DiskReader,
    data_tx: crossbeam_channel::Sender<ScanMessage>,
    recycle_rx: crossbeam_channel::Receiver<AlignedBuffer>,
    progress: Arc<AtomicU64>,
) -> IoThreadResult {
    let mut current_offset: u64 = 0;
    let mut bad_sectors = Vec::new();
    let mut jump_size = BUFFER_SIZE as u64;
    let mut first_read = true;
    let mut consecutive_failures: u64 = 0;
    let disk_size = reader.size();
    let max_total_bad_sectors = disk_size / MAX_JUMP_SIZE;

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

        if req_tx
            .send(ReadRequest {
                offset: read_offset,
                buffer,
            })
            .is_err()
        {
            break;
        }

        match resp_rx.recv_timeout(READ_TIMEOUT) {
            Ok(ReadResponse {
                buffer,
                result: Ok(n),
            }) if n > 0 => {
                first_read = false;
                if consecutive_failures > 0 {
                    consecutive_failures = 0;
                    jump_size = BUFFER_SIZE as u64;
                }
                current_offset = read_offset + n as u64;
                progress.store(current_offset, Ordering::Relaxed);
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
            Ok(ReadResponse {
                buffer: _,
                result: Ok(_),
            }) => {
                let _ = data_tx.send(ScanMessage::Done);
                break;
            }
            Ok(ReadResponse {
                buffer,
                result: Err(e),
            }) => {
                if !is_recoverable_io_error(&e) {
                    let _ = data_tx.send(ScanMessage::FatalError(e));
                    break;
                }

                bad_sectors.push(current_offset);
                consecutive_failures += 1;
                current_offset += jump_size;
                progress.store(current_offset, Ordering::Relaxed);

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
                progress.store(current_offset, Ordering::Relaxed);

                if consecutive_failures > BAD_SECTOR_BACKOFF_THRESHOLD {
                    jump_size = (jump_size * 2).min(MAX_JUMP_SIZE);
                }

                if consecutive_failures > MAX_CONSECUTIVE_FAILURES
                    || bad_sectors.len() as u64 > max_total_bad_sectors
                {
                    let _ = data_tx.send(ScanMessage::Done);
                    break;
                }

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
