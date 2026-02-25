use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::carving::read_at_offset;
use crate::formats::jpeg::{
    detect_jpeg_break, find_sos_offset, matches_jpeg_continuation, validate_jpeg, JpegInfo,
};
use crate::formats::png::{
    detect_png_break, matches_png_continuation, validate_png_header, PngInfo, IEND_CRC,
};
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::types::{
    categorize_dimensions, is_metadata_asset_jpeg, is_metadata_asset_png, BreakConfidence,
    BreakPoint, ContinuationSignature, DimensionVerdict, Fragment, FragmentMap, FragmentRanges,
    ImageFormat, Offset, QuantizationQuality, RecoveredFile, RecoveryMethod,
    BREAK_DETECTION_READ_SIZE, CONTINUATION_MATCH_WINDOW, CONTINUATION_SCAN_CLUSTER_SIZE,
    LOW_MARKER_COUNT_THRESHOLD, LOW_QUALITY_MAX_DIMENSION, MAX_CONTINUATION_CANDIDATES,
    MIN_FRAGMENT_SIZE, MIN_PHOTO_BYTES, MIN_PNG_CHUNK_VARIETY, MIN_PNG_VARIETY_DIMENSION,
    MIN_SCAN_DATA_ENTROPY, REASSEMBLY_MAX_GAP, SMALL_BUFFER_SIZE,
};

thread_local! {
    static REASM_BUFFER: RefCell<AlignedBuffer> = RefCell::new(AlignedBuffer::with_size(SMALL_BUFFER_SIZE));
    static DETECT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

struct ReassemblySpec {
    max_file_bytes: u64,
    footer_size: u64,
    format: ImageFormat,
}

const JPEG_REASM_SPEC: ReassemblySpec = ReassemblySpec {
    max_file_bytes: 50 * 1024 * 1024,
    footer_size: 2,
    format: ImageFormat::Jpeg,
};

const PNG_REASM_SPEC: ReassemblySpec = ReassemblySpec {
    max_file_bytes: 100 * 1024 * 1024,
    footer_size: 12,
    format: ImageFormat::Png,
};

pub fn reassemble(
    map: &FragmentMap,
    reader: &DiskReader,
    recovered_offsets: &HashSet<u64>,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Vec<RecoveredFile> {
    let total = map.jpeg_headers().len() + map.png_headers().len();
    let counter = AtomicUsize::new(0);

    let mut recovered = reassemble_format(
        map.jpeg_headers(),
        map.jpeg_footers(),
        &JPEG_REASM_SPEC,
        reader,
        recovered_offsets,
        detect_jpeg_break_at,
        matches_jpeg_continuation,
        jpeg_header_passes,
        progress,
        &counter,
        total,
    );

    recovered.extend(reassemble_format(
        map.png_headers(),
        map.png_footers(),
        &PNG_REASM_SPEC,
        reader,
        recovered_offsets,
        detect_png_break_at,
        matches_png_continuation,
        png_header_passes,
        progress,
        &counter,
        total,
    ));

    recovered
}

fn detect_jpeg_break_at(data: &[u8]) -> Option<BreakPoint> {
    let sos_offset = find_sos_offset(data)?;
    let relative = detect_jpeg_break(data, sos_offset)?;
    Some(BreakPoint {
        break_offset: relative as Offset,
        confidence: if data
            .get(relative..relative + 512)
            .is_some_and(|s| s.iter().all(|&b| b == 0))
        {
            BreakConfidence::Definite
        } else {
            BreakConfidence::Probable
        },
        signature: ContinuationSignature::JpegScanData,
    })
}

fn detect_png_break_at(data: &[u8]) -> Option<BreakPoint> {
    let relative = detect_png_break(data)?;
    Some(BreakPoint {
        break_offset: relative as Offset,
        confidence: if data
            .get(relative..relative + 512)
            .is_some_and(|s| s.iter().all(|&b| b == 0))
        {
            BreakConfidence::Definite
        } else {
            BreakConfidence::Probable
        },
        signature: ContinuationSignature::PngIdat,
    })
}

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

fn jpeg_header_passes(data: &[u8]) -> bool {
    match validate_jpeg(data) {
        Some(info) => !jpeg_veto(&info),
        None => false,
    }
}

fn png_header_passes(data: &[u8]) -> bool {
    match validate_png_header(data) {
        Some(info) => !png_veto(&info),
        None => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn reassemble_format(
    headers: &[Fragment],
    footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    recovered_offsets: &HashSet<u64>,
    detect_break: fn(&[u8]) -> Option<BreakPoint>,
    matches_continuation: fn(&[u8]) -> bool,
    header_passes: fn(&[u8]) -> bool,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    counter: &AtomicUsize,
    total: usize,
) -> Vec<RecoveredFile> {
    headers
        .par_iter()
        .filter_map(|header| {
            let result = if recovered_offsets.contains(&header.offset) {
                None
            } else {
                REASM_BUFFER.with(|buf_cell| {
                    DETECT_BUF.with(|det_cell| {
                        let mut buffer = buf_cell.borrow_mut();
                        let mut detect_buf = det_cell.borrow_mut();
                        try_reassemble(
                            header,
                            footers,
                            spec,
                            reader,
                            &mut buffer,
                            &mut detect_buf,
                            detect_break,
                            matches_continuation,
                            header_passes,
                        )
                    })
                })
            };
            if let Some(cb) = progress {
                cb(counter.fetch_add(1, Ordering::Relaxed), total);
            }
            result
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn try_reassemble(
    header: &Fragment,
    footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
    detect_buf: &mut Vec<u8>,
    detect_break: fn(&[u8]) -> Option<BreakPoint>,
    matches_continuation: fn(&[u8]) -> bool,
    header_passes: fn(&[u8]) -> bool,
) -> Option<RecoveredFile> {
    detect_buf.clear();

    read_range(
        reader,
        header.offset,
        BREAK_DETECTION_READ_SIZE as u64,
        detect_buf,
        buffer,
    )?;

    if !header_passes(detect_buf) {
        return None;
    }

    let bp = detect_break(detect_buf)?;

    if bp.break_offset < MIN_FRAGMENT_SIZE {
        return None;
    }

    let absolute_break = header.offset + bp.break_offset;
    let first_fragment = header.offset..absolute_break;

    let search_end = (absolute_break + REASSEMBLY_MAX_GAP).min(reader.size());

    let candidates = scan_for_continuations(
        reader,
        absolute_break,
        search_end,
        buffer,
        matches_continuation,
    );

    if candidates.is_empty() {
        return None;
    }

    for candidate_offset in &candidates {
        let result = try_chain_with_footer(
            header,
            &first_fragment,
            *candidate_offset,
            footers,
            spec,
            reader,
            buffer,
        );
        if result.is_some() {
            return result;
        }
    }

    None
}

fn scan_for_continuations(
    reader: &DiskReader,
    search_start: u64,
    search_end: u64,
    buffer: &mut AlignedBuffer,
    matches_continuation: fn(&[u8]) -> bool,
) -> Vec<u64> {
    let mut candidates = Vec::new();
    let cluster = CONTINUATION_SCAN_CLUSTER_SIZE;
    let aligned_start = (search_start + cluster - 1) & !(cluster - 1);
    let mut offset = aligned_start;

    let prefetch_chunk = 64 * 1024 * 1024u64;
    let mut prefetch_end = 0u64;

    while offset < search_end && candidates.len() < MAX_CONTINUATION_CANDIDATES {
        if offset >= prefetch_end {
            let chunk = prefetch_chunk.min(search_end - offset);
            reader.advise_willneed(offset, chunk);
            prefetch_end = offset + chunk;
        }

        if let Some(data) = read_at_offset(reader, offset, buffer) {
            let window = &data[..data.len().min(CONTINUATION_MATCH_WINDOW)];
            if matches_continuation(window) {
                candidates.push(offset);
            }
        }

        offset += cluster;
    }

    candidates
}

fn try_chain_with_footer(
    header: &Fragment,
    first_fragment: &Range<u64>,
    continuation_offset: u64,
    footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
) -> Option<RecoveredFile> {
    let first_size = first_fragment.end - first_fragment.start;
    let remaining_budget = spec.max_file_bytes.saturating_sub(first_size);

    let min_footer_offset = continuation_offset + MIN_FRAGMENT_SIZE;
    let max_footer_offset = continuation_offset + remaining_budget;

    let start_idx = footers.partition_point(|f| f.offset < min_footer_offset);

    for footer in &footers[start_idx..] {
        if footer.offset >= max_footer_offset {
            break;
        }

        let second_end = footer.offset + spec.footer_size;
        let second_fragment = continuation_offset..second_end;
        let total_size = first_size + (second_end - continuation_offset);

        if total_size > spec.max_file_bytes || total_size < MIN_PHOTO_BYTES {
            continue;
        }

        if !verify_tail(reader, second_end, spec, buffer) {
            continue;
        }

        let fragments = vec![first_fragment.clone(), second_fragment];
        let depth = fragments.len() as u8;
        return Some(RecoveredFile::new(
            FragmentRanges::Multi(fragments),
            RecoveryMethod::Reassembled { depth },
            spec.format,
            header.entropy,
        ));
    }

    None
}

fn verify_tail(
    reader: &DiskReader,
    fragment_end: u64,
    spec: &ReassemblySpec,
    buffer: &mut AlignedBuffer,
) -> bool {
    match spec.format {
        ImageFormat::Jpeg => {
            if fragment_end < 2 {
                return false;
            }
            match read_at_offset(reader, fragment_end - 2, buffer) {
                Some(data) if data.len() >= 2 => data[0] == 0xFF && data[1] == 0xD9,
                _ => false,
            }
        }
        ImageFormat::Png => {
            if fragment_end < 12 {
                return false;
            }
            match read_at_offset(reader, fragment_end - 12, buffer) {
                Some(data) if data.len() >= 12 => {
                    data[0..4] == [0, 0, 0, 0]
                        && data[4..8] == *b"IEND"
                        && u32::from_be_bytes([data[8], data[9], data[10], data[11]]) == IEND_CRC
                }
                _ => false,
            }
        }
    }
}

fn read_range(
    reader: &DiskReader,
    start: u64,
    size: u64,
    dest: &mut Vec<u8>,
    buffer: &mut AlignedBuffer,
) -> Option<()> {
    let mut offset = start;
    let end = start + size;

    while offset < end {
        let aligned_offset = offset & ALIGNMENT_MASK;
        let skip = (offset - aligned_offset) as usize;

        let n = reader.read_at(aligned_offset, buffer).ok()?;
        if n == 0 {
            return None;
        }

        let available = n.saturating_sub(skip);
        if available == 0 {
            return None;
        }
        let remaining = (end - offset) as usize;
        let to_copy = available.min(remaining);

        dest.extend_from_slice(&buffer.as_slice()[skip..skip + to_copy]);
        offset += to_copy as u64;
    }

    Some(())
}
