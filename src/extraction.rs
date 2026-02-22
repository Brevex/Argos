use std::cell::RefCell;

use crate::formats::jpeg::{JPEG_EOI, JPEG_SOI};
use crate::formats::png::{IEND_CHUNK_TYPE, PNG_SIGNATURE};
use crate::io::{is_recoverable_io_error, zero_sector, AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::types::{
    ExtractionReport, ExtractionResult, ImageFormat, RecoveredFile, CORRUPT_SECTOR_RATIO,
    SMALL_BUFFER_SIZE, VALIDATION_HEADER_SIZE,
};
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static EXTRACT_BUFFER: RefCell<AlignedBuffer> = RefCell::new(AlignedBuffer::with_size(SMALL_BUFFER_SIZE));
}

pub fn extract_all(
    files: &[RecoveredFile],
    reader: &DiskReader,
    output_dir: &Path,
    progress_callback: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> io::Result<ExtractionReport> {
    fs::create_dir_all(output_dir)?;

    let total = files.len();
    let counter = AtomicUsize::new(0);

    let mut indices: Vec<usize> = (0..total).collect();
    indices.sort_unstable_by_key(|&i| files[i].header_offset());

    let results: Vec<_> = indices
        .par_iter()
        .map(|&i| {
            let file = &files[i];
            let filename = generate_filename(i, file.format);
            let output_path = output_dir.join(&filename);

            let extraction = EXTRACT_BUFFER.with(|cell| {
                let mut buffer = cell.borrow_mut();
                extract_single(file, reader, &output_path, &mut buffer)
            });

            if let Some(cb) = progress_callback {
                let current = counter.fetch_add(1, Ordering::Relaxed);
                cb(current, total);
            }

            (output_path, file.format, extraction)
        })
        .collect();

    let mut report = ExtractionReport {
        extracted: Vec::with_capacity(files.len()),
        failed: 0,
        corrupt_discarded: 0,
    };

    for (output_path, format, extraction) in results {
        match extraction {
            Ok(result) => {
                if result.total_sectors > 0
                    && result.zero_filled_sectors * CORRUPT_SECTOR_RATIO > result.total_sectors
                {
                    let _ = fs::remove_file(&output_path);
                    report.corrupt_discarded += 1;
                    continue;
                }

                if !validate_in_memory(&result, format) {
                    let _ = fs::remove_file(&output_path);
                    report.failed += 1;
                    continue;
                }

                report.extracted.push(output_path);
            }
            Err(_) => {
                let _ = fs::remove_file(&output_path);
                report.failed += 1;
            }
        }
    }

    sync_directory(output_dir)?;

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

fn sync_directory(dir: &Path) -> io::Result<()> {
    let d = File::open(dir)?;
    d.sync_all()
}

pub fn generate_filename(index: usize, format: ImageFormat) -> String {
    format!("recovered_{:06}.{}", index, format.extension())
}
