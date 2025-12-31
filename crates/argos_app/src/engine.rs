//! Concurrent scanning engine for forensic image recovery.
//!
//! This module implements a producer-consumer pipeline that reads disk chunks
//! in one thread and distributes them to multiple worker threads for scanning.

use argos_core::{BlockSource, FileScanner, FileType, JpegScanner, PngScanner};
use argos_io::DiskReader;
use crossbeam_channel::{bounded, Receiver, Sender};
use humansize::{format_size, BINARY};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use crate::recovery::RecoveryManager;

const CHUNK_SIZE: usize = 4 * 1024 * 1024;
const OVERLAP: usize = 4 * 1024;
const DATA_CHANNEL_CAPACITY: usize = 10;
const RECYCLE_CHANNEL_CAPACITY: usize = DATA_CHANNEL_CAPACITY + 2;
const EVENT_CHANNEL_CAPACITY: usize = 1000;

/// A chunk of data read from the disk.
///
/// Uses `Arc<Vec<u8>>` for the data buffer to enable cheap cloning (O(1))
/// when resending on channel timeout, avoiding expensive 4MB copies.
#[derive(Debug, Clone)]
pub struct DataChunk {
    pub offset: u64,
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ScanEvent {
    HeaderFound { offset: u64, ftype: FileType },

    FooterFound { offset: u64, ftype: FileType },

    WorkerDone,
}

pub type EngineResult<T> = anyhow::Result<T>;

pub fn run_scan(
    device_path: &str,
    output_dir: &Path,
    running: Arc<AtomicBool>,
) -> EngineResult<()> {
    let start_time = Instant::now();
    let num_workers = num_cpus::get();

    let device_size = {
        let reader = DiskReader::new(device_path)?;
        reader.size()
    };

    println!("[Engine] Starting scan on: {}", device_path);
    println!("[Engine] Device size: {}", format_size(device_size, BINARY));
    println!("[Engine] Using {} worker threads", num_workers);

    let pb = Arc::new(ProgressBar::new(device_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:50.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let (data_tx, data_rx): (Sender<DataChunk>, Receiver<DataChunk>) =
        bounded(DATA_CHANNEL_CAPACITY);
    let (recycle_tx, recycle_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) =
        bounded(RECYCLE_CHANNEL_CAPACITY);
    let (event_tx, event_rx): (Sender<ScanEvent>, Receiver<ScanEvent>) =
        bounded(EVENT_CHANNEL_CAPACITY);
    let device_path_owned = device_path.to_string();
    let pb_producer = Arc::clone(&pb);
    let running_producer = Arc::clone(&running);

    let producer_handle = thread::spawn(move || {
        if let Err(e) = producer_thread(
            &device_path_owned,
            data_tx,
            recycle_rx,
            pb_producer,
            running_producer,
        ) {
            eprintln!("[Producer] Error: {}", e);
        }
    });

    let mut worker_handles = Vec::with_capacity(num_workers);
    for worker_id in 0..num_workers {
        let rx = data_rx.clone();
        let recycle_tx = recycle_tx.clone();
        let tx = event_tx.clone();

        let handle = thread::spawn(move || {
            worker_thread(worker_id, rx, recycle_tx, tx);
        });

        worker_handles.push(handle);
    }

    drop(data_rx);
    drop(recycle_tx);
    drop(event_tx);

    let mut recovery_manager = RecoveryManager::new(device_path, output_dir)?;

    let mut headers_found = 0u64;
    let mut footers_found = 0u64;
    let mut workers_done = 0usize;

    for event in event_rx {
        match &event {
            ScanEvent::HeaderFound { .. } => {
                headers_found += 1;
            }
            ScanEvent::FooterFound { .. } => {
                footers_found += 1;
            }
            ScanEvent::WorkerDone => {
                workers_done += 1;
            }
        }

        recovery_manager.process_event(&event);

        if let ScanEvent::WorkerDone = event {
            if workers_done == num_workers {
                break;
            }
        }
    }

    if let Err(e) = producer_handle.join() {
        eprintln!("[FATAL] Producer thread panicked: {:?}", e);
    }

    for (i, handle) in worker_handles.into_iter().enumerate() {
        if let Err(e) = handle.join() {
            eprintln!("[FATAL] Worker thread {} panicked: {:?}", i, e);
        }
    }

    let was_cancelled = !running.load(Ordering::SeqCst);

    pb.finish_and_clear();

    if was_cancelled {
        println!("\n⚠️  Received Ctrl+C! Stopping gracefully...");
    }

    let elapsed = start_time.elapsed();

    println!("\n╔════════════════════════════════════════╗");
    if was_cancelled {
        println!("║       === Scan Interrupted ===         ║");
    } else {
        println!("║         === Scan Finished ===          ║");
    }
    println!("╠════════════════════════════════════════╣");
    println!(
        "║ Elapsed Time:       {:>18} ║",
        format!("{:.1}s", elapsed.as_secs_f64())
    );
    println!(
        "║ Scanned Space:      {:>18} ║",
        format_size(pb.position(), BINARY)
    );
    println!("║ Headers Found:      {:>18} ║", headers_found);
    println!("║ Footers Found:      {:>18} ║", footers_found);
    println!(
        "║ Recovered Images:   {:>18} ║",
        recovery_manager.files_recovered()
    );
    println!(
        "║ Skipped Files:      {:>18} ║",
        recovery_manager.files_skipped()
    );
    println!(
        "║ Pending Candidates: {:>17} ║",
        recovery_manager.pending_candidates()
    );
    println!("╠════════════════════════════════════════╣");
    println!("║ Files saved to:     {:<18} ║", output_dir.display());
    println!("╚════════════════════════════════════════╝");

    Ok(())
}

fn producer_thread(
    device_path: &str,
    data_tx: Sender<DataChunk>,
    recycle_rx: Receiver<Vec<u8>>,
    pb: Arc<ProgressBar>,
    running: Arc<AtomicBool>,
) -> EngineResult<()> {
    use crossbeam_channel::SendTimeoutError;
    use std::time::Duration;

    let mut reader = DiskReader::new(device_path)?;
    let total_size = reader.size();

    let mut offset: u64 = 0;

    'outer: while offset < total_size {
        let mut buffer = match recycle_rx.try_recv() {
            Ok(mut buf) => {
                if buf.capacity() < CHUNK_SIZE {
                    buf.reserve(CHUNK_SIZE - buf.len());
                }
                buf.resize(CHUNK_SIZE, 0);
                buf
            }
            Err(_) => vec![0u8; CHUNK_SIZE],
        };

        let bytes_read = reader.read_chunk(offset, &mut buffer)?;

        if bytes_read == 0 {
            break;
        }

        buffer.truncate(bytes_read);

        let chunk = DataChunk {
            offset,
            data: Arc::new(buffer),
        };

        loop {
            match data_tx.send_timeout(chunk.clone(), Duration::from_millis(50)) {
                Ok(_) => break,
                Err(SendTimeoutError::Timeout(_)) => {
                    if !running.load(Ordering::SeqCst) {
                        break 'outer;
                    }
                }
                Err(SendTimeoutError::Disconnected(_)) => break 'outer,
            }
        }

        let advance = if bytes_read >= OVERLAP {
            bytes_read - OVERLAP
        } else {
            bytes_read
        };

        offset += advance as u64;

        pb.set_position(offset.min(total_size));
    }

    Ok(())
}

/// Worker thread: receives data chunks and scans them for signatures.
fn worker_thread(
    _worker_id: usize,
    data_rx: Receiver<DataChunk>,
    recycle_tx: Sender<Vec<u8>>,
    event_tx: Sender<ScanEvent>,
) {
    let jpeg_scanner = JpegScanner::new();
    let png_scanner = PngScanner::new();
    let scanners: Vec<&dyn FileScanner> = vec![&jpeg_scanner, &png_scanner];

    for chunk in data_rx {
        for scanner in &scanners {
            let headers = scanner.scan_headers(&chunk.data);
            for relative_offset in headers {
                let absolute_offset = chunk.offset + relative_offset as u64;

                let _ = event_tx.send(ScanEvent::HeaderFound {
                    offset: absolute_offset,
                    ftype: scanner.file_type(),
                });
            }

            let footers = scanner.scan_footers(&chunk.data);
            for relative_offset in footers {
                let absolute_offset = chunk.offset + relative_offset as u64;

                let _ = event_tx.send(ScanEvent::FooterFound {
                    offset: absolute_offset,
                    ftype: scanner.file_type(),
                });
            }
        }

        if let Ok(vec) = Arc::try_unwrap(chunk.data) {
            let _ = recycle_tx.send(vec);
        }
    }

    let _ = event_tx.send(ScanEvent::WorkerDone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_chunk_overlap_calculation() {
        let chunk_size = 4 * 1024 * 1024;
        let overlap = 4 * 1024;
        let advance = chunk_size - overlap;

        assert_eq!(advance, 4 * 1024 * 1024 - 4 * 1024);
        assert_eq!(advance, 4190208);
    }

    #[test]
    fn test_scan_with_embedded_jpeg() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();

        let mut data = Vec::new();
        data.extend_from_slice(&[0x00; 100]);
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&[0x00; 50]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        data.extend_from_slice(&[0x00; 47]);

        temp_file.write_all(&data).unwrap();
        temp_file.flush().unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = run_scan(temp_file.path().to_str().unwrap(), temp_dir.path(), running);
        assert!(result.is_ok());
    }

    #[test]
    fn test_scan_empty_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let result = run_scan(temp_file.path().to_str().unwrap(), temp_dir.path(), running);
        assert!(result.is_ok());
    }
}
