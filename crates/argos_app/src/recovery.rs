//! File recovery module for reconstructing files from scan events.
//!
//! This module processes header/footer events and extracts complete files
//! using a stack-based approach to handle nested files (e.g., JPEG thumbnails).
//!
//! # Performance
//!
//! File extraction is performed in parallel using a dedicated thread pool,
//! preventing I/O from blocking the main event processing loop.

use crate::engine::ScanEvent;
use argos_core::{BlockSource, FileType};
use argos_io::DiskReader;
use crossbeam_channel::{bounded, Sender};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const MIN_FILE_SIZE: u64 = 64 * 1024;
const EXTRACTION_BUFFER_SIZE: usize = 64 * 1024;
const MIN_RESOLUTION: usize = 600;
const FALLBACK_SIZE: u64 = 500 * 1024;
const EXTRACTION_WORKERS: usize = 2;
const EXTRACTION_QUEUE_SIZE: usize = 16;

#[derive(Debug, Clone, Copy)]
struct Candidate {
    offset_start: u64,
    file_type: FileType,
}

/// Job sent to extraction workers.
struct ExtractionJob {
    start: u64,
    size: u64,
    output_path: PathBuf,
    device_path: String,
}

/// Manages file recovery using a stack-based approach.
///
/// The stack structure correctly handles nested files like JPEG thumbnails:
/// - Header1 found → push
/// - Header2 (thumbnail) found → push
/// - Footer2 found → pop, save thumbnail
/// - Footer1 found → pop, save main image
///
/// # Parallel Extraction
///
/// Large files are extracted in background threads to avoid blocking
/// event processing. Atomic counters track completion status.
pub struct RecoveryManager {
    stack: Vec<Candidate>,
    reader: DiskReader,
    output_dir: PathBuf,
    device_path: String,

    // Atomic counters for thread-safe stats updates
    files_recovered: Arc<AtomicU64>,
    files_skipped: Arc<AtomicU64>,

    // Extraction thread pool
    // Option allows explicit drop to signal workers to shut down
    extraction_tx: Option<Sender<ExtractionJob>>,
    extraction_handles: Vec<JoinHandle<()>>,
}

impl RecoveryManager {
    /// Creates a new RecoveryManager with parallel extraction workers.
    pub fn new(device_path: &str, output_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(output_dir)?;
        let reader = DiskReader::new(device_path)?;

        let files_recovered = Arc::new(AtomicU64::new(0));
        let files_skipped = Arc::new(AtomicU64::new(0));

        // Create extraction worker pool
        let (tx, rx) = bounded::<ExtractionJob>(EXTRACTION_QUEUE_SIZE);
        let mut handles = Vec::with_capacity(EXTRACTION_WORKERS);

        for worker_id in 0..EXTRACTION_WORKERS {
            let rx = rx.clone();
            let recovered = Arc::clone(&files_recovered);
            let skipped = Arc::clone(&files_skipped);

            let handle = thread::Builder::new()
                .name(format!("extraction-{}", worker_id))
                .spawn(move || {
                    extraction_worker(rx, recovered, skipped);
                })
                .expect("failed to spawn extraction worker");

            handles.push(handle);
        }

        Ok(Self {
            stack: Vec::new(),
            reader,
            output_dir: output_dir.to_path_buf(),
            device_path: device_path.to_string(),
            files_recovered,
            files_skipped,
            extraction_tx: Some(tx),
            extraction_handles: handles,
        })
    }

    /// Processes a scan event, potentially recovering a file.
    pub fn process_event(&mut self, event: &ScanEvent) {
        match event {
            ScanEvent::HeaderFound { offset, ftype } => {
                self.stack.push(Candidate {
                    offset_start: *offset,
                    file_type: *ftype,
                });
            }
            ScanEvent::FooterFound { offset, ftype } => {
                let should_pop = self
                    .stack
                    .last()
                    .map(|c| c.file_type == *ftype)
                    .unwrap_or(false);

                if should_pop {
                    let candidate = self.stack.pop().expect("stack verified non-empty");
                    self.attempt_recovery(&candidate, *offset, *ftype);
                }
            }
            ScanEvent::WorkerDone => {}
        }
    }

    fn attempt_recovery(&mut self, candidate: &Candidate, footer_offset: u64, ftype: FileType) {
        let footer_size = ftype.footer_size();
        let file_size = footer_offset
            .saturating_sub(candidate.offset_start)
            .saturating_add(footer_size);

        if file_size < MIN_FILE_SIZE {
            self.files_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if file_size > MAX_FILE_SIZE {
            self.files_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let mut header_buf = vec![0u8; 4096];
        let header_read = match self
            .reader
            .read_chunk(candidate.offset_start, &mut header_buf)
        {
            Ok(n) => n,
            Err(e) => {
                eprintln!(
                    "[Recovery] Failed to read header at offset {}: {}",
                    candidate.offset_start, e
                );
                self.files_skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        if header_read > 0 {
            if let Some((width, height)) =
                argos_core::get_image_dimensions(&header_buf[..header_read])
            {
                if width < MIN_RESOLUTION || height < MIN_RESOLUTION {
                    self.files_skipped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            } else if file_size < FALLBACK_SIZE {
                self.files_skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        let extension = ftype.extension();
        let filename = format!(
            "{}_{:016X}.{}",
            ftype.name(),
            candidate.offset_start,
            extension
        );
        let output_path = self.output_dir.join(&filename);

        // Queue job to extraction worker pool (non-blocking)
        let job = ExtractionJob {
            start: candidate.offset_start,
            size: file_size,
            output_path,
            device_path: self.device_path.clone(),
        };

        // This may block if queue is full (backpressure)
        if let Some(tx) = &self.extraction_tx {
            if tx.send(job).is_err() {
                eprintln!("[Recovery] Extraction worker pool shut down unexpectedly");
                self.files_skipped.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            eprintln!("[Recovery] Extraction channel already closed");
            self.files_skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn files_recovered(&self) -> u64 {
        self.files_recovered.load(Ordering::Relaxed)
    }

    pub fn files_skipped(&self) -> u64 {
        self.files_skipped.load(Ordering::Relaxed)
    }

    pub fn pending_candidates(&self) -> usize {
        self.stack.len()
    }

    /// Waits for all pending extraction jobs to complete.
    ///
    /// This method:
    /// 1. Closes the extraction channel (no more jobs accepted)
    /// 2. Waits for all workers to finish their current jobs
    ///
    /// Call this before reading final `files_recovered()`/`files_skipped()` counts.
    #[allow(dead_code)] // Used in tests and available for external callers
    pub fn wait_for_completion(&mut self) {
        // Close the channel to signal workers to shut down
        self.extraction_tx.take();

        // Wait for all workers to finish
        let handles = std::mem::take(&mut self.extraction_handles);
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Drop for RecoveryManager {
    fn drop(&mut self) {
        // CRITICAL: Drop the sender FIRST to signal workers to shut down.
        // Without this, workers wait forever for more jobs = deadlock!
        self.extraction_tx.take();

        // Take the handles so we can join them
        let handles = std::mem::take(&mut self.extraction_handles);

        // Wait for all workers to complete their pending jobs
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Extraction worker thread function.
///
/// Receives jobs from the channel and writes files to disk.
/// Each worker opens its own reader for thread safety.
fn extraction_worker(
    rx: crossbeam_channel::Receiver<ExtractionJob>,
    files_recovered: Arc<AtomicU64>,
    files_skipped: Arc<AtomicU64>,
) {
    for job in rx {
        match save_file_job(&job) {
            Ok(()) => {
                files_recovered.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!(
                    "[Extraction] Failed to save {}: {}",
                    job.output_path.display(),
                    e
                );
                files_skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Performs the actual file extraction.
fn save_file_job(job: &ExtractionJob) -> anyhow::Result<()> {
    let mut reader = DiskReader::new(&job.device_path)?;

    let file = File::create(&job.output_path)?;
    let mut writer = BufWriter::with_capacity(131_072, file);
    let mut remaining = job.size;
    let mut offset = job.start;
    let mut buffer = vec![0u8; EXTRACTION_BUFFER_SIZE];

    while remaining > 0 {
        let to_read = std::cmp::min(remaining as usize, EXTRACTION_BUFFER_SIZE);
        let bytes_read = reader.read_chunk(offset, &mut buffer[..to_read])?;

        if bytes_read == 0 {
            break;
        }

        writer.write_all(&buffer[..bytes_read])?;

        offset += bytes_read as u64;
        remaining -= bytes_read as u64;
    }

    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_stack_based_matching() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let valid_size = 512 * 1024;
        let mut data = Vec::with_capacity(valid_size + 200);

        data.extend_from_slice(&[0x00; 100]);
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE0; valid_size]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        data.extend_from_slice(&[0x00; 100]);

        temp_file.write_all(&data).unwrap();
        temp_file.flush().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: 524391,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 0);

        // Wait for async extraction to complete
        manager.wait_for_completion();
        assert_eq!(manager.files_recovered(), 1);

        let recovered_files: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(recovered_files.len(), 1);
    }

    #[test]
    fn test_nested_files_thumbnail() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let chunk_size = 512 * 1024;
        let mut data = Vec::with_capacity(chunk_size * 3);

        data.extend_from_slice(&[0x00; 100]);
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE0; 1024]);

        let h2_offset = 100 + 3 + 1024;
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE1; chunk_size]);

        let f2_offset = h2_offset + 3 + chunk_size as u64;
        data.extend_from_slice(&[0xFF, 0xD9]);

        data.extend_from_slice(&vec![0xE2; chunk_size]);

        let f1_offset = f2_offset + 2 + chunk_size as u64;
        data.extend_from_slice(&[0xFF, 0xD9]);
        data.extend_from_slice(&[0x00; 100]);

        temp_file.write_all(&data).unwrap();
        temp_file.flush().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);

        manager.process_event(&ScanEvent::HeaderFound {
            offset: h2_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 2);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f2_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f1_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 0);

        // Wait for async extraction to complete
        manager.wait_for_completion();
        assert_eq!(manager.files_recovered(), 2);
    }

    #[test]
    fn test_orphan_footer_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.stack.len(), 0);
        assert_eq!(manager.files_recovered(), 0);
    }

    #[test]
    fn test_mismatched_types_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 0,
            ftype: FileType::Jpeg,
        });

        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Png,
        });
        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Png,
        });

        assert_eq!(manager.stack.len(), 1);
        assert_eq!(manager.files_recovered(), 0);
    }
}
