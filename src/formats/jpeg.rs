use crate::types::{JpegMetadata, QuantizationQuality};

pub const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
pub const JPEG_EOI: [u8; 2] = [0xFF, 0xD9];

const QUANTIZATION_HIGH_THRESHOLD: u16 = 25;
const QUANTIZATION_LOW_THRESHOLD: u16 = 80;

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
    let mut has_sos = false;
    let mut metadata = JpegMetadata::default();

    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            if has_sos {
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
                has_sos = true;
            }

            _ => {}
        }

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

pub fn validate_jpeg_structure(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    if data[0..2] != JPEG_SOI || data[data.len() - 2..] != JPEG_EOI {
        return false;
    }

    validate_jpeg(data).is_some()
}

#[inline]
pub fn find_jpeg_footer(data: &[u8]) -> Option<usize> {
    if data.len() < 2 {
        return None;
    }
    (0..data.len().saturating_sub(1)).find(|&i| data[i..i + 2] == JPEG_EOI)
}

fn assess_quantization_table(table_data: &[u8]) -> QuantizationQuality {
    if table_data.is_empty() {
        return QuantizationQuality::Unknown;
    }

    let precision_byte = table_data[0];
    let is_16bit = (precision_byte >> 4) & 0x01 == 1;

    let values: Vec<u16> = if is_16bit {
        table_data[1..]
            .chunks_exact(2)
            .take(64)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect()
    } else {
        table_data[1..].iter().take(64).map(|&b| b as u16).collect()
    };

    if values.is_empty() {
        return QuantizationQuality::Unknown;
    }

    let avg: u16 = (values.iter().map(|&v| v as u32).sum::<u32>() / values.len() as u32) as u16;

    if avg <= QUANTIZATION_HIGH_THRESHOLD {
        QuantizationQuality::High
    } else if avg <= QUANTIZATION_LOW_THRESHOLD {
        QuantizationQuality::Medium
    } else {
        QuantizationQuality::Low
    }
}
