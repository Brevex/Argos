use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::core::{
    Fragment, FragmentMap, FragmentRanges, ImageFormat, RecoveredFile, RecoveryMethod,
    MIN_PHOTO_BYTES, SMALL_BUFFER_SIZE,
};
use crate::format::jpeg::candidate_score as jpeg_candidate_score;
use crate::format::png::candidate_score as png_candidate_score;
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::scan::is_valid_scan_context;

const FADVISE_CHUNK: u64 = 256 * 1024 * 1024;
const MAX_FOOTER_CANDIDATES: usize = 32;
const FOOTER_CONTEXT_WINDOW: usize = 256;

thread_local! {
    static CARVE_BUFFER: RefCell<AlignedBuffer> = RefCell::new(AlignedBuffer::with_size(SMALL_BUFFER_SIZE));
}

struct FormatSpec {
    max_file_bytes: u64,
    footer_size: u64,
    format: ImageFormat,
    validate_footer_context: bool,
}

const JPEG_SPEC: FormatSpec = FormatSpec {
    max_file_bytes: 50 * 1024 * 1024,
    footer_size: 2,
    format: ImageFormat::Jpeg,
    validate_footer_context: true,
};

const PNG_SPEC: FormatSpec = FormatSpec {
    max_file_bytes: 100 * 1024 * 1024,
    footer_size: 12,
    format: ImageFormat::Png,
    validate_footer_context: false,
};

pub fn linear_carve(
    map: &FragmentMap,
    reader: &DiskReader,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Vec<RecoveredFile> {
    let total = map.jpeg_headers().len() + map.png_headers().len();
    let counter = AtomicUsize::new(0);

    let mut recovered = linear_carve_format(
        map.jpeg_headers(),
        map.jpeg_footers(),
        &JPEG_SPEC,
        reader,
        jpeg_candidate_score,
        progress,
        &counter,
        total,
    );

    recovered.extend(linear_carve_format(
        map.png_headers(),
        map.png_footers(),
        &PNG_SPEC,
        reader,
        png_candidate_score,
        progress,
        &counter,
        total,
    ));

    recovered
}

#[allow(clippy::too_many_arguments)]
fn linear_carve_format(
    headers: &[Fragment],
    footers: &[Fragment],
    spec: &FormatSpec,
    reader: &DiskReader,
    candidate_score: fn(&[u8]) -> Option<u8>,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    counter: &AtomicUsize,
    total: usize,
) -> Vec<RecoveredFile> {
    if !headers.is_empty() {
        let first = headers[0].offset;
        let last = headers[headers.len() - 1].offset;
        let mut adv = first;
        while adv <= last {
            let chunk = FADVISE_CHUNK.min(last - adv + SMALL_BUFFER_SIZE as u64);
            reader.advise_willneed(adv, chunk);
            adv += FADVISE_CHUNK;
        }
    }

    headers
        .par_iter()
        .filter_map(|header| {
            let result = CARVE_BUFFER.with(|cell| {
                let mut buf = cell.borrow_mut();
                let confidence = match read_at_offset(reader, header.offset, &mut buf) {
                    Some(data) => candidate_score(data)?,
                    None => return None,
                };

                let footer = if spec.validate_footer_context {
                    find_best_footer(
                        header,
                        footers,
                        MIN_PHOTO_BYTES,
                        spec.max_file_bytes,
                        reader,
                        &mut buf,
                    )?
                } else {
                    find_nearest_footer(header, footers, MIN_PHOTO_BYTES, spec.max_file_bytes)?
                };

                Some(RecoveredFile::new(
                    FragmentRanges::Linear(header.offset..footer.offset + spec.footer_size),
                    RecoveryMethod::Linear,
                    spec.format,
                    header.entropy,
                    confidence,
                ))
            });
            if let Some(cb) = progress {
                cb(counter.fetch_add(1, Ordering::Relaxed), total);
            }
            result
        })
        .collect()
}

pub fn read_at_offset<'a>(
    reader: &DiskReader,
    offset: u64,
    buffer: &'a mut AlignedBuffer,
) -> Option<&'a [u8]> {
    let aligned = offset & ALIGNMENT_MASK;
    let skip = (offset - aligned) as usize;
    let n = reader.read_at(aligned, buffer).ok()?;
    if n <= skip {
        return None;
    }
    Some(&buffer.as_slice()[skip..n])
}

fn find_nearest_footer<'a>(
    header: &Fragment,
    footers: &'a [Fragment],
    min_size: u64,
    max_size: u64,
) -> Option<&'a Fragment> {
    let min_offset = header.offset + min_size;
    let max_offset = header.offset + max_size;

    let start_idx = footers.partition_point(|f| f.offset < min_offset);

    footers[start_idx..].iter().find(|f| f.offset < max_offset)
}

fn find_best_footer<'a>(
    header: &Fragment,
    footers: &'a [Fragment],
    min_size: u64,
    max_size: u64,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
) -> Option<&'a Fragment> {
    let min_offset = header.offset + min_size;
    let max_offset = header.offset + max_size;

    let start_idx = footers.partition_point(|f| f.offset < min_offset);

    for (i, footer) in footers[start_idx..].iter().enumerate() {
        if footer.offset >= max_offset {
            break;
        }
        if i >= MAX_FOOTER_CANDIDATES {
            break;
        }

        if footer.offset < FOOTER_CONTEXT_WINDOW as u64 {
            continue;
        }

        let context_start = footer.offset - FOOTER_CONTEXT_WINDOW as u64;
        if let Some(data) = read_at_offset(reader, context_start, buffer) {
            let context_len = data.len().min(FOOTER_CONTEXT_WINDOW);
            if is_valid_scan_context(&data[..context_len]) {
                return Some(footer);
            }
        }
    }

    None
}

#[derive(Debug, Default)]
pub struct RecoveryStats {
    pub jpeg_linear: usize,
    pub jpeg_reassembled: usize,
    pub png_linear: usize,
    pub png_reassembled: usize,
}

impl RecoveryStats {
    pub fn from_recovered(files: &[RecoveredFile]) -> Self {
        let mut stats = Self::default();

        for file in files {
            match (file.format, file.method) {
                (ImageFormat::Jpeg, RecoveryMethod::Linear) => stats.jpeg_linear += 1,
                (ImageFormat::Jpeg, RecoveryMethod::Reassembled { .. }) => {
                    stats.jpeg_reassembled += 1
                }
                (ImageFormat::Png, RecoveryMethod::Linear) => stats.png_linear += 1,
                (ImageFormat::Png, RecoveryMethod::Reassembled { .. }) => {
                    stats.png_reassembled += 1
                }
            }
        }
        stats
    }

    pub fn total_files(&self) -> usize {
        self.jpeg_linear + self.jpeg_reassembled + self.png_linear + self.png_reassembled
    }
}
