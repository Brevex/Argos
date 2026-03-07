use std::cell::RefCell;
use std::collections::HashMap;

use crate::format::jpeg::{JPEG_EOI, JPEG_SOI};
use crate::format::png::{IEND_CHUNK_TYPE, PNG_SIGNATURE};
use crate::io::{is_recoverable_io_error, zero_sector, AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::core::{
    ConfidenceTier, ExtractionReport, ExtractionResult, ImageFormat, RecoveredFile,
    CORRUPT_SECTOR_RATIO, FINGERPRINT_SIZE, SMALL_BUFFER_SIZE, VALIDATION_HEADER_SIZE,
};
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static EXTRACT_BUFFER: RefCell<AlignedBuffer> = RefCell::new(AlignedBuffer::with_size(SMALL_BUFFER_SIZE));
}

fn compute_fingerprint(
    file: &RecoveredFile,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
) -> Option<[u8; 32]> {
    let ranges = file.fragments.as_slice();
    if ranges.is_empty() {
        return None;
    }

    let mut hasher = blake3::Hasher::new();

    let first_range = &ranges[0];
    let first_len = (first_range.end - first_range.start).min(FINGERPRINT_SIZE as u64);
    let aligned = first_range.start & ALIGNMENT_MASK;
    let skip = (first_range.start - aligned) as usize;
    if let Ok(n) = reader.read_at(aligned, buffer) {
        let available = n.saturating_sub(skip);
        let to_hash = available.min(first_len as usize);
        if to_hash > 0 {
            hasher.update(&buffer.as_slice()[skip..skip + to_hash]);
        }
    }

    let last_range = &ranges[ranges.len() - 1];
    let total_end = last_range.end;
    let tail_start = total_end.saturating_sub(FINGERPRINT_SIZE as u64);
    let tail_start = tail_start.max(last_range.start);
    let aligned_tail = tail_start & ALIGNMENT_MASK;
    let skip_tail = (tail_start - aligned_tail) as usize;
    if let Ok(n) = reader.read_at(aligned_tail, buffer) {
        let available = n.saturating_sub(skip_tail);
        let to_hash = available.min((total_end - tail_start) as usize);
        if to_hash > 0 {
            hasher.update(&buffer.as_slice()[skip_tail..skip_tail + to_hash]);
        }
    }

    let total_size: u64 = ranges.iter().map(|r| r.end - r.start).sum();
    hasher.update(&total_size.to_le_bytes());

    Some(*hasher.finalize().as_bytes())
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

pub fn extract_all(
    files: &[RecoveredFile],
    reader: &DiskReader,
    output_dir: &Path,
    progress_callback: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> io::Result<ExtractionReport> {
    fs::create_dir_all(output_dir)?;
    for tier in &["high", "partial", "low"] {
        fs::create_dir_all(output_dir.join(tier))?;
    }

    let total = files.len();
    let counter = AtomicUsize::new(0);

    let mut indices: Vec<usize> = (0..total).collect();
    indices.sort_unstable_by(|&a, &b| {
        files[b]
            .confidence
            .cmp(&files[a].confidence)
            .then_with(|| files[a].header_offset().cmp(&files[b].header_offset()))
    });

    let fingerprints: Vec<Option<[u8; 32]>> = indices
        .par_iter()
        .map(|&i| {
            EXTRACT_BUFFER.with(|cell| {
                let mut buffer = cell.borrow_mut();
                compute_fingerprint(&files[i], reader, &mut buffer)
            })
        })
        .collect();

    let mut seen: HashMap<[u8; 32], usize> = HashMap::new();
    let mut dedup_mask: Vec<bool> = vec![false; indices.len()];
    for (pos, &i) in indices.iter().enumerate() {
        if let Some(fp) = fingerprints[pos] {
            if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(fp) {
                e.insert(i);
            } else {
                dedup_mask[pos] = true;
            }
        }
    }

    let extract_indices: Vec<usize> = indices
        .iter()
        .zip(dedup_mask.iter())
        .filter_map(|(&i, &is_dup)| if is_dup { None } else { Some(i) })
        .collect();

    let pre_dedup_skipped = total - extract_indices.len();

    let mut sorted_extract: Vec<usize> = extract_indices;
    sorted_extract.sort_unstable_by_key(|&i| files[i].header_offset());

    let mut exact_hashes: HashMap<[u8; 32], (std::path::PathBuf, u8)> = HashMap::new();

    let results: Vec<_> = sorted_extract
        .par_iter()
        .map(|&i| {
            let file = &files[i];
            let tier = ConfidenceTier::from_score(file.confidence);
            let filename = generate_filename(i, file.format);
            let output_path = output_dir.join(tier.dirname()).join(&filename);

            let extraction = EXTRACT_BUFFER.with(|cell| {
                let mut buffer = cell.borrow_mut();
                extract_single(file, reader, &output_path, &mut buffer)
            });

            if let Some(cb) = progress_callback {
                let current = counter.fetch_add(1, Ordering::Relaxed);
                cb(current, total);
            }

            (output_path, file.format, file.confidence, extraction)
        })
        .collect();

    let mut report = ExtractionReport {
        extracted: Vec::with_capacity(files.len()),
        failed: 0,
        corrupt_discarded: 0,
        dedup_skipped: pre_dedup_skipped,
        high_confidence: 0,
        partial_confidence: 0,
        low_confidence: 0,
        tail_check_failed: 0,
        head_validation_failed: 0,
        decode_failed: 0,
    };

    for (output_path, format, confidence, extraction) in results {
        match extraction {
            Ok(result) => {
                if result.bytes_written == 0 {
                    report.tail_check_failed += 1;
                    report.failed += 1;
                    continue;
                }

                if result.total_sectors > 0
                    && result.zero_filled_sectors * CORRUPT_SECTOR_RATIO > result.total_sectors
                {
                    let _ = fs::remove_file(&output_path);
                    report.corrupt_discarded += 1;
                    continue;
                }

                if !validate_in_memory(&result, format) {
                    let _ = fs::remove_file(&output_path);
                    report.head_validation_failed += 1;
                    report.failed += 1;
                    continue;
                }

                let decoded = decode_validate(&output_path, format);
                if !decoded {
                    report.decode_failed += 1;
                }

                match hash_file(&output_path) {
                    Ok(hash) => {
                        match exact_hashes.entry(hash) {
                            std::collections::hash_map::Entry::Vacant(e) => {
                                e.insert((output_path.clone(), confidence));
                            }
                            std::collections::hash_map::Entry::Occupied(mut e) => {
                                let (existing_path, existing_confidence) = e.get();
                                if confidence > *existing_confidence {
                                    let _ = fs::remove_file(existing_path);

                                    report.extracted.retain(|p| p != existing_path);

                                    match ConfidenceTier::from_score(*existing_confidence) {
                                        ConfidenceTier::High => {
                                            report.high_confidence =
                                                report.high_confidence.saturating_sub(1)
                                        }
                                        ConfidenceTier::Partial => {
                                            report.partial_confidence =
                                                report.partial_confidence.saturating_sub(1)
                                        }
                                        ConfidenceTier::Low => {
                                            report.low_confidence =
                                                report.low_confidence.saturating_sub(1)
                                        }
                                    }
                                    e.insert((output_path.clone(), confidence));
                                } else {
                                    let _ = fs::remove_file(&output_path);
                                    report.dedup_skipped += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }

                match ConfidenceTier::from_score(confidence) {
                    ConfidenceTier::High => report.high_confidence += 1,
                    ConfidenceTier::Partial => report.partial_confidence += 1,
                    ConfidenceTier::Low => report.low_confidence += 1,
                }
                report.extracted.push(output_path);
            }
            Err(_) => {
                let _ = fs::remove_file(&output_path);
                report.failed += 1;
                continue;
            }
        }
    }

    sync_directory(output_dir)?;
    for tier in &["high", "partial", "low"] {
        let _ = sync_directory(&output_dir.join(tier));
    }

    Ok(report)
}

fn extract_single(
    file: &RecoveredFile,
    reader: &DiskReader,
    output_path: &Path,
    buffer: &mut AlignedBuffer,
) -> io::Result<ExtractionResult> {
    let ranges = file.fragments.as_slice();
    if ranges.is_empty() {
        return Ok(ExtractionResult {
            zero_filled_sectors: 0,
            total_sectors: 0,
            head: [0u8; VALIDATION_HEADER_SIZE],
            tail: [0u8; VALIDATION_HEADER_SIZE],
            bytes_written: 0,
        });
    }

    let last_range = &ranges[ranges.len() - 1];
    if last_range.end >= VALIDATION_HEADER_SIZE as u64 {
        let tail_start = last_range.end - VALIDATION_HEADER_SIZE as u64;
        let aligned = tail_start & ALIGNMENT_MASK;
        let valid_tail = match reader.read_at(aligned, buffer) {
            Ok(n) => {
                let skip = (tail_start - aligned) as usize;
                if n >= skip + VALIDATION_HEADER_SIZE {
                    let tail = &buffer.as_slice()[skip..skip + VALIDATION_HEADER_SIZE];
                    match file.format {
                        ImageFormat::Jpeg => tail[VALIDATION_HEADER_SIZE - 2..] == JPEG_EOI,
                        ImageFormat::Png => {
                            tail[VALIDATION_HEADER_SIZE - 4..] == *IEND_CHUNK_TYPE
                                || tail.windows(4).any(|w| w == IEND_CHUNK_TYPE)
                        }
                    }
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if !valid_tail {
            return Ok(ExtractionResult {
                zero_filled_sectors: 0,
                total_sectors: 0,
                head: [0u8; VALIDATION_HEADER_SIZE],
                tail: [0u8; VALIDATION_HEADER_SIZE],
                bytes_written: 0,
            });
        }
    }

    let mut out = File::create(output_path)?;
    let mut zero_filled_sectors: usize = 0;
    let mut total_sectors: usize = 0;
    let mut head = [0u8; VALIDATION_HEADER_SIZE];
    let mut tail = [0u8; VALIDATION_HEADER_SIZE];
    let mut bytes_written: usize = 0;

    for range in ranges {
        let mut offset = range.start;

        while offset < range.end {
            let aligned_offset = offset & ALIGNMENT_MASK;
            let skip = (offset - aligned_offset) as usize;
            total_sectors += 1;

            let write_data;
            let to_write;

            match reader.read_at(aligned_offset, buffer) {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }
                    let available = n.saturating_sub(skip);
                    let remaining = (range.end - offset) as usize;
                    to_write = available.min(remaining);
                    write_data = &buffer.as_slice()[skip..skip + to_write];
                }
                Err(e) => {
                    if is_recoverable_io_error(&e) {
                        let remaining = (range.end - offset) as usize;
                        to_write = remaining.min(4096 - skip);
                        write_data = &zero_sector()[..to_write];
                        zero_filled_sectors += 1;
                    } else {
                        return Err(e);
                    }
                }
            }

            if to_write > 0 {
                out.write_all(write_data)?;
                track_head_tail(write_data, bytes_written, &mut head, &mut tail);
                bytes_written += to_write;
            }

            offset += to_write as u64;
        }
    }

    Ok(ExtractionResult {
        zero_filled_sectors,
        total_sectors,
        head,
        tail,
        bytes_written,
    })
}

#[inline]
fn track_head_tail(
    data: &[u8],
    bytes_written: usize,
    head: &mut [u8; VALIDATION_HEADER_SIZE],
    tail: &mut [u8; VALIDATION_HEADER_SIZE],
) {
    let len = data.len();
    if len == 0 {
        return;
    }

    let head_remaining = VALIDATION_HEADER_SIZE.saturating_sub(bytes_written);
    if head_remaining > 0 {
        let n = len.min(head_remaining);
        head[bytes_written..bytes_written + n].copy_from_slice(&data[..n]);
    }

    if bytes_written + len <= VALIDATION_HEADER_SIZE {
        tail[bytes_written..bytes_written + len].copy_from_slice(data);
    } else if len >= VALIDATION_HEADER_SIZE {
        tail.copy_from_slice(&data[len - VALIDATION_HEADER_SIZE..]);
    } else {
        tail.copy_within(len.., 0);
        tail[VALIDATION_HEADER_SIZE - len..].copy_from_slice(data);
    }
}

fn validate_in_memory(result: &ExtractionResult, format: ImageFormat) -> bool {
    if result.bytes_written < VALIDATION_HEADER_SIZE {
        return false;
    }

    let valid_head = match format {
        ImageFormat::Jpeg => result.head[..2] == JPEG_SOI,
        ImageFormat::Png => result.head[..8] == PNG_SIGNATURE,
    };

    if !valid_head {
        return false;
    }

    match format {
        ImageFormat::Jpeg => result.tail[VALIDATION_HEADER_SIZE - 2..] == JPEG_EOI,
        ImageFormat::Png => {
            result.tail[VALIDATION_HEADER_SIZE - 4..] == *IEND_CHUNK_TYPE
                || result.tail.windows(4).any(|w| w == IEND_CHUNK_TYPE)
        }
    }
}

fn decode_validate(path: &Path, format: ImageFormat) -> bool {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let reader = io::BufReader::new(file);
    match format {
        ImageFormat::Jpeg => {
            let mut decoder = jpeg_decoder::Decoder::new(reader);
            decoder.read_info().is_ok()
        }
        ImageFormat::Png => {
            let decoder = png::Decoder::new(reader);
            decoder.read_info().is_ok()
        }
    }
}

fn sync_directory(dir: &Path) -> io::Result<()> {
    let d = File::open(dir)?;
    d.sync_all()
}

pub fn generate_filename(index: usize, format: ImageFormat) -> String {
    format!("recovered_{:06}.{}", index, format.extension())
}
