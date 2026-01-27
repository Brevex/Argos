use crate::engine::ScanEvent;
use argos_core::{
    aligned_buffer::AlignedBuffer,
    carving::{CarveDecision, SkipReason, SmartCarver, SmartCarverConfig},
    error::FileFormat as ErrorFileFormat,
    io::DiskReader,
    matching::{FooterCandidate, GlobalMatcher, HeaderCandidate},
    statistics::compute_entropy,
    validation::{ValidationContext, ValidationPipeline},
    FileType,
};
use chrono::Utc;
use crossbeam_channel::{bounded, Sender};
use dashmap::DashSet;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use xxhash_rust::xxh3::xxh3_64;

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const MIN_FILE_SIZE: u64 = 2 * 1024;
const EXTRACTION_BUFFER_SIZE: usize = 64 * 1024;
const MIN_RESOLUTION: usize = 600;
const EXTRACTION_WORKERS: usize = 2;
const EXTRACTION_QUEUE_SIZE: usize = 16;
const MIN_ENTROPY: f64 = 4.0;
const MAX_ENTROPY: f64 = 8.0;
const ENTROPY_SAMPLE_SIZE: usize = 4096;
const ENTROPY_SAMPLE_COUNT: usize = 3;
const HASH_SAMPLE_SIZE: usize = 64 * 1024;

#[inline]
fn filetype_to_format(ftype: FileType) -> ErrorFileFormat {
    match ftype {
        FileType::Jpeg => ErrorFileFormat::Jpeg,
        FileType::Png => ErrorFileFormat::Png,
        FileType::Unknown => ErrorFileFormat::Unknown,
    }
}

#[inline]
fn format_to_filetype(format: ErrorFileFormat) -> FileType {
    match format {
        ErrorFileFormat::Jpeg => FileType::Jpeg,
        ErrorFileFormat::Png => FileType::Png,
        ErrorFileFormat::Unknown => FileType::Unknown,
    }
}

struct ExtractionJob {
    start: u64,
    size: u64,
    output_path: PathBuf,
    file_type: FileType,

    header_cache: Option<Vec<u8>>,
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
    reader: Arc<DiskReader>,
    output_dir: PathBuf,
    #[allow(dead_code)]
    device_path: Arc<str>,
    files_recovered: Arc<AtomicU64>,
    files_skipped: Arc<AtomicU64>,
    #[allow(dead_code)]
    seen_hashes: Arc<DashSet<u64>>,
    extraction_tx: Option<Sender<ExtractionJob>>,
    extraction_handles: Vec<JoinHandle<()>>,
    #[allow(dead_code)]
    config: crate::engine::UnsafeConfig,
    header_buf: AlignedBuffer,
    global_matcher: GlobalMatcher,
    collected_headers: Vec<HeaderCandidate>,
    collected_footers: Vec<FooterCandidate>,
    #[allow(dead_code)]
    jpeg_validator: ValidationPipeline,
    #[allow(dead_code)]
    png_validator: ValidationPipeline,
    streaming_mode: bool,
}

impl RecoveryManager {
    pub fn new(
        device_path: &str,
        output_dir: &Path,
        config: crate::engine::UnsafeConfig,
    ) -> anyhow::Result<Self> {
        fs::create_dir_all(output_dir)?;

        let reader = Arc::new(DiskReader::new(device_path)?);

        let files_recovered = Arc::new(AtomicU64::new(0));
        let files_skipped = Arc::new(AtomicU64::new(0));
        let _headers_pruned = Arc::new(AtomicU64::new(0));
        let seen_hashes = Arc::new(DashSet::new());

        let (tx, rx) = bounded::<ExtractionJob>(EXTRACTION_QUEUE_SIZE);
        let mut handles = Vec::with_capacity(EXTRACTION_WORKERS);

        for worker_id in 0..EXTRACTION_WORKERS {
            let rx = rx.clone();
            let recovered = Arc::clone(&files_recovered);
            let skipped = Arc::clone(&files_skipped);
            let hashes = Arc::clone(&seen_hashes);
            let shared_reader = Arc::clone(&reader);

            match thread::Builder::new()
                .name(format!("extraction-{}", worker_id))
                .spawn(move || {
                    extraction_worker(rx, recovered, skipped, hashes, shared_reader, config);
                }) {
                Ok(handle) => handles.push(handle),
                Err(e) => {
                    eprintln!("[Recovery] WARNING: Failed to spawn extraction worker {}: {} - degraded performance", worker_id, e);
                }
            }
        }

        if handles.is_empty() {
            anyhow::bail!("Failed to spawn any extraction workers");
        }

        Ok(Self {
            reader,
            output_dir: output_dir.to_path_buf(),
            device_path: Arc::from(device_path),
            files_recovered,
            files_skipped,
            seen_hashes,
            extraction_tx: Some(tx),
            extraction_handles: handles,
            config,
            header_buf: AlignedBuffer::new_default(4096),
            global_matcher: GlobalMatcher::new(),
            collected_headers: Vec::new(),
            collected_footers: Vec::new(),
            jpeg_validator: ValidationPipeline::for_jpeg_with_rendering(),
            png_validator: ValidationPipeline::for_png_with_rendering(),
            streaming_mode: false,
        })
    }

    pub fn process_event(&mut self, event: &ScanEvent) {
        match event {
            ScanEvent::HeaderFound { offset, ftype } => {
                self.collected_headers.push(HeaderCandidate {
                    offset: *offset,
                    format: filetype_to_format(*ftype),
                    quality: 0.9,
                    dimensions: None,
                    expected_size_range: None,
                });
            }
            ScanEvent::FooterFound { offset, ftype } => {
                self.collected_footers.push(FooterCandidate {
                    offset: *offset,
                    format: filetype_to_format(*ftype),
                    quality: 0.9,
                });

                if self.streaming_mode {
                    self.try_immediate_match(*offset, *ftype);
                }
            }
            ScanEvent::WorkerDone => {
                if !self.streaming_mode {
                    self.run_global_matching();
                }
            }
        }
    }

    fn try_immediate_match(&mut self, footer_offset: u64, ftype: FileType) {
        let footer_format = filetype_to_format(ftype);

        let mut best_header_idx = None;
        let mut best_score = 0.0f64;

        for (idx, header) in self.collected_headers.iter().enumerate() {
            if header.format != footer_format {
                continue;
            }

            if footer_offset <= header.offset {
                continue;
            }

            let distance = footer_offset - header.offset;

            if !(MIN_FILE_SIZE..=MAX_FILE_SIZE).contains(&distance) {
                continue;
            }

            let score = self.calculate_match_score(header, footer_offset, distance);

            if score > best_score {
                best_score = score;
                best_header_idx = Some(idx);
            }
        }

        if let Some(idx) = best_header_idx {
            if best_score >= 0.3 {
                let header = self.collected_headers.remove(idx);
                self.attempt_recovery_from_match(&header, footer_offset, ftype);
            }
        }
    }

    fn calculate_match_score(
        &self,
        header: &HeaderCandidate,
        _footer_offset: u64,
        distance: u64,
    ) -> f64 {
        let mut score = 0.5;

        let distance_score = 1.0 / (1.0 + (distance as f64 / 1_000_000.0).ln().max(0.0));
        score += distance_score * 0.2;
        score += header.quality * 0.2;

        if let Some((min_size, max_size)) = header.expected_size_range {
            if distance >= min_size && distance <= max_size {
                score += 0.3;
            }
        }

        score.min(1.0)
    }

    fn attempt_recovery_from_match(
        &mut self,
        header: &HeaderCandidate,
        footer_offset: u64,
        ftype: FileType,
    ) {
        let footer_size = ftype.footer_size();
        let file_size = footer_offset
            .saturating_sub(header.offset)
            .saturating_add(footer_size);

        if !(MIN_FILE_SIZE..=MAX_FILE_SIZE).contains(&file_size) {
            self.files_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let header_read_size = 4096.min(file_size as usize);
        let header_read = match self
            .reader
            .read_into(header.offset, &mut self.header_buf[..header_read_size])
        {
            Ok(n) if n > 0 => n,
            _ => {
                self.files_skipped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        if header_read > 0 {
            if let Some((width, height)) =
                argos_core::get_image_dimensions(&self.header_buf[..header_read])
            {
                if width < MIN_RESOLUTION && height < MIN_RESOLUTION {
                    self.files_skipped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
        }

        let extension = ftype.extension();
        let filename = format!("{}_{:016X}.{}", ftype.name(), header.offset, extension);
        let output_path = self.output_dir.join(&filename);

        let header_cache = if header_read > 0 {
            Some(self.header_buf[..header_read].to_vec())
        } else {
            None
        };

        let job = ExtractionJob {
            start: header.offset,
            size: file_size,
            output_path,
            file_type: ftype,
            header_cache,
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

    fn run_global_matching(&mut self) {
        if self.collected_headers.is_empty() || self.collected_footers.is_empty() {
            return;
        }

        for header in &self.collected_headers {
            self.global_matcher.add_header(header.clone());
        }
        for footer in &self.collected_footers {
            self.global_matcher.add_footer(footer.clone());
        }

        let matches = if self.collected_headers.len() < 500 {
            self.global_matcher.solve_optimal()
        } else {
            self.global_matcher.solve_greedy()
        };

        let match_data: Vec<_> = matches
            .iter()
            .filter_map(|result| {
                let header = self.global_matcher.get_header(result.header_idx)?;
                let footer = self.global_matcher.get_footer(result.footer_idx)?;
                Some((
                    header.clone(),
                    footer.offset,
                    format_to_filetype(header.format),
                ))
            })
            .collect();

        for (header, footer_offset, ftype) in match_data {
            self.attempt_recovery_from_match(&header, footer_offset, ftype);
        }

        self.collected_headers.clear();
        self.collected_footers.clear();
    }

    #[allow(dead_code)]
    pub fn finalize(&mut self) {
        if !self.collected_headers.is_empty() && !self.collected_footers.is_empty() {
            self.run_global_matching();
        }
    }

    pub fn files_recovered(&self) -> u64 {
        self.files_recovered.load(Ordering::Relaxed)
    }

    pub fn files_skipped(&self) -> u64 {
        self.files_skipped.load(Ordering::Relaxed)
    }

    pub fn pending_candidates(&self) -> usize {
        self.collected_headers.len()
    }

    #[allow(dead_code)]
    pub fn collected_headers_count(&self) -> usize {
        self.collected_headers.len()
    }

    #[allow(dead_code)]
    pub fn collected_footers_count(&self) -> usize {
        self.collected_footers.len()
    }

    #[allow(dead_code)]
    pub fn set_streaming_mode(&mut self, enabled: bool) {
        self.streaming_mode = enabled;
    }

    #[allow(dead_code)]
    pub fn wait_for_completion(&mut self) {
        self.finalize();

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
    shared_reader: Arc<DiskReader>,
    config: crate::engine::UnsafeConfig,
) {
    let smart_carver = SmartCarver::with_config(SmartCarverConfig {
        structural_validation: true,
        bifragment_carving: true,
        statistical_filtering: true,
        filter_thumbnails: true,
        filter_graphics: true,
        ..Default::default()
    });

    let mut debug_file = if config.debug {
        Some(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/argos_debug.log")
                .ok()
                .map(BufWriter::new),
        )
    } else {
        None
    };

    let mut reusable_buffer = AlignedBuffer::new_default(EXTRACTION_BUFFER_SIZE);

    for job in rx {
        match save_file_streaming(
            &job,
            &mut reusable_buffer,
            &seen_hashes,
            &smart_carver,
            &shared_reader,
            config,
            &mut debug_file,
        ) {
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
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };

    let format = filetype_to_format(file_type);
    let pipeline = ValidationPipeline::for_format(format);
    let ctx = ValidationContext {
        format,
        expected_size: Some(data.len() as u64),
        is_fragmented: false,
        fragment_count: 1,
    };

    let result = pipeline.validate(&data, &ctx);

    result.passed && result.confidence >= 0.5
}

#[allow(clippy::too_many_arguments)]
fn save_file_streaming(
    job: &ExtractionJob,
    buffer: &mut [u8],
    seen_hashes: &DashSet<u64>,
    smart_carver: &SmartCarver,
    reader: &DiskReader,
    config: crate::engine::UnsafeConfig,
    debug_file: &mut Option<Option<BufWriter<File>>>,
) -> anyhow::Result<bool> {
    macro_rules! debug_log {
        ($($arg:tt)*) => {
            if let Some(Some(ref mut file)) = debug_file {
                let _ = writeln!(file, $($arg)*);
                let _ = file.flush();
            }
        };
    }

    let header_data = job.header_cache.as_deref();

    if let Some(header) = header_data {
        match job.file_type {
            FileType::Jpeg => {
                if header.len() < 2 || header[0] != 0xFF || header[1] != 0xD8 {
                    debug_log!(
                        "[SKIP] 0x{:016X} {} - Invalid JPEG header",
                        job.start,
                        job.file_type
                    );
                    return Ok(false);
                }
            }
            FileType::Png => {
                if header.len() < 8
                    || !header.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
                {
                    debug_log!(
                        "[SKIP] 0x{:016X} {} - Invalid PNG header",
                        job.start,
                        job.file_type
                    );
                    return Ok(false);
                }
            }
            _ => {}
        }
    }

    let quick_hash = {
        let mut hash_input = Vec::with_capacity(HASH_SAMPLE_SIZE * 2);

        if let Some(header) = header_data {
            let take = header.len().min(HASH_SAMPLE_SIZE);
            hash_input.extend_from_slice(&header[..take]);
        } else {
            let bytes = reader.read_into(job.start, buffer)?;
            let take = bytes.min(HASH_SAMPLE_SIZE);
            hash_input.extend_from_slice(&buffer[..take]);
        }

        if job.size > (HASH_SAMPLE_SIZE * 2) as u64 {
            let tail_offset = job.start + job.size - HASH_SAMPLE_SIZE as u64;
            let bytes = reader.read_into(tail_offset, buffer)?;
            let take = bytes.min(HASH_SAMPLE_SIZE);
            hash_input.extend_from_slice(&buffer[..take]);
        }

        xxh3_64(&hash_input)
    };

    if !seen_hashes.insert(quick_hash) {
        debug_log!("[SKIP] 0x{:016X} {} - Duplicate", job.start, job.file_type);
        return Ok(false);
    }

    let header_entropy = if let Some(header) = header_data {
        compute_entropy(header)
    } else {
        let bytes = reader.read_into(job.start, buffer)?;
        compute_entropy(&buffer[..bytes.min(ENTROPY_SAMPLE_SIZE)])
    };

    if !config.unsafe_mode && !(MIN_ENTROPY..=MAX_ENTROPY).contains(&header_entropy) {
        let reason = if header_entropy < MIN_ENTROPY {
            "EntropyTooLow"
        } else {
            "EntropyTooHigh"
        };
        debug_log!(
            "[SKIP] 0x{:016X} {} - {} (entropy={:.3})",
            job.start,
            job.file_type,
            reason,
            header_entropy
        );
        seen_hashes.remove(&quick_hash);
        return Ok(false);
    }

    let needs_structural_analysis =
        smart_carver.config().structural_validation || smart_carver.config().bifragment_carving;

    const STREAMING_THRESHOLD: u64 = 256 * 1024;

    if job.size <= STREAMING_THRESHOLD || needs_structural_analysis {
        let mut file_data = Vec::with_capacity(job.size as usize);

        let mut offset = job.start;
        if let Some(header) = header_data {
            file_data.extend_from_slice(header);
            offset += header.len() as u64;
        }

        while file_data.len() < job.size as usize {
            let to_read = (job.size as usize - file_data.len()).min(buffer.len());
            let bytes = match reader.read_into(offset, &mut buffer[..to_read]) {
                Ok(n) if n > 0 => n,
                Ok(_) => break,
                Err(e) => {
                    debug_log!("[WARN] 0x{:016X} I/O error: {}", offset, e);
                    break;
                }
            };
            file_data.extend_from_slice(&buffer[..bytes]);
            offset += bytes as u64;
        }

        let analysis = match job.file_type {
            FileType::Jpeg => smart_carver.analyze_jpeg(&file_data, job.start, reader),
            FileType::Png => smart_carver.analyze_png(&file_data, job.start),
            _ => argos_core::carving::SmartCarveResult::extract(
                job.file_type,
                job.start,
                file_data.len() as u64,
            ),
        };

        if let CarveDecision::Skip(ref reason) = analysis.decision {
            let should_reject =
                matches!(reason, SkipReason::InvalidStructure) || !config.unsafe_mode;
            if should_reject {
                debug_log!(
                    "[SKIP] 0x{:016X} {} - {:?}",
                    job.start,
                    job.file_type,
                    reason
                );
                seen_hashes.remove(&quick_hash);
                return Ok(false);
            }
        }

        let file = File::create(&job.output_path)?;
        let mut writer = BufWriter::with_capacity(131_072, file);
        let mut hasher = Sha256::new();

        hasher.update(&file_data);
        writer.write_all(&file_data)?;
        writer.flush()?;

        let sha256 = format!("{:x}", hasher.finalize());

        write_chain_of_custody(
            &job.output_path,
            job.start,
            file_data.len() as u64,
            &sha256,
            job.file_type,
            false,
            1,
        )?;

        return Ok(true);
    }

    let file = File::create(&job.output_path)?;
    let mut writer = BufWriter::with_capacity(131_072, file);
    let mut hasher = Sha256::new();
    let mut total_written: u64 = 0;
    let mut offset = job.start;

    if let Some(header) = header_data {
        hasher.update(header);
        writer.write_all(header)?;
        total_written += header.len() as u64;
        offset += header.len() as u64;
    }

    while total_written < job.size {
        let to_read = ((job.size - total_written) as usize).min(buffer.len());
        let bytes = match reader.read_into(offset, &mut buffer[..to_read]) {
            Ok(n) if n > 0 => n,
            Ok(_) => break,
            Err(e) => {
                eprintln!("[Recovery] I/O error at 0x{:016X}: {}", offset, e);
                break;
            }
        };

        hasher.update(&buffer[..bytes]);
        writer.write_all(&buffer[..bytes])?;
        total_written += bytes as u64;
        offset += bytes as u64;
    }

    writer.flush()?;
    let sha256 = format!("{:x}", hasher.finalize());

    write_chain_of_custody(
        &job.output_path,
        job.start,
        total_written,
        &sha256,
        job.file_type,
        false,
        1,
    )?;

    Ok(true)
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn save_file_job_with_buffer(
    job: &ExtractionJob,
    file_data: &mut Vec<u8>,
    seen_hashes: &DashSet<u64>,
    smart_carver: &SmartCarver,
    reader: &DiskReader,
    config: crate::engine::UnsafeConfig,
    debug_file: &mut Option<Option<BufWriter<File>>>,
) -> anyhow::Result<bool> {
    macro_rules! debug_log {
        ($($arg:tt)*) => {
            if let Some(Some(ref mut file)) = debug_file {
                let _ = writeln!(file, $($arg)*);
                let _ = file.flush();
            }
        };
    }

    let mut total_read = 0;
    let mut offset = job.start;

    file_data.clear();
    file_data.reserve(job.size as usize);

    let mut aligned_buffer = AlignedBuffer::new_default(EXTRACTION_BUFFER_SIZE);

    while total_read < job.size as usize {
        let to_read = std::cmp::min(job.size as usize - total_read, EXTRACTION_BUFFER_SIZE);
        let buffer = aligned_buffer.as_mut_slice();

        let bytes_read = match reader.read_into(offset, &mut buffer[..to_read]) {
            Ok(n) if n > 0 => n,
            Ok(_) => break,
            Err(e) => {
                eprintln!(
                    "[Recovery] I/O error reading at offset 0x{:016X}: {} - filling with zeros",
                    offset, e
                );

                for b in &mut buffer[..to_read] {
                    *b = 0;
                }
                file_data.extend_from_slice(&buffer[..to_read]);
                total_read += to_read;
                offset += to_read as u64;
                continue;
            }
        };

        file_data.extend_from_slice(&buffer[..bytes_read]);

        total_read += bytes_read;
        offset += bytes_read as u64;
    }

    let hash = compute_partial_hash(file_data);
    if !seen_hashes.insert(hash) {
        debug_log!(
            "[SKIP] 0x{:016X} {} - Reason: Duplicate (hash collision)",
            job.start,
            job.file_type
        );
        return Ok(false);
    }

    let entropy = sample_entropy(file_data);

    if !config.unsafe_mode && !(MIN_ENTROPY..=MAX_ENTROPY).contains(&entropy) {
        let reason = if entropy < MIN_ENTROPY {
            "EntropyTooLow"
        } else {
            "EntropyTooHigh"
        };
        debug_log!(
            "[SKIP] 0x{:016X} {} - Reason: {} (entropy={:.3}, min={:.1}, max={:.2})",
            job.start,
            job.file_type,
            reason,
            entropy,
            MIN_ENTROPY,
            MAX_ENTROPY
        );
        seen_hashes.remove(&hash);
        return Ok(false);
    }

    if config.debug && file_data.len() >= 4 {
        let header_bytes: Vec<String> = file_data
            .iter()
            .take(16)
            .map(|b| format!("{:02X}", b))
            .collect();
        let has_soi = file_data.len() >= 2 && file_data[0] == 0xFF && file_data[1] == 0xD8;
        debug_log!(
            "[DEBUG] 0x{:016X} {} - size={}, has_SOI={}, first_16_bytes=[{}]",
            job.start,
            job.file_type,
            file_data.len(),
            has_soi,
            header_bytes.join(" ")
        );
    }

    let analysis_result = match job.file_type {
        FileType::Jpeg => smart_carver.analyze_jpeg(file_data, job.start, reader),
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
        CarveDecision::Skip(ref reason) => {
            let should_reject = match reason {
                SkipReason::InvalidStructure => true,
                _ => !config.unsafe_mode,
            };

            if should_reject {
                debug_log!(
                    "[SKIP] 0x{:016X} {} - Reason: {:?}, validation_notes={:?}",
                    job.start,
                    job.file_type,
                    reason,
                    analysis_result.validation_notes
                );
                seen_hashes.remove(&hash);
                return Ok(false);
            }
        }
        CarveDecision::Extract | CarveDecision::ExtractPartial | CarveDecision::AttemptBgc => {}
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

            let mut aligned_frag_buffer = AlignedBuffer::new_default(EXTRACTION_BUFFER_SIZE);

            for fragment in &mf_result.fragments {
                let mut frag_offset = fragment.offset;
                let mut remaining = fragment.size as usize;
                while remaining > 0 {
                    let to_read = remaining.min(EXTRACTION_BUFFER_SIZE);
                    let frag_buffer = aligned_frag_buffer.as_mut_slice();
                    let bytes_read =
                        match reader.read_into(frag_offset, &mut frag_buffer[..to_read]) {
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
                    hasher.update(&frag_buffer[..bytes_read]);
                    writer.write_all(&frag_buffer[..bytes_read])?;
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
                let mut aligned_tail = AlignedBuffer::new_default(tail_size as usize);
                let tail_buffer = aligned_tail.as_mut_slice();
                match reader.read_into(tail_offset, tail_buffer) {
                    Ok(n) if n > 0 => {
                        hasher.update(&tail_buffer[..n]);
                        writer.write_all(&tail_buffer[..n])?;
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

fn should_filter_as_graphic(
    _path: &Path,
    _file_type: FileType,
    _smart_carver: &SmartCarver,
) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_global_matcher_based_matching() {
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

        let mut manager = RecoveryManager::new(
            temp_file.path().to_str().unwrap(),
            temp_dir.path(),
            crate::engine::UnsafeConfig::default(),
        )
        .unwrap();

        manager.set_streaming_mode(true);

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.collected_headers.len(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: 524391,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.collected_headers.len(), 0);

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

        let mut manager = RecoveryManager::new(
            temp_file.path().to_str().unwrap(),
            temp_dir.path(),
            crate::engine::UnsafeConfig::default(),
        )
        .unwrap();

        manager.set_streaming_mode(true);

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.collected_headers.len(), 1);

        manager.process_event(&ScanEvent::HeaderFound {
            offset: h2_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.collected_headers.len(), 2);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f2_offset,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.collected_headers.len(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f1_offset,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.collected_headers.len(), 0);

        manager.wait_for_completion();

        let total_processed = manager.files_recovered() + manager.files_skipped();
        assert_eq!(total_processed, 2, "both files should have been processed");
    }

    #[test]
    fn test_orphan_footer_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager = RecoveryManager::new(
            temp_file.path().to_str().unwrap(),
            temp_dir.path(),
            crate::engine::UnsafeConfig::default(),
        )
        .unwrap();

        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.collected_headers.len(), 0);
        assert_eq!(manager.collected_footers.len(), 1);
        assert_eq!(manager.files_recovered(), 0);
    }

    #[test]
    fn test_mismatched_types_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager = RecoveryManager::new(
            temp_file.path().to_str().unwrap(),
            temp_dir.path(),
            crate::engine::UnsafeConfig::default(),
        )
        .unwrap();

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

        assert_eq!(manager.collected_headers.len(), 1);
        assert_eq!(manager.files_recovered(), 0);
    }
}
