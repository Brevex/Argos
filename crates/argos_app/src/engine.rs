use argos_core::{io::Reader, BlockSource, FileType, SignatureScanner};
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

const EVENT_CHANNEL_CAPACITY: usize = 1000;

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
        let reader = Reader::new(device_path)?;
        reader.size()
    };

    println!("[Engine] Starting scan on: {}", device_path);
    println!("[Engine] Device size: {}", format_size(device_size, BINARY));
    println!("[Engine] Using {} worker threads", num_workers);

    let pb = Arc::new(ProgressBar::new(device_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:50.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("invalid progress bar template - this is a bug")
            .progress_chars("##-"),
    );

    let (data_tx, data_rx): (Sender<DataChunk>, Receiver<DataChunk>) =
        bounded(DATA_CHANNEL_CAPACITY);
    let (recycle_tx, recycle_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) =
        crossbeam_channel::unbounded();
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

    use crossbeam_channel::RecvTimeoutError;
    use std::time::Duration;

    loop {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
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
            Err(RecvTimeoutError::Timeout) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
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
        println!("\n⚠️  Stopping...");
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

    let mut reader = Reader::new(device_path)?;
    let is_mmap = reader.is_mmap();
    if is_mmap {
        eprintln!("[Producer] Using memory-mapped I/O (zero-copy)");
    }
    let total_size = reader.size();

    const PREFETCH_CHUNKS: usize = 2;

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

        let mut chunk = DataChunk {
            offset,
            data: Arc::new(buffer),
        };

        loop {
            if !running.load(Ordering::SeqCst) {
                break 'outer;
            }
            match data_tx.send_timeout(chunk, Duration::from_millis(50)) {
                Ok(_) => break,
                Err(SendTimeoutError::Timeout(returned)) => {
                    chunk = returned;
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

        if offset >= total_size {
            break;
        }

        if let Reader::Mmap(ref mmap_reader) = reader {
            let prefetch_offset = offset + (CHUNK_SIZE * PREFETCH_CHUNKS) as u64;
            if prefetch_offset < total_size {
                mmap_reader.prefetch(prefetch_offset, CHUNK_SIZE);
            }
        }

        pb.set_position(offset.min(total_size));
    }

    Ok(())
}

fn worker_thread(
    _worker_id: usize,
    data_rx: Receiver<DataChunk>,
    recycle_tx: Sender<Vec<u8>>,
    event_tx: Sender<ScanEvent>,
) {
    let jpeg_scanner = SignatureScanner::jpeg();
    let png_scanner = SignatureScanner::png();

    macro_rules! scan_with {
        ($scanner:expr, $chunk:expr, $event_tx:expr) => {{
            let chunk_offset = $chunk.offset;
            let ftype = $scanner.file_type();

            $scanner.scan_headers_callback(&$chunk.data, |relative_offset| {
                let absolute_offset = chunk_offset + relative_offset as u64;
                let _ = $event_tx.send(ScanEvent::HeaderFound {
                    offset: absolute_offset,
                    ftype,
                });
            });

            $scanner.scan_footers_callback(&$chunk.data, |relative_offset| {
                let absolute_offset = chunk_offset + relative_offset as u64;
                let _ = $event_tx.send(ScanEvent::FooterFound {
                    offset: absolute_offset,
                    ftype,
                });
            });
        }};
    }

    for chunk in data_rx {
        scan_with!(jpeg_scanner, chunk, event_tx);
        scan_with!(png_scanner, chunk, event_tx);

        if let Ok(vec) = Arc::try_unwrap(chunk.data) {
            let _ = recycle_tx.send(vec);
        }
    }

    let _ = event_tx.send(ScanEvent::WorkerDone);
}

use crate::signature_index::SignatureIndex;

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

const MIN_FILE_SIZE: u64 = 64 * 1024;

pub fn run_multipass_scan(
    device_path: &str,
    output_dir: &Path,
    running: Arc<AtomicBool>,
) -> EngineResult<()> {
    let start_time = Instant::now();

    let reader = Reader::new(device_path)?;
    let device_size = reader.size();
    drop(reader);

    println!("[MultiPass] Starting multi-pass scan on: {}", device_path);
    println!(
        "[MultiPass] Device size: {}",
        format_size(device_size, BINARY)
    );

    println!("\n[Pass 1/3] Collecting signatures...");

    let mut index = SignatureIndex::with_capacity(device_size);
    let pass1_start = Instant::now();

    {
        let pb = ProgressBar::new(device_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[Pass 1] [{bar:40.green/black}] {bytes}/{total_bytes}")
                .expect("invalid template")
                .progress_chars("=>-"),
        );

        collect_signatures(device_path, &mut index, &pb, &running)?;
        pb.finish_and_clear();
    }

    if !running.load(Ordering::SeqCst) {
        println!("\n⚠️  Scan cancelled during Pass 1");
        return Ok(());
    }

    index.finalize();
    let stats = index.stats();
    println!(
        "[Pass 1] Complete in {:.1}s - {}",
        pass1_start.elapsed().as_secs_f64(),
        stats
    );

    println!("\n[Pass 2/3] Matching contiguous files...");
    let pass2_start = Instant::now();

    let jpeg_candidates: Vec<_> = index.jpeg_candidates(MAX_FILE_SIZE).collect();
    let png_candidates: Vec<_> = index.png_candidates(MAX_FILE_SIZE).collect();
    let total_candidates = jpeg_candidates.len() + png_candidates.len();

    println!(
        "[Pass 2] Found {} potential contiguous files",
        total_candidates
    );

    let mut recovery_manager = crate::recovery::RecoveryManager::new(device_path, output_dir)?;
    let mut recovered = 0u64;
    let mut skipped = 0u64;

    {
        let pb = ProgressBar::new(total_candidates as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[Pass 2] [{bar:40.yellow/black}] {pos}/{len} candidates")
                .expect("invalid template")
                .progress_chars("=>-"),
        );

        for candidate in jpeg_candidates.iter().chain(png_candidates.iter()) {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            let size = candidate.estimated_size();
            if size < MIN_FILE_SIZE || size > MAX_FILE_SIZE {
                skipped += 1;
                pb.inc(1);
                continue;
            }

            recovery_manager.process_event(&ScanEvent::HeaderFound {
                offset: candidate.header_offset,
                ftype: candidate.file_type,
            });
            recovery_manager.process_event(&ScanEvent::FooterFound {
                offset: candidate.footer_offset,
                ftype: candidate.file_type,
            });

            recovered += 1;
            pb.inc(1);
        }

        pb.finish_and_clear();
    }

    println!(
        "[Pass 2] Complete in {:.1}s - {} candidates processed, {} skipped",
        pass2_start.elapsed().as_secs_f64(),
        recovered,
        skipped
    );

    println!("\n[Pass 3/3] Fragment carving for orphan headers...");
    let pass3_start = Instant::now();

    let jpeg_orphans = index.orphan_headers(FileType::Jpeg, MAX_FILE_SIZE);
    let png_orphans = index.orphan_headers(FileType::Png, MAX_FILE_SIZE);
    let total_orphans = jpeg_orphans.len() + png_orphans.len();

    println!(
        "[Pass 3] Found {} orphan headers for fragment carving",
        total_orphans
    );

    println!(
        "[Pass 3] Complete in {:.1}s - {} orphans identified (carving pending)",
        pass3_start.elapsed().as_secs_f64(),
        total_orphans
    );

    let elapsed = start_time.elapsed();

    println!("\n╔════════════════════════════════════════╗");
    println!("║     === Multi-Pass Scan Complete ===   ║");
    println!("╠════════════════════════════════════════╣");
    println!(
        "║ Total Time:         {:>18} ║",
        format!("{:.1}s", elapsed.as_secs_f64())
    );
    println!("║ JPEG Headers:       {:>18} ║", stats.jpeg_headers);
    println!("║ JPEG Footers:       {:>18} ║", stats.jpeg_footers);
    println!("║ PNG Headers:        {:>18} ║", stats.png_headers);
    println!("║ PNG Footers:        {:>18} ║", stats.png_footers);
    println!("║ Contiguous Files:   {:>18} ║", recovered);
    println!("║ Orphan Headers:     {:>18} ║", total_orphans);
    println!(
        "║ Files Recovered:    {:>18} ║",
        recovery_manager.files_recovered()
    );
    println!("╚════════════════════════════════════════╝");

    Ok(())
}

fn collect_signatures(
    device_path: &str,
    index: &mut SignatureIndex,
    pb: &ProgressBar,
    running: &AtomicBool,
) -> EngineResult<()> {
    let mut reader = Reader::new(device_path)?;
    let total_size = reader.size();

    let jpeg_scanner = SignatureScanner::jpeg();
    let png_scanner = SignatureScanner::png();

    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut offset: u64 = 0;

    while offset < total_size {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        let bytes_read = reader.read_chunk(offset, &mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        let chunk = &buffer[..bytes_read];

        jpeg_scanner.scan_headers_callback(chunk, |rel_offset| {
            if rel_offset + 4 <= bytes_read {
                let header_data = &chunk[rel_offset..];
                if argos_core::jpeg::quick_validate_header(header_data) {
                    index.add_header(offset + rel_offset as u64, FileType::Jpeg);
                }
            }
        });
        jpeg_scanner.scan_footers_callback(chunk, |rel_offset| {
            index.add_footer(offset + rel_offset as u64, FileType::Jpeg);
        });

        png_scanner.scan_headers_callback(chunk, |rel_offset| {
            if rel_offset + 16 <= bytes_read {
                let header_data = &chunk[rel_offset..];
                if argos_core::png::quick_validate_header(header_data) {
                    index.add_header(offset + rel_offset as u64, FileType::Png);
                }
            }
        });
        png_scanner.scan_footers_callback(chunk, |rel_offset| {
            index.add_footer(offset + rel_offset as u64, FileType::Png);
        });

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

#[cfg(test)]
mod stress_tests {
    use crate::engine::{DataChunk, ScanEvent, DATA_CHANNEL_CAPACITY, EVENT_CHANNEL_CAPACITY};
    use crossbeam_channel::{bounded, Receiver, Sender};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn test_deadlock_scenario() {
        let num_workers = 8;
        let total_chunks = 1000;
        let running = Arc::new(AtomicBool::new(true));

        let (data_tx, data_rx): (Sender<DataChunk>, Receiver<DataChunk>) =
            bounded(DATA_CHANNEL_CAPACITY);

        let (recycle_tx, recycle_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) =
            crossbeam_channel::unbounded();

        let (event_tx, event_rx): (Sender<ScanEvent>, Receiver<ScanEvent>) =
            bounded(EVENT_CHANNEL_CAPACITY);

        let producer_running = running.clone();
        let producer_handle = std::thread::spawn(move || {
            let mut _sent_count = 0;
            for i in 0..total_chunks {
                let buffer = match recycle_rx.try_recv() {
                    Ok(mut buf) => {
                        buf.resize(1024, 0);
                        buf
                    }
                    Err(_) => vec![0u8; 1024],
                };

                let chunk = DataChunk {
                    offset: i as u64 * 1024,
                    data: Arc::new(buffer),
                };

                let mut chunk_to_send = chunk;
                loop {
                    match data_tx.send_timeout(chunk_to_send, Duration::from_millis(10)) {
                        Ok(_) => break,
                        Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => {
                            chunk_to_send = returned;
                            if !producer_running.load(Ordering::SeqCst) {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
                _sent_count += 1;
            }
        });

        let mut worker_handles = Vec::new();
        for _ in 0..num_workers {
            let rx = data_rx.clone();
            let recycle_tx = recycle_tx.clone();
            let tx = event_tx.clone();

            worker_handles.push(std::thread::spawn(move || {
                for chunk in rx {
                    std::thread::sleep(Duration::from_millis(1));

                    if let Ok(vec) = Arc::try_unwrap(chunk.data) {
                        let _ = recycle_tx.send(vec);
                    }
                }
                let _ = tx.send(ScanEvent::WorkerDone);
            }));
        }

        drop(data_rx);
        drop(recycle_tx);
        drop(event_tx);

        let mut workers_done = 0;
        let start = Instant::now();

        loop {
            match event_rx.recv_timeout(Duration::from_secs(1)) {
                Ok(ScanEvent::WorkerDone) => {
                    workers_done += 1;
                    if workers_done == num_workers {
                        break;
                    }
                }
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if start.elapsed() > Duration::from_secs(10) {
                        panic!("TEST TIMED OUT due to deadlock!");
                    }
                }
                Err(_) => break,
            }
        }

        producer_handle.join().unwrap();
        for h in worker_handles {
            h.join().unwrap();
        }
    }
}
