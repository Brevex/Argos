use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::formats::jpeg::{validate_jpeg, validate_jpeg_full, JpegInfo};
use crate::formats::png::{validate_png_full, validate_png_header, PngInfo};
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::types::{
    categorize_dimensions, is_metadata_asset_jpeg, is_metadata_asset_png, DimensionVerdict,
    Fragment, FragmentLists, FragmentRanges, ImageFormat, QuantizationQuality, RecoveredFile,
    RecoveryMethod, LOW_MARKER_COUNT_THRESHOLD, LOW_QUALITY_MAX_DIMENSION, MIN_PHOTO_BYTES,
    MIN_PNG_CHUNK_VARIETY, MIN_PNG_VARIETY_DIMENSION, MIN_SCAN_DATA_ENTROPY,
};

const CONTIGUOUS_SEARCH_LIMIT: u64 = 10 * 1024 * 1024;
const BIFRAGMENT_MAX_GAP: u64 = 50 * 1024 * 1024;
const CLUSTER_SIZES: [u64; 3] = [4096, 8192, 32768];
const MAX_BIFRAGMENT_CANDIDATES: usize = 8;

thread_local! {
    static CARVE_BUFFER: RefCell<AlignedBuffer> = RefCell::new(AlignedBuffer::new());
    static BIFRAG_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

struct FormatSpec {
    max_file_bytes: u64,
    footer_size: u64,
    format: ImageFormat,
}

const JPEG_SPEC: FormatSpec = FormatSpec {
    max_file_bytes: 50 * 1024 * 1024,
    footer_size: 2,
    format: ImageFormat::Jpeg,
};

const PNG_SPEC: FormatSpec = FormatSpec {
    max_file_bytes: 100 * 1024 * 1024,
    footer_size: 12,
    format: ImageFormat::Png,
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

fn jpeg_bifragment_passes(data: &[u8]) -> bool {
    match validate_jpeg_full(data) {
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

fn png_bifragment_passes(data: &[u8]) -> bool {
    match validate_png_full(data) {
        Some(info) => !png_veto(&info),
        None => false,
    }
}

pub fn linear_carve(
    lists: &FragmentLists,
    reader: &DiskReader,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Vec<RecoveredFile> {
    let total = lists.jpeg_headers.len() + lists.png_headers.len();
    let counter = AtomicUsize::new(0);

    let mut recovered = linear_carve_format(
        &lists.jpeg_headers,
        &lists.jpeg_footers,
        &JPEG_SPEC,
        reader,
        jpeg_candidate_passes,
        progress,
        &counter,
        total,
    );

    recovered.extend(linear_carve_format(
        &lists.png_headers,
        &lists.png_footers,
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
    headers: &[&Fragment],
    footers: &[&Fragment],
    spec: &FormatSpec,
    reader: &DiskReader,
    candidate_passes: fn(&[u8]) -> bool,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    counter: &AtomicUsize,
    total: usize,
) -> Vec<RecoveredFile> {
    headers
        .par_iter()
        .filter_map(|header| {
            let result = {
                let footer =
                    find_nearest_footer(header, footers, MIN_PHOTO_BYTES, spec.max_file_bytes)?;
                CARVE_BUFFER.with(|cell| {
                    let mut buf = cell.borrow_mut();
                    if let Some(data) = read_at_offset(reader, header.offset, &mut buf) {
                        if !candidate_passes(data) {
                            return None;
                        }
                    }
                    Some(RecoveredFile::new(
                        FragmentRanges::Linear(header.offset..footer.offset + spec.footer_size),
                        RecoveryMethod::Linear,
                        spec.format,
                        header.entropy,
                    ))
                })
            };
            if let Some(cb) = progress {
                cb(counter.fetch_add(1, Ordering::Relaxed), total);
            }
            result
        })
        .collect()
}

pub fn bifragment_carve(
    lists: &FragmentLists,
    reader: &DiskReader,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Vec<RecoveredFile> {
    let total = lists.jpeg_headers.len() + lists.png_headers.len();
    let counter = AtomicUsize::new(0);

    let mut recovered = bifragment_carve_format(
        &lists.jpeg_headers,
        &lists.jpeg_footers,
        &JPEG_SPEC,
        reader,
        jpeg_bifragment_passes,
        progress,
        &counter,
        total,
    );

    recovered.extend(bifragment_carve_format(
        &lists.png_headers,
        &lists.png_footers,
        &PNG_SPEC,
        reader,
        png_bifragment_passes,
        progress,
        &counter,
        total,
    ));

    recovered
}

#[allow(clippy::too_many_arguments)]
fn bifragment_carve_format(
    headers: &[&Fragment],
    footers: &[&Fragment],
    spec: &FormatSpec,
    reader: &DiskReader,
    validate_and_pass: fn(&[u8]) -> bool,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    counter: &AtomicUsize,
    total: usize,
) -> Vec<RecoveredFile> {
    headers
        .par_iter()
        .filter_map(|header| {
            let result = {
                if has_nearby_footer(footers, header.offset, CONTIGUOUS_SEARCH_LIMIT) {
                    None
                } else {
                    let min_footer_offset = header.offset + MIN_PHOTO_BYTES;
                    let max_footer_offset = header.offset + BIFRAGMENT_MAX_GAP;
                    let start_idx = footers.partition_point(|f| f.offset < min_footer_offset);
                    let end_idx = footers.partition_point(|f| f.offset < max_footer_offset);

                    CARVE_BUFFER.with(|buf_cell| {
                        BIFRAG_BUF.with(|bifrag_cell| {
                            let mut buffer = buf_cell.borrow_mut();
                            let mut bifrag_buf = bifrag_cell.borrow_mut();

                            for footer in &footers[start_idx..end_idx] {
                                if footer.offset <= header.offset {
                                    continue;
                                }
                                if let Some(file) = try_bifragment_points(
                                    header,
                                    footer,
                                    spec,
                                    reader,
                                    &mut buffer,
                                    &mut bifrag_buf,
                                    validate_and_pass,
                                ) {
                                    return Some(file);
                                }
                            }
                            None
                        })
                    })
                }
            };
            if let Some(cb) = progress {
                cb(counter.fetch_add(1, Ordering::Relaxed), total);
            }
            result
        })
        .collect()
}

fn try_bifragment_points(
    header: &Fragment,
    footer: &Fragment,
    spec: &FormatSpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
    bifrag_buf: &mut Vec<u8>,
    validate_and_pass: fn(&[u8]) -> bool,
) -> Option<RecoveredFile> {
    let gap = footer.offset - header.offset;

    for &cluster_size in &CLUSTER_SIZES {
        let mut candidates_tested = 0;
        let mut frag_point = header.offset + cluster_size;

        while frag_point < footer.offset && candidates_tested < MAX_BIFRAGMENT_CANDIDATES {
            let first_frag_size = frag_point - header.offset;
            let second_frag_size = footer.offset + spec.footer_size - frag_point;

            if first_frag_size < MIN_PHOTO_BYTES || second_frag_size < MIN_PHOTO_BYTES {
                frag_point += cluster_size;
                candidates_tested += 1;
                continue;
            }

            let total_size = first_frag_size + second_frag_size;
            if total_size > spec.max_file_bytes {
                break;
            }

            if read_bifragment_candidate(
                reader,
                header.offset,
                first_frag_size,
                frag_point,
                second_frag_size,
                buffer,
                bifrag_buf,
            ) && validate_and_pass(bifrag_buf)
            {
                return Some(RecoveredFile::new(
                    FragmentRanges::Bifragment([
                        header.offset..frag_point,
                        frag_point..footer.offset + spec.footer_size,
                    ]),
                    RecoveryMethod::Bifragment,
                    spec.format,
                    header.entropy,
                ));
            }

            frag_point += cluster_size;
            candidates_tested += 1;
        }

        if gap > cluster_size * MAX_BIFRAGMENT_CANDIDATES as u64 {
            let mid_point = header.offset + (gap / 2);
            let aligned_mid = mid_point & !(cluster_size - 1);
            let first_frag_size = aligned_mid - header.offset;
            let second_frag_size = footer.offset + spec.footer_size - aligned_mid;

            if first_frag_size >= MIN_PHOTO_BYTES && second_frag_size >= MIN_PHOTO_BYTES {
                let total_size = first_frag_size + second_frag_size;
                if total_size <= spec.max_file_bytes
                    && read_bifragment_candidate(
                        reader,
                        header.offset,
                        first_frag_size,
                        aligned_mid,
                        second_frag_size,
                        buffer,
                        bifrag_buf,
                    )
                    && validate_and_pass(bifrag_buf)
                {
                    return Some(RecoveredFile::new(
                        FragmentRanges::Bifragment([
                            header.offset..aligned_mid,
                            aligned_mid..footer.offset + spec.footer_size,
                        ]),
                        RecoveryMethod::Bifragment,
                        spec.format,
                        header.entropy,
                    ));
                }
            }
        }
    }

    None
}

fn read_at_offset<'a>(
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
    footers: &[&'a Fragment],
    min_size: u64,
    max_size: u64,
) -> Option<&'a Fragment> {
    let min_offset = header.offset + min_size;
    let max_offset = header.offset + max_size;

    let start_idx = footers.partition_point(|f| f.offset < min_offset);

    footers[start_idx..]
        .iter()
        .find(|f| f.offset < max_offset)
        .copied()
}

fn has_nearby_footer(footers: &[&Fragment], header_offset: u64, limit: u64) -> bool {
    let start_idx = footers.partition_point(|f| f.offset <= header_offset);

    if let Some(footer) = footers.get(start_idx) {
        footer.offset - header_offset < limit
    } else {
        false
    }
}

fn read_bifragment_candidate(
    reader: &DiskReader,
    first_offset: u64,
    first_size: u64,
    second_offset: u64,
    second_size: u64,
    buffer: &mut AlignedBuffer,
    dest: &mut Vec<u8>,
) -> bool {
    dest.clear();

    if !read_range_into(reader, first_offset, first_size, dest, buffer) {
        return false;
    }

    if !read_range_into(reader, second_offset, second_size, dest, buffer) {
        return false;
    }

    true
}

fn read_range_into(
    reader: &DiskReader,
    start: u64,
    size: u64,
    dest: &mut Vec<u8>,
    buffer: &mut AlignedBuffer,
) -> bool {
    let mut offset = start;
    let end = start + size;

    while offset < end {
        let aligned_offset = offset & ALIGNMENT_MASK;
        let skip = (offset - aligned_offset) as usize;

        let n = match reader.read_at(aligned_offset, buffer) {
            Ok(n) if n > 0 => n,
            _ => return false,
        };

        let available = n.saturating_sub(skip);
        let remaining = (end - offset) as usize;
        let to_copy = available.min(remaining);

        dest.extend_from_slice(&buffer.as_slice()[skip..skip + to_copy]);
        offset += to_copy as u64;
    }

    true
}

#[derive(Debug, Default)]
pub struct RecoveryStats {
    pub jpeg_linear: usize,
    pub jpeg_bifragment: usize,
    pub png_linear: usize,
    pub png_bifragment: usize,
}

impl RecoveryStats {
    pub fn from_recovered(files: &[RecoveredFile]) -> Self {
        let mut stats = Self::default();

        for file in files {
            match (file.format, file.method) {
                (ImageFormat::Jpeg, RecoveryMethod::Linear) => stats.jpeg_linear += 1,
                (ImageFormat::Jpeg, RecoveryMethod::Bifragment) => stats.jpeg_bifragment += 1,
                (ImageFormat::Png, RecoveryMethod::Linear) => stats.png_linear += 1,
                (ImageFormat::Png, RecoveryMethod::Bifragment) => stats.png_bifragment += 1,
            }
        }
        stats
    }

    pub fn total_files(&self) -> usize {
        self.jpeg_linear + self.jpeg_bifragment + self.png_linear + self.png_bifragment
    }
}
