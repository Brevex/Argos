use super::signatures::{FileType, SignatureMatch, SignatureRegistry};
use crate::core::io::BlockDeviceReader;
use byteorder::{LittleEndian, ReadBytesExt};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub total_bytes: u64,
    pub scanned_bytes: u64,
    pub matches_found: usize,
    pub estimated_remaining: Option<Duration>,
    pub speed_bps: u64,
}

impl ScanProgress {
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            scanned_bytes: 0,
            matches_found: 0,
            estimated_remaining: None,
            speed_bps: 0,
        }
    }

    pub fn update(&mut self, scanned_bytes: u64, matches_found: usize, speed_bps: u64) {
        self.scanned_bytes = scanned_bytes;
        self.matches_found = matches_found;
        self.speed_bps = speed_bps;
        if speed_bps > 0 {
            let remaining_bytes = self.total_bytes.saturating_sub(scanned_bytes);
            self.estimated_remaining = Some(Duration::from_secs(remaining_bytes / speed_bps));
        }
    }
}

pub type ProgressCallback = Box<dyn Fn(&ScanProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub source_path: String,
    pub total_bytes: u64,
    pub duration: Duration,
    pub matches: Vec<SignatureMatch>,
    pub type_counts: HashMap<FileType, usize>,
    pub errors: Vec<String>,
}

impl ScanResult {
    pub fn new(source_path: String, total_bytes: u64, duration: Duration) -> Self {
        Self {
            source_path,
            total_bytes,
            duration,
            matches: Vec::new(),
            type_counts: HashMap::new(),
            errors: Vec::new(),
        }
    }

    pub fn add_match(&mut self, match_: SignatureMatch) {
        *self.type_counts.entry(match_.file_type()).or_insert(0) += 1;
        self.matches.push(match_);
    }

    pub fn total_matches(&self) -> usize {
        self.matches.len()
    }

    pub fn summary(&self) -> String {
        let mut summary = format!(
            "Scanned {} ({} bytes) in {:.2}s\n",
            self.source_path,
            self.total_bytes,
            self.duration.as_secs_f64()
        );
        summary.push_str(&format!(
            "Found {} potential files:\n",
            self.total_matches()
        ));
        for (file_type, count) in &self.type_counts {
            summary.push_str(&format!("  - {}: {}\n", file_type, count));
        }
        summary
    }
}

pub struct RecoveredFile {
    pub id: u64,
    pub file_type: FileType,
    pub offset: u64,
    pub data: Vec<u8>,
    pub confidence: f32,
    pub is_corrupted: bool,
}

impl RecoveredFile {
    pub fn new(id: u64, file_type: FileType, offset: u64, data: Vec<u8>, confidence: f32) -> Self {
        Self {
            id,
            file_type,
            offset,
            data,
            confidence,
            is_corrupted: false,
        }
    }
}

pub struct Carver {
    registry: Arc<SignatureRegistry>,
}

impl Carver {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(SignatureRegistry::default_images()),
        }
    }

    pub fn scan<R: BlockDeviceReader>(
        &self,
        device: &R,
        options: &ScanOptions,
        progress_callback: Option<ProgressCallback>,
    ) -> anyhow::Result<ScanResult> {
        let start = Instant::now();
        let device_size = device.size();
        let chunk_size = options.chunk_size;

        let scanned_bytes = Arc::new(AtomicU64::new(0));
        let matches_found = Arc::new(AtomicUsize::new(0));

        let progress_tracker = if let Some(cb) = &progress_callback {
            Some((cb, Arc::clone(&scanned_bytes), Arc::clone(&matches_found)))
        } else {
            None
        };

        // Calculate number of chunks
        let num_chunks = (device_size + chunk_size as u64 - 1) / chunk_size as u64;

        let all_matches: Vec<SignatureMatch> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                let offset = chunk_idx * chunk_size as u64;
                let actual_chunk_size =
                    ((offset + chunk_size as u64).min(device_size) - offset) as usize;

                // Stateless read
                match device.read_at(offset, actual_chunk_size) {
                    Ok(data) => {
                        let matches = self.scan_chunk(&data, offset);

                        // Update progress
                        let current_scanned = scanned_bytes
                            .fetch_add(actual_chunk_size as u64, Ordering::Relaxed)
                            + actual_chunk_size as u64;
                        let current_matches = matches_found
                            .fetch_add(matches.len(), Ordering::Relaxed)
                            + matches.len();

                        // Report progress occasionally or if needed.
                        if let Some((cb, _, _)) = &progress_tracker {
                            let p = ScanProgress {
                                total_bytes: device_size,
                                scanned_bytes: current_scanned,
                                matches_found: current_matches,
                                estimated_remaining: None,
                                speed_bps: 0,
                            };
                            cb(&p);
                        }

                        matches
                    }
                    Err(_) => Vec::new(),
                }
            })
            .flat_map(|matches| matches)
            .filter(|m| {
                options.file_types.is_empty() || options.file_types.contains(&m.file_type())
            })
            .collect();

        let mut result = ScanResult::new(device.path().to_string(), device_size, start.elapsed());
        for m in all_matches {
            result.add_match(m);
        }

        Ok(result)
    }

    pub fn scan_parallel(
        &self,
        data: &[u8],
        device_path: &str,
        options: &ScanOptions,
        progress_callback: Option<ProgressCallback>,
    ) -> anyhow::Result<ScanResult> {
        let start_time = Instant::now();
        let data_size = data.len() as u64;
        let chunk_size = options.chunk_size;

        let bytes_scanned = AtomicU64::new(0);
        let matches_found = AtomicUsize::new(0);

        let chunks: Vec<(u64, &[u8])> = data
            .chunks(chunk_size)
            .enumerate()
            .map(|(i, chunk)| ((i * chunk_size) as u64, chunk))
            .collect();

        let all_matches: Vec<SignatureMatch> = chunks
            .par_iter()
            .flat_map(|(offset, chunk)| {
                let matches = self.scan_chunk(chunk, *offset);
                bytes_scanned.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                matches_found.fetch_add(matches.len(), Ordering::Relaxed);
                matches
            })
            .filter(|m| {
                options.file_types.is_empty() || options.file_types.contains(&m.file_type())
            })
            .collect();

        if let Some(callback) = progress_callback {
            let mut p = ScanProgress::new(data_size);
            p.update(
                data_size,
                all_matches.len(),
                data_size / start_time.elapsed().as_secs().max(1),
            );
            callback(&p);
        }

        let mut result = ScanResult::new(device_path.to_string(), data_size, start_time.elapsed());
        for m in all_matches {
            result.add_match(m);
        }
        Ok(result)
    }

    fn scan_chunk(&self, data: &[u8], base_offset: u64) -> Vec<SignatureMatch> {
        let mut matches = Vec::new();
        for (offset, sig) in self.registry.find_all_matches_with_offsets(data) {
            let start_offset = base_offset + offset as u64;
            let remaining = &data[offset..];
            let end_offset = sig
                .find_footer(remaining)
                .map(|pos| start_offset + pos as u64);
            let estimated_size = end_offset
                .map(|e| e - start_offset)
                .unwrap_or(sig.max_size());

            if estimated_size >= 100 {
                matches.push(SignatureMatch::new(
                    sig.file_type(),
                    start_offset,
                    end_offset,
                    estimated_size,
                ));
            }
        }
        matches
    }

    pub fn recover_file(
        &self,
        data: &[u8],
        match_info: &SignatureMatch,
        file_id: u64,
    ) -> anyhow::Result<RecoveredFile> {
        let file_type = match_info.file_type();
        let size = self
            .determine_file_size(data, file_type)
            .map(|s| s as usize)
            .or_else(|| {
                match_info
                    .end_offset()
                    .map(|e| (e - match_info.start_offset()) as usize)
            })
            .unwrap_or_else(|| match_info.estimated_size() as usize);

        if size == 0 || size > data.len() {
            return Err(anyhow::anyhow!("Invalid file size"));
        }

        let file_data = data[..size].to_vec();
        let valid = self.validate(&file_data, file_type);
        let mut recovered = RecoveredFile::new(
            file_id,
            file_type,
            match_info.start_offset(),
            file_data,
            if valid { 0.9 } else { 0.5 },
        );
        if !valid {
            recovered.is_corrupted = true;
        }

        Ok(recovered)
    }

    fn determine_file_size(&self, data: &[u8], file_type: FileType) -> Option<u64> {
        match file_type {
            FileType::Jpeg => self.find_jpeg_end(data).map(|s| s as u64),
            FileType::Png => self.find_png_end(data).map(|s| s as u64),
            FileType::Gif => self.find_gif_end(data).map(|s| s as u64),
            FileType::Bmp => {
                if data.len() >= 6 {
                    let mut rdr = Cursor::new(&data[2..6]);
                    rdr.read_u32::<LittleEndian>().ok().map(|s| s as u64)
                } else {
                    None
                }
            }
            FileType::WebP => {
                if data.len() >= 8 && &data[8..12] == b"WEBP" {
                    let mut rdr = Cursor::new(&data[4..8]);
                    rdr.read_u32::<LittleEndian>().ok().map(|s| (s + 8) as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn validate(&self, data: &[u8], _file_type: FileType) -> bool {
        !data.is_empty()
    }

    fn find_jpeg_end(&self, data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(1) {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                return Some(i + 2);
            }
        }
        None
    }

    fn find_png_end(&self, data: &[u8]) -> Option<usize> {
        let iend = [0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];
        if data.len() < iend.len() {
            return None;
        }
        for i in 0..=data.len() - iend.len() {
            if &data[i..i + iend.len()] == iend {
                return Some(i + iend.len());
            }
        }
        None
    }

    fn find_gif_end(&self, data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(1) {
            if data[i] == 0x00 && data[i + 1] == 0x3B {
                return Some(i + 2);
            }
        }
        None
    }
}

pub struct ScanOptions {
    pub chunk_size: usize,
    pub file_types: Vec<FileType>,
}
