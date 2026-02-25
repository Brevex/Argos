use crate::formats::jpeg::quick_jpeg_dimensions;
use crate::formats::png::{validate_png_header, IEND_CRC, PNG_SIGNATURE};
use crate::types::{
    calculate_entropy, categorize_dimensions, DimensionVerdict, Fragment, FragmentCollector,
    FragmentKind, Offset, LOW_ENTROPY_THRESHOLD,
};

const ENTROPY_SAMPLE_SIZE: usize = 1024;
const EOI_CONTEXT_WINDOW: usize = 512;
const EOI_MIN_CONTEXT_ENTROPY: f32 = 7.0;

pub fn scan_block(offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    let expected_crc = IEND_CRC.to_be_bytes();

    for pos in memchr::memchr3_iter(0xFF, 0x89, 0x49, data) {
        match data[pos] {
            0xFF => {
                if pos + 2 < data.len() && data[pos + 1] == 0xD8 && data[pos + 2] == 0xFF {
                    'soi: {
                        let verdict = if let Some((w, h)) = quick_jpeg_dimensions(&data[pos..]) {
                            match categorize_dimensions(w as u32, h as u32) {
                                DimensionVerdict::Photo => DimensionVerdict::Photo,
                                _ => break 'soi,
                            }
                        } else {
                            DimensionVerdict::Photo
                        };

                        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
                        let entropy = calculate_entropy(&data[pos..sample_end]);
                        if entropy < LOW_ENTROPY_THRESHOLD {
                            break 'soi;
                        }

                        collector.collect(Fragment::with_verdict(
                            offset + pos as u64,
                            FragmentKind::JpegHeader,
                            entropy,
                            verdict,
                        ));
                    }
                } else if pos + 1 < data.len() && data[pos + 1] == 0xD9 {
                    'eoi: {
                        if pos > 0 && data[pos - 1] == 0x00 {
                            break 'eoi;
                        }

                        if pos < EOI_CONTEXT_WINDOW {
                            break 'eoi;
                        }

                        let context = &data[pos - EOI_CONTEXT_WINDOW..pos];
                        let context_entropy = calculate_entropy(context);
                        if context_entropy < EOI_MIN_CONTEXT_ENTROPY {
                            break 'eoi;
                        }
                        if !is_valid_scan_context(context) {
                            break 'eoi;
                        }

                        collector.collect(Fragment::new(
                            offset + pos as u64,
                            FragmentKind::JpegFooter,
                            0.0,
                        ));
                    }
                }
            }
            0x89 => {
                if pos + 8 <= data.len() && data[pos..pos + 8] == PNG_SIGNATURE {
                    'png: {
                        let verdict = if let Some(info) = validate_png_header(&data[pos..]) {
                            match categorize_dimensions(info.width, info.height) {
                                DimensionVerdict::Photo => DimensionVerdict::Photo,
                                _ => break 'png,
                            }
                        } else {
                            DimensionVerdict::Photo
                        };

                        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
                        let entropy = calculate_entropy(&data[pos..sample_end]);
                        if entropy < LOW_ENTROPY_THRESHOLD {
                            break 'png;
                        }

                        collector.collect(Fragment::with_verdict(
                            offset + pos as u64,
                            FragmentKind::PngHeader,
                            entropy,
                            verdict,
                        ));
                    }
                }
            }
            0x49 => {
                if pos >= 4
                    && pos + 4 <= data.len()
                    && data[pos - 4..pos] == [0x00, 0x00, 0x00, 0x00]
                    && data[pos..pos + 4] == *b"IEND"
                {
                    let iend_pos = pos - 4;
                    if iend_pos + 12 <= data.len()
                        && data[iend_pos + 8..iend_pos + 12] == expected_crc
                    {
                        collector.collect(Fragment::new(
                            offset + iend_pos as u64,
                            FragmentKind::PngIend,
                            0.0,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

#[inline]
pub fn is_valid_scan_context(context: &[u8]) -> bool {
    let mut i = 0;
    while i + 1 < context.len() {
        if context[i] == 0xFF {
            let next = context[i + 1];
            if next != 0x00 && !(0xD0..=0xD9).contains(&next) {
                return false;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    true
}
