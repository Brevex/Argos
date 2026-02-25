use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::analysis::is_valid_scan_context;
use crate::formats::jpeg::{validate_jpeg, JpegInfo};
use crate::formats::png::{validate_png_header, PngInfo};
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::types::{
    categorize_dimensions, is_metadata_asset_jpeg, is_metadata_asset_png, DimensionVerdict,
    Fragment, FragmentMap, FragmentRanges, ImageFormat, QuantizationQuality, RecoveredFile,
    RecoveryMethod, LOW_MARKER_COUNT_THRESHOLD, LOW_QUALITY_MAX_DIMENSION, MIN_PHOTO_BYTES,
    MIN_PNG_CHUNK_VARIETY, MIN_PNG_VARIETY_DIMENSION, MIN_SCAN_DATA_ENTROPY, SMALL_BUFFER_SIZE,
};

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

fn jpeg_veto(info: &JpegInfo) -> bool {
    match categorize_dimensions(info.width as u32, info.height as u32) {
        DimensionVerdict::TooSmall | DimensionVerdict::Asset => return true,
        DimensionVerdict::Photo => {}
    }
    if is_metadata_asset_jpeg(info.width as u32, info.height as u32, &info.metadata) {
        return true;
    }
    if info.metadata.quantization_quality == QuantizationQuality::Low
        && !info.metadata.has_exif
        && !info.metadata.has_icc_profile
        && info.width as u32 <= LOW_QUALITY_MAX_DIMENSION
        && info.height as u32 <= LOW_QUALITY_MAX_DIMENSION
    {
        return true;
    }
    if info.metadata.marker_count < LOW_MARKER_COUNT_THRESHOLD && !info.metadata.has_exif {
        return true;
    }
    info.metadata.has_sos
        && info.metadata.scan_data_entropy > 0.0
        && info.metadata.scan_data_entropy < MIN_SCAN_DATA_ENTROPY
}

fn png_veto(info: &PngInfo) -> bool {
    if info.idat_count == 0 {
        return true;
    }
    match categorize_dimensions(info.width, info.height) {
        DimensionVerdict::TooSmall | DimensionVerdict::Asset => return true,
        DimensionVerdict::Photo => {}
    }
    if is_metadata_asset_png(info.width, info.height, &info.metadata) {
        return true;
    }
    if info.metadata.chunk_variety < MIN_PNG_CHUNK_VARIETY
        && info.width <= MIN_PNG_VARIETY_DIMENSION
        && info.height <= MIN_PNG_VARIETY_DIMENSION
    {
        return true;
    }
    false
}

fn jpeg_candidate_passes(data: &[u8]) -> bool {
    match validate_jpeg(data) {
        Some(info) => !jpeg_veto(&info),
        None => false,
    }
}

fn png_candidate_passes(data: &[u8]) -> bool {
    match validate_png_header(data) {
        Some(info) => !png_veto(&info),
        None => false,
    }
}

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
        jpeg_candidate_passes,
        progress,
        &counter,
        total,
    );

    recovered.extend(linear_carve_format(
        map.png_headers(),
        map.png_footers(),
        &PNG_SPEC,
        reader,
        png_candidate_passes,
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
    candidate_passes: fn(&[u8]) -> bool,
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
                if let Some(data) = read_at_offset(reader, header.offset, &mut buf) {
                    if !candidate_passes(data) {
                        return None;
                    }
                }

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
