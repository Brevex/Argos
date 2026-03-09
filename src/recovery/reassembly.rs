use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::core::{
    BreakConfidence, BreakPoint, ContinuationSignature, Fragment, FragmentMap, FragmentRanges,
    ImageFormat, Offset, RecoveredFile, RecoveryMethod, BREAK_DETECTION_READ_SIZE,
    CONTINUATION_MATCH_WINDOW, CONTINUATION_SCAN_CLUSTER_SIZE, MAX_CHAIN_DEPTH,
    MAX_CONTINUATION_CANDIDATES, MIN_FRAGMENT_SIZE, MIN_PHOTO_BYTES, REASSEMBLY_MAX_GAP,
    SMALL_BUFFER_SIZE,
};
use crate::format::jpeg::{
    candidate_score as jpeg_candidate_score, detect_jpeg_break, find_sos_offset,
    matches_jpeg_continuation,
};
use crate::format::png::{
    candidate_score as png_candidate_score, detect_png_break, matches_png_continuation, IEND_CRC,
};
use crate::fs::FsHintMap;
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};
use crate::recovery::carving::read_at_offset;

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
    hints: Option<&FsHintMap>,
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
        jpeg_candidate_score,
        hints,
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
        png_candidate_score,
        hints,
        progress,
        &counter,
        total,
    ));

    recovered
}

const ZERO_FILL_CHECK_SIZE: usize = 512;

fn classify_break_confidence(data: &[u8], offset: usize) -> BreakConfidence {
    if data
        .get(offset..offset + ZERO_FILL_CHECK_SIZE)
        .is_some_and(|s| s.iter().all(|&b| b == 0))
    {
        BreakConfidence::Definite
    } else {
        BreakConfidence::Probable
    }
}

fn detect_jpeg_break_at(data: &[u8]) -> Option<BreakPoint> {
    let sos_offset = find_sos_offset(data)?;
    let result = detect_jpeg_break(data, sos_offset)?;
    Some(BreakPoint {
        break_offset: result.offset as Offset,
        confidence: classify_break_confidence(data, result.offset),
        signature: ContinuationSignature::JpegScanData,
        last_rst_index: result.last_rst_index,
    })
}

fn detect_png_break_at(data: &[u8]) -> Option<BreakPoint> {
    let relative = detect_png_break(data)?;
    Some(BreakPoint {
        break_offset: relative as Offset,
        confidence: classify_break_confidence(data, relative),
        signature: ContinuationSignature::PngIdat,
        last_rst_index: None,
    })
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
    header_score: fn(&[u8]) -> Option<u8>,
    hints: Option<&FsHintMap>,
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
                            header_score,
                            hints,
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
    header_score: fn(&[u8]) -> Option<u8>,
    hints: Option<&FsHintMap>,
) -> Option<RecoveredFile> {
    detect_buf.clear();

    read_range(
        reader,
        header.offset,
        BREAK_DETECTION_READ_SIZE as u64,
        detect_buf,
        buffer,
    )?;

    let confidence = header_score(detect_buf)?;

    let bp = detect_break(detect_buf)?;

    if bp.break_offset < MIN_FRAGMENT_SIZE {
        return None;
    }

    let absolute_break = header.offset + bp.break_offset;
    let first_fragment = header.offset..absolute_break;
    let first_size = bp.break_offset;

    build_chain(
        header,
        vec![first_fragment],
        absolute_break,
        first_size,
        footers,
        spec,
        reader,
        buffer,
        detect_buf,
        detect_break,
        matches_continuation,
        confidence,
        hints,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_chain(
    header: &Fragment,
    fragments: Vec<Range<u64>>,
    break_offset: u64,
    total_size: u64,
    footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
    detect_buf: &mut Vec<u8>,
    detect_break: fn(&[u8]) -> Option<BreakPoint>,
    matches_continuation: fn(&[u8]) -> bool,
    confidence: u8,
    hints: Option<&FsHintMap>,
    depth: u8,
) -> Option<RecoveredFile> {
    if depth == 1 {
        if let Some(hint_map) = hints {
            if let Some(hint) = hint_map.get(&header.offset) {
                if let Some(result) = try_hint_guided(
                    header, &fragments, total_size, hint, footers, spec, reader, buffer, confidence,
                ) {
                    return Some(result);
                }
            }
        }
    }

    let search_end = (break_offset + REASSEMBLY_MAX_GAP).min(reader.size());

    let candidates = scan_for_continuations(
        reader,
        break_offset,
        search_end,
        buffer,
        matches_continuation,
    );

    if candidates.is_empty() {
        return None;
    }

    for &candidate_offset in &candidates {
        let result = try_complete_with_footer(
            header,
            &fragments,
            candidate_offset,
            total_size,
            footers,
            spec,
            reader,
            buffer,
            confidence,
        );
        if result.is_some() {
            return result;
        }
    }

    if depth < MAX_CHAIN_DEPTH {
        for &candidate_offset in &candidates {
            let remaining_budget = spec.max_file_bytes.saturating_sub(total_size);
            let read_size = (BREAK_DETECTION_READ_SIZE as u64).min(remaining_budget);
            if read_size < MIN_FRAGMENT_SIZE {
                continue;
            }

            detect_buf.clear();
            if read_range(reader, candidate_offset, read_size, detect_buf, buffer).is_none() {
                continue;
            }

            if let Some(bp) = detect_break(detect_buf) {
                if bp.break_offset < MIN_FRAGMENT_SIZE {
                    continue;
                }

                let new_break = candidate_offset + bp.break_offset;
                let frag_size = bp.break_offset;
                let new_total = total_size + frag_size;

                if new_total > spec.max_file_bytes {
                    continue;
                }

                let mut new_fragments = fragments.clone();
                new_fragments.push(candidate_offset..new_break);

                let result = build_chain(
                    header,
                    new_fragments,
                    new_break,
                    new_total,
                    footers,
                    spec,
                    reader,
                    buffer,
                    detect_buf,
                    detect_break,
                    matches_continuation,
                    confidence,
                    hints,
                    depth + 1,
                );

                if result.is_some() {
                    return result;
                }
            }
        }
    }

    None
}

#[allow(clippy::too_many_arguments)]
fn try_hint_guided(
    header: &Fragment,
    first_fragments: &[Range<u64>],
    prior_size: u64,
    hint: &crate::fs::FsHint,
    _footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
    confidence: u8,
) -> Option<RecoveredFile> {
    if hint.extents.len() < 2 {
        return None;
    }

    let mut all_fragments = first_fragments.to_vec();
    let mut accumulated_size = prior_size;

    for &(extent_offset, extent_len) in &hint.extents[1..] {
        if accumulated_size >= hint.data_size {
            break;
        }
        let len = extent_len.min(hint.data_size - accumulated_size);
        all_fragments.push(extent_offset..extent_offset + len);
        accumulated_size += len;
    }

    if all_fragments.len() < 2 {
        return None;
    }

    let last_frag = all_fragments.last()?;
    if !verify_tail(reader, last_frag.end, spec, buffer) {
        return None;
    }

    if accumulated_size < MIN_PHOTO_BYTES || accumulated_size > spec.max_file_bytes {
        return None;
    }

    let depth = all_fragments.len() as u8;
    Some(RecoveredFile::new(
        FragmentRanges::Multi(all_fragments),
        RecoveryMethod::Reassembled { depth },
        spec.format,
        header.entropy,
        confidence,
    ))
}

#[allow(clippy::too_many_arguments)]
fn try_complete_with_footer(
    header: &Fragment,
    fragments: &[Range<u64>],
    continuation_offset: u64,
    prior_size: u64,
    footers: &[Fragment],
    spec: &ReassemblySpec,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
    confidence: u8,
) -> Option<RecoveredFile> {
    let remaining_budget = spec.max_file_bytes.saturating_sub(prior_size);

    let min_footer_offset = continuation_offset + MIN_FRAGMENT_SIZE;
    let max_footer_offset = continuation_offset + remaining_budget;

    let start_idx = footers.partition_point(|f| f.offset < min_footer_offset);

    for footer in &footers[start_idx..] {
        if footer.offset >= max_footer_offset {
            break;
        }

        let frag_end = footer.offset + spec.footer_size;
        let last_fragment = continuation_offset..frag_end;
        let total_size = prior_size + (frag_end - continuation_offset);

        if total_size > spec.max_file_bytes || total_size < MIN_PHOTO_BYTES {
            continue;
        }

        if !verify_tail(reader, frag_end, spec, buffer) {
            continue;
        }

        let mut all_fragments = fragments.to_vec();
        all_fragments.push(last_fragment);
        let depth = all_fragments.len() as u8;
        return Some(RecoveredFile::new(
            FragmentRanges::Multi(all_fragments),
            RecoveryMethod::Reassembled { depth },
            spec.format,
            header.entropy,
            confidence,
        ));
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
