use crate::types::{calculate_entropy, JpegMetadata, QuantizationQuality};

pub const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
pub const JPEG_EOI: [u8; 2] = [0xFF, 0xD9];

const QUANTIZATION_HIGH_THRESHOLD: u16 = 25;
const QUANTIZATION_LOW_THRESHOLD: u16 = 80;
const SCAN_DATA_ENTROPY_SAMPLE: usize = 2048;
const ZERO_GAP_THRESHOLD: usize = 4096;
const ZERO_RUN_BREAK_THRESHOLD: usize = 512;
const BREAK_ENTROPY_SAMPLE: usize = 256;
const BREAK_LOW_ENTROPY: f32 = 4.0;
const CONTINUATION_MIN_ENTROPY: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub struct JpegInfo {
    pub width: u16,
    pub height: u16,
    pub metadata: JpegMetadata,
}

#[inline]
pub fn is_valid_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xC0..=0xCF |
        0xD0..=0xD9 |
        0xDA |
        0xDB |
        0xDC..=0xDF |
        0xE0..=0xEF |
        0xFE
    )
}

pub fn quick_jpeg_dimensions(data: &[u8]) -> Option<(u16, u16)> {
    if data.len() < 10 || data[0..2] != JPEG_SOI {
        return None;
    }

    if data[2] != 0xFF {
        return None;
    }

    let mut pos = 2;

    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            break;
        }

        let marker = data[pos + 1];

        if marker == 0x00 {
            pos += 2;
            continue;
        }

        if marker == 0xFF {
            pos += 1;
            continue;
        }

        if matches!(marker, 0xD0..=0xD7) {
            pos += 2;
            continue;
        }

        if marker == 0xD9 || marker == 0xDA {
            break;
        }

        if matches!(marker, 0xC0..=0xC3) {
            if pos + 8 < data.len() {
                let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                if width > 0 && height > 0 {
                    return Some((width, height));
                }
            }
            return None;
        }

        if pos + 3 >= data.len() {
            break;
        }

        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if seg_len < 2 {
            break;
        }

        pos = pos + 2 + seg_len;
    }

    None
}

pub fn validate_jpeg(data: &[u8]) -> Option<JpegInfo> {
    if data.len() < 10 || data[0..2] != JPEG_SOI {
        return None;
    }

    if data.len() > 2 && data[2] != 0xFF {
        return None;
    }

    let mut pos = 2;
    let mut width = 0u16;
    let mut height = 0u16;
    let mut has_sof = false;
    let mut metadata = JpegMetadata::default();
    let mut prev_segment_end = 2usize;

    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            if metadata.has_sos {
                pos += 1;
                continue;
            }
            break;
        }

        let marker = data[pos + 1];

        if marker == 0x00 {
            pos += 2;
            continue;
        }

        if marker == 0xFF {
            pos += 1;
            continue;
        }

        if !is_valid_marker(marker) {
            break;
        }

        metadata.marker_count += 1;

        if !metadata.has_sos && pos > prev_segment_end {
            let gap = &data[prev_segment_end..pos];
            if gap.len() > ZERO_GAP_THRESHOLD && gap.iter().all(|&b| b == 0) {
                return None;
            }
        }

        if matches!(marker, 0xD0..=0xD7) {
            pos += 2;
            continue;
        }

        if marker == 0xD9 {
            break;
        }

        if pos + 3 >= data.len() {
            break;
        }

        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if seg_len < 2 {
            break;
        }

        let seg_end = pos + 2 + seg_len;

        match marker {
            0xC0..=0xC3 => {
                if pos + 8 < data.len() {
                    height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                    width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                    has_sof = width > 0 && height > 0;
                }
            }

            0xE0 => {
                if seg_len >= 16 && pos + 9 <= data.len() && &data[pos + 4..pos + 9] == b"JFIF\x00"
                {
                    metadata.has_jfif = true;
                }
            }

            0xE1 => {
                if seg_len >= 8
                    && pos + 10 <= data.len()
                    && &data[pos + 4..pos + 10] == b"Exif\x00\x00"
                {
                    metadata.has_exif = true;
                }
            }

            0xE2 => {
                if seg_len >= 14
                    && pos + 16 <= data.len()
                    && &data[pos + 4..pos + 16] == b"ICC_PROFILE\x00"
                {
                    metadata.has_icc_profile = true;
                }
            }

            0xDB => {
                if seg_end <= data.len() && seg_len > 2 {
                    metadata.quantization_quality =
                        assess_quantization_table(&data[pos + 4..pos + 2 + seg_len]);
                }
            }

            0xDA => {
                metadata.has_sos = true;
                let scan_start = seg_end;
                let scan_sample_end = (scan_start + SCAN_DATA_ENTROPY_SAMPLE).min(data.len());
                if scan_sample_end > scan_start {
                    metadata.scan_data_entropy =
                        calculate_entropy(&data[scan_start..scan_sample_end]);
                }
            }

            _ => {}
        }

        prev_segment_end = seg_end;
        pos = seg_end;
    }

    if !has_sof || width == 0 || height == 0 {
        return None;
    }

    Some(JpegInfo {
        width,
        height,
        metadata,
    })
}

pub fn validate_jpeg_full(data: &[u8]) -> Option<JpegInfo> {
    if data.len() < 4 || data[data.len() - 2..] != JPEG_EOI {
        return None;
    }
    validate_jpeg(data)
}

fn assess_quantization_table(table_data: &[u8]) -> QuantizationQuality {
    if table_data.is_empty() {
        return QuantizationQuality::Unknown;
    }

    let precision_byte = table_data[0];
    let is_16bit = (precision_byte >> 4) & 0x01 == 1;

    let mut sum = 0u32;
    let mut count = 0u32;

    if is_16bit {
        for chunk in table_data[1..].chunks_exact(2).take(64) {
            sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
            count += 1;
        }
    } else {
        for &b in table_data[1..].iter().take(64) {
            sum += b as u32;
            count += 1;
        }
    }

    if count == 0 {
        return QuantizationQuality::Unknown;
    }

    let avg = (sum / count) as u16;

    if avg <= QUANTIZATION_HIGH_THRESHOLD {
        QuantizationQuality::High
    } else if avg <= QUANTIZATION_LOW_THRESHOLD {
        QuantizationQuality::Medium
    } else {
        QuantizationQuality::Low
    }
}

pub fn find_sos_offset(data: &[u8]) -> Option<usize> {
    if data.len() < 4 || data[0..2] != JPEG_SOI {
        return None;
    }

    let mut pos = 2;

    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            break;
        }

        let marker = data[pos + 1];

        if marker == 0x00 || marker == 0xFF {
            pos += if marker == 0xFF { 1 } else { 2 };
            continue;
        }

        if matches!(marker, 0xD0..=0xD7) {
            pos += 2;
            continue;
        }

        if marker == 0xDA {
            if pos + 3 >= data.len() {
                return None;
            }
            let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
            if seg_len < 2 {
                return None;
            }
            let scan_start = pos + 2 + seg_len;
            return (scan_start < data.len()).then_some(scan_start);
        }

        if marker == 0xD9 {
            return None;
        }

        if pos + 3 >= data.len() {
            break;
        }

        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if seg_len < 2 {
            break;
        }

        pos = pos + 2 + seg_len;
    }

    None
}

pub fn detect_jpeg_break(data: &[u8], scan_start: usize) -> Option<usize> {
    if scan_start >= data.len() {
        return None;
    }

    let mut i = scan_start;
    let mut zero_run = 0usize;

    while i < data.len() {
        if data[i] == 0x00 {
            zero_run += 1;
            if zero_run >= ZERO_RUN_BREAK_THRESHOLD {
                return Some(i + 1 - zero_run);
            }
            i += 1;
            continue;
        }
        zero_run = 0;

        if data[i] == 0xFF && i + 1 < data.len() {
            let next = data[i + 1];
            if next == 0x00 {
                i += 2;
                continue;
            }
            if matches!(next, 0xD0..=0xD7) {
                i += 2;
                continue;
            }
            if next == 0xD9 {
                return None;
            }
            return Some(i);
        }

        i += 1;
    }

    if i > scan_start + BREAK_ENTROPY_SAMPLE {
        let tail_start = data.len().saturating_sub(BREAK_ENTROPY_SAMPLE);
        let tail_entropy = calculate_entropy(&data[tail_start..]);
        if tail_entropy < BREAK_LOW_ENTROPY {
            let mut probe = tail_start;
            while probe > scan_start + BREAK_ENTROPY_SAMPLE {
                let sample_entropy = calculate_entropy(&data[probe - BREAK_ENTROPY_SAMPLE..probe]);
                if sample_entropy >= CONTINUATION_MIN_ENTROPY {
                    return Some(probe);
                }
                probe = probe.saturating_sub(BREAK_ENTROPY_SAMPLE);
            }
            return Some(tail_start);
        }
    }

    None
}

pub fn matches_jpeg_continuation(cluster_data: &[u8]) -> bool {
    if cluster_data.len() < 16 {
        return false;
    }

    let entropy = calculate_entropy(cluster_data);
    if entropy < CONTINUATION_MIN_ENTROPY {
        return false;
    }

    let mut i = 0;
    while i + 1 < cluster_data.len() {
        if cluster_data[i] == 0xFF {
            let next = cluster_data[i + 1];
            if next == 0x00 || matches!(next, 0xD0..=0xD7) {
                i += 2;
                continue;
            }
            if next == 0xD9 {
                return true;
            }
            return false;
        }
        i += 1;
    }

    true
}
