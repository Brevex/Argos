use crate::engine::ScanEvent;
use argos_core::{
    carving::{CarveDecision, SkipReason, SmartCarver, SmartCarverConfig},
    io::DiskReader,
    statistics::{compute_entropy, ImageClassification},
    FileType,
};
use chrono::Utc;
use crossbeam_channel::{bounded, Sender};
use dashmap::DashSet;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use xxhash_rust::xxh3::xxh3_64;

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const MIN_FILE_SIZE: u64 = 64 * 1024;
const EXTRACTION_BUFFER_SIZE: usize = 64 * 1024;
const MIN_RESOLUTION: usize = 600;
const FALLBACK_SIZE: u64 = 1024 * 1024;
const EXTRACTION_WORKERS: usize = 2;
const EXTRACTION_QUEUE_SIZE: usize = 16;
const MAX_HEADER_DISTANCE: u64 = 200 * 1024 * 1024;
const MIN_ENTROPY: f64 = 6.0;
const MAX_ENTROPY: f64 = 7.99;
const ENTROPY_SAMPLE_SIZE: usize = 4096;
const ENTROPY_SAMPLE_COUNT: usize = 3;
const HASH_SAMPLE_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct Candidate {
    offset_start: u64,
    file_type: FileType,
}

struct ExtractionJob {
    start: u64,
    size: u64,
    output_path: PathBuf,
    device_path: Arc<str>,
    file_type: FileType,
}

#[derive(Debug, Serialize)]
struct ChainOfCustody {
    filename: String,
    source_offset: String,
    source_offset_decimal: u64,
    file_size: u64,
    sha256_hash: String,
    recovery_timestamp: String,
    file_type: String,
    is_fragmented: bool,
    fragment_count: usize,
}

pub struct RecoveryManager {
    stack: Vec<Candidate>,
    reader: DiskReader,
    output_dir: PathBuf,
    device_path: Arc<str>,
    files_recovered: Arc<AtomicU64>,
    files_skipped: Arc<AtomicU64>,
    headers_pruned: Arc<AtomicU64>,
    #[allow(dead_code)]
    seen_hashes: Arc<DashSet<u64>>,
    extraction_tx: Option<Sender<ExtractionJob>>,
    extraction_handles: Vec<JoinHandle<()>>,
}

impl RecoveryManager {
    pub fn new(device_path: &str, output_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(output_dir)?;
        let reader = DiskReader::new(device_path)?;

        let files_recovered = Arc::new(AtomicU64::new(0));
        let files_skipped = Arc::new(AtomicU64::new(0));
        let headers_pruned = Arc::new(AtomicU64::new(0));
        let seen_hashes = Arc::new(DashSet::new());

        let (tx, rx) = bounded::<ExtractionJob>(EXTRACTION_QUEUE_SIZE);
        let mut handles = Vec::with_capacity(EXTRACTION_WORKERS);

        for worker_id in 0..EXTRACTION_WORKERS {
            let rx = rx.clone();
            let recovered = Arc::clone(&files_recovered);
            let skipped = Arc::clone(&files_skipped);
            let hashes = Arc::clone(&seen_hashes);

            let handle = thread::Builder::new()
                .name(format!("extraction-{}", worker_id))
                .spawn(move || {
                    extraction_worker(rx, recovered, skipped, hashes);
                })
                .expect("failed to spawn extraction worker");

            handles.push(handle);
        }

        Ok(Self {
            stack: Vec::new(),
            reader,
            output_dir: output_dir.to_path_buf(),
            device_path: Arc::from(device_path),
            files_recovered,
            files_skipped,
            headers_pruned,
            seen_hashes,
            extraction_tx: Some(tx),
            extraction_handles: handles,
        })
    }

    pub fn process_event(&mut self, event: &ScanEvent) {
        match event {
            ScanEvent::HeaderFound { offset, ftype } => {
                let prune_threshold = offset.saturating_sub(MAX_HEADER_DISTANCE);
                let original_len = self.stack.len();
                self.stack.retain(|c| c.offset_start >= prune_threshold);
                let pruned = original_len - self.stack.len();
                if pruned > 0 {
                    self.headers_pruned
                        .fetch_add(pruned as u64, Ordering::Relaxed);
                }

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

        let header_read_size = 4096.min(file_size as usize);
        let mut header_buf = vec![0u8; header_read_size];
        let header_read = match self
            .reader
            .read_into(candidate.offset_start, &mut header_buf)
        {
            Ok(n) if n > 0 => n,
            _ => {
                self.files_skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        header_buf.truncate(header_read);

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

        let job = ExtractionJob {
            start: candidate.offset_start,
            size: file_size,
            output_path,
            device_path: Arc::clone(&self.device_path),
            file_type: ftype,
        };

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

    #[allow(dead_code)]
    pub fn headers_pruned(&self) -> u64 {
        self.headers_pruned.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn wait_for_completion(&mut self) {
        self.extraction_tx.take();
        let handles = std::mem::take(&mut self.extraction_handles);
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Drop for RecoveryManager {
    fn drop(&mut self) {
        self.extraction_tx.take();
        let handles = std::mem::take(&mut self.extraction_handles);
        for handle in handles {
            let _ = handle.join();
        }
    }
}

fn extraction_worker(
    rx: crossbeam_channel::Receiver<ExtractionJob>,
    files_recovered: Arc<AtomicU64>,
    files_skipped: Arc<AtomicU64>,
    seen_hashes: Arc<DashSet<u64>>,
) {
    let smart_carver = SmartCarver::with_config(SmartCarverConfig {
        structural_validation: true,
        bifragment_carving: true,
        statistical_filtering: true,
        filter_thumbnails: true,
        filter_graphics: true,
        ..Default::default()
    });

    let mut reusable_buffer = Vec::with_capacity(MAX_FILE_SIZE as usize);

    for job in rx {
        reusable_buffer.clear();
        reusable_buffer.reserve(job.size as usize);

        match save_file_job_with_buffer(&job, &mut reusable_buffer, &seen_hashes, &smart_carver) {
            Ok(true) => {
                files_recovered.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {
                files_skipped.fetch_add(1, Ordering::Relaxed);
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

fn compute_partial_hash(data: &[u8]) -> u64 {
    if data.len() <= HASH_SAMPLE_SIZE * 2 {
        xxh3_64(data)
    } else {
        let mut hasher_input = Vec::with_capacity(HASH_SAMPLE_SIZE * 2);
        hasher_input.extend_from_slice(&data[..HASH_SAMPLE_SIZE]);
        hasher_input.extend_from_slice(&data[data.len() - HASH_SAMPLE_SIZE..]);
        xxh3_64(&hasher_input)
    }
}

fn sample_entropy(data: &[u8]) -> f64 {
    if data.len() < ENTROPY_SAMPLE_SIZE {
        return compute_entropy(data);
    }

    let mut total_entropy = 0.0;
    let sample_positions = if data.len() < ENTROPY_SAMPLE_SIZE * ENTROPY_SAMPLE_COUNT {
        vec![0]
    } else {
        let step = (data.len() - ENTROPY_SAMPLE_SIZE) / (ENTROPY_SAMPLE_COUNT - 1);
        (0..ENTROPY_SAMPLE_COUNT).map(|i| i * step).collect()
    };

    for pos in &sample_positions {
        let end = (*pos + ENTROPY_SAMPLE_SIZE).min(data.len());
        total_entropy += compute_entropy(&data[*pos..end]);
    }

    total_entropy / sample_positions.len() as f64
}

fn validate_image_decode(path: &Path, file_type: FileType) -> bool {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let reader = BufReader::new(file);

    let format = match file_type {
        FileType::Jpeg => image::ImageFormat::Jpeg,
        FileType::Png => image::ImageFormat::Png,
        _ => return true,
    };

    image::load(reader, format).is_ok()
}
fn save_file_job_with_buffer(
    job: &ExtractionJob,
    file_data: &mut Vec<u8>,
    seen_hashes: &DashSet<u64>,
    smart_carver: &SmartCarver,
) -> anyhow::Result<bool> {
    let reader = DiskReader::new(&*job.device_path)?;
    let mut total_read = 0;
    let mut offset = job.start;

    file_data.clear();
    file_data.reserve(job.size as usize);

    while total_read < job.size as usize {
        let to_read = std::cmp::min(job.size as usize - total_read, EXTRACTION_BUFFER_SIZE);

        let mut buffer = vec![0u8; to_read];
        let bytes_read = match reader.read_into(offset, &mut buffer) {
            Ok(n) if n > 0 => n,
            Ok(_) => break,
            Err(e) => {
                eprintln!(
                    "[Recovery] I/O error reading at offset 0x{:016X}: {} - filling with zeros",
                    offset, e
                );
                let zeros = vec![0u8; to_read];
                file_data.extend_from_slice(&zeros);
                total_read += to_read;
                offset += to_read as u64;
                continue;
            }
        };
        buffer.truncate(bytes_read);

        file_data.extend_from_slice(&buffer);

        total_read += bytes_read;
        offset += bytes_read as u64;
    }

    let hash = compute_partial_hash(file_data);
    if !seen_hashes.insert(hash) {
        return Ok(false);
    }

    let entropy = sample_entropy(file_data);
    if !(MIN_ENTROPY..=MAX_ENTROPY).contains(&entropy) {
        seen_hashes.remove(&hash);
        return Ok(false);
    }

    let analysis_result = match job.file_type {
        FileType::Jpeg => smart_carver.analyze_jpeg(file_data, job.start, &reader),
        FileType::Png => smart_carver.analyze_png(file_data, job.start),
        _ => {
            let sha256_hash = compute_sha256(file_data);
            let file = File::create(&job.output_path)?;
            let mut writer = BufWriter::with_capacity(131_072, file);
            writer.write_all(file_data)?;
            writer.flush()?;

            write_chain_of_custody(
                &job.output_path,
                job.start,
                file_data.len() as u64,
                &sha256_hash,
                job.file_type,
                false,
                1,
            )?;

            return Ok(true);
        }
    };

    match analysis_result.decision {
        CarveDecision::Skip(SkipReason::InvalidStructure) => {
            seen_hashes.remove(&hash);
            return Ok(false);
        }
        CarveDecision::Skip(SkipReason::Thumbnail) => {
            seen_hashes.remove(&hash);
            return Ok(false);
        }
        CarveDecision::Extract | CarveDecision::ExtractPartial | CarveDecision::AttemptBgc => {}
        CarveDecision::Skip(_) => {
            seen_hashes.remove(&hash);
            return Ok(false);
        }
    }

    let mut hasher = Sha256::new();
    let file = File::create(&job.output_path)?;
    let mut writer = BufWriter::with_capacity(131_072, file);
    let mut total_written: u64 = 0;
    let mut is_fragmented = false;
    let mut fragment_count = 1usize;

    if let Some(ref mf_result) = analysis_result.multi_fragment_result {
        if mf_result.fragments.len() > 1 {
            is_fragmented = true;
            fragment_count = mf_result.fragments.len();

            for fragment in &mf_result.fragments {
                let mut frag_offset = fragment.offset;
                let mut remaining = fragment.size as usize;
                while remaining > 0 {
                    let to_read = remaining.min(EXTRACTION_BUFFER_SIZE);
                    let mut buffer = vec![0u8; to_read];
                    let bytes_read = match reader.read_into(frag_offset, &mut buffer) {
                        Ok(n) if n > 0 => n,
                        Ok(_) => break,
                        Err(e) => {
                            eprintln!(
                                "[Recovery] Fragment read error at 0x{:016X}: {}",
                                frag_offset, e
                            );
                            break;
                        }
                    };
                    buffer.truncate(bytes_read);
                    hasher.update(&buffer);
                    writer.write_all(&buffer)?;
                    total_written += bytes_read as u64;
                    frag_offset += bytes_read as u64;
                    remaining = remaining.saturating_sub(bytes_read);
                }
            }
            writer.flush()?;
        } else {
            hasher.update(file_data.as_slice());
            writer.write_all(file_data)?;
            writer.flush()?;
            total_written = file_data.len() as u64;
        }
    } else if let Some(ref bgc_result) = analysis_result.bgc_result {
        if bgc_result.is_fragmented {
            is_fragmented = true;
            fragment_count = 2;

            let head_data = &file_data[..bgc_result.head_size as usize];
            hasher.update(head_data);
            writer.write_all(head_data)?;
            total_written += head_data.len() as u64;

            if let (Some(tail_offset), Some(tail_size)) =
                (bgc_result.tail_offset, bgc_result.tail_size)
            {
                let mut tail_buffer = vec![0u8; tail_size as usize];
                match reader.read_into(tail_offset, &mut tail_buffer) {
                    Ok(n) if n > 0 => {
                        tail_buffer.truncate(n);
                        hasher.update(&tail_buffer);
                        writer.write_all(&tail_buffer)?;
                        total_written += n as u64;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!(
                            "[Recovery] Tail read error at 0x{:016X}: {}",
                            tail_offset, e
                        );
                    }
                }
            }
            writer.flush()?;
        } else {
            hasher.update(file_data.as_slice());
            writer.write_all(file_data)?;
            writer.flush()?;
            total_written = file_data.len() as u64;
        }
    } else {
        hasher.update(file_data.as_slice());
        writer.write_all(file_data)?;
        writer.flush()?;
        total_written = file_data.len() as u64;
    }

    let sha256_hash = format!("{:x}", hasher.finalize());

    if !validate_image_decode(&job.output_path, job.file_type) {
        let _ = fs::remove_file(&job.output_path);
        seen_hashes.remove(&hash);
        return Ok(false);
    }

    if smart_carver.config().filter_graphics
        && should_filter_as_graphic(&job.output_path, job.file_type, smart_carver)
    {
        let _ = fs::remove_file(&job.output_path);
        seen_hashes.remove(&hash);
        return Ok(false);
    }

    write_chain_of_custody(
        &job.output_path,
        job.start,
        total_written,
        &sha256_hash,
        job.file_type,
        is_fragmented,
        fragment_count,
    )?;

    Ok(true)
}

fn write_chain_of_custody(
    image_path: &Path,
    source_offset: u64,
    file_size: u64,
    sha256_hash: &str,
    file_type: FileType,
    is_fragmented: bool,
    fragment_count: usize,
) -> anyhow::Result<()> {
    let filename = image_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let custody = ChainOfCustody {
        filename: filename.clone(),
        source_offset: format!("0x{:016X}", source_offset),
        source_offset_decimal: source_offset,
        file_size,
        sha256_hash: sha256_hash.to_string(),
        recovery_timestamp: Utc::now().to_rfc3339(),
        file_type: file_type.name().to_string(),
        is_fragmented,
        fragment_count,
    };

    let sidecar_path = image_path.with_extension(format!(
        "{}.custody.json",
        image_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
    ));

    let json = serde_json::to_string_pretty(&custody)?;
    fs::write(&sidecar_path, json)?;

    Ok(())
}

fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn should_filter_as_graphic(path: &Path, file_type: FileType, smart_carver: &SmartCarver) -> bool {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let reader = BufReader::new(file);
    let format = match file_type {
        FileType::Jpeg => image::ImageFormat::Jpeg,
        FileType::Png => image::ImageFormat::Png,
        _ => return false,
    };

    let img = match image::load(reader, format) {
        Ok(i) => i,
        Err(_) => return false,
    };

    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let pixels = rgb.as_raw();

    let (classification, _stats) =
        smart_carver.classify_image(pixels, width as usize, height as usize, 3);

    matches!(
        classification,
        ImageClassification::ArtificialGraphic | ImageClassification::Encrypted
    )
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

        manager.wait_for_completion();

        let total_processed = manager.files_recovered() + manager.files_skipped();
        assert_eq!(
            total_processed, 1,
            "file should have been processed (recovered or skipped)"
        );
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

        manager.wait_for_completion();

        let total_processed = manager.files_recovered() + manager.files_skipped();
        assert_eq!(total_processed, 2, "both files should have been processed");
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
