pub mod jpeg;
pub mod png;

use crate::core::traits::FormatStrategy;
use crate::core::{BreakConfidence, BreakPoint, ContinuationSignature, ImageFormat, Offset};

pub struct JpegFormat;

impl FormatStrategy for JpegFormat {
    const FORMAT: ImageFormat = ImageFormat::Jpeg;
    const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;
    const MIN_FILE_SIZE: u64 = 50 * 1024;
    const FOOTER_SIZE: u64 = 2;
    const VALIDATE_FOOTER_CONTEXT: bool = true;

    fn candidate_score(data: &[u8]) -> Option<u8> {
        jpeg::candidate_score(data)
    }

    fn detect_break(data: &[u8]) -> Option<BreakPoint> {
        let sos_offset = jpeg::find_sos_offset(data)?;
        let result = jpeg::detect_jpeg_break(data, sos_offset)?;
        let confidence = if data
            .get(result.offset..result.offset + 512)
            .is_some_and(|s| s.iter().all(|&b| b == 0))
        {
            BreakConfidence::Definite
        } else {
            BreakConfidence::Probable
        };
        Some(BreakPoint {
            break_offset: result.offset as Offset,
            confidence,
            signature: ContinuationSignature::JpegScanData,
            last_rst_index: result.last_rst_index,
        })
    }

    fn matches_continuation(cluster_data: &[u8]) -> bool {
        jpeg::matches_jpeg_continuation(cluster_data)
    }
}

pub struct PngFormat;

impl FormatStrategy for PngFormat {
    const FORMAT: ImageFormat = ImageFormat::Png;
    const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
    const MIN_FILE_SIZE: u64 = 50 * 1024;
    const FOOTER_SIZE: u64 = 12;
    const VALIDATE_FOOTER_CONTEXT: bool = false;

    fn candidate_score(data: &[u8]) -> Option<u8> {
        png::candidate_score(data)
    }

    fn detect_break(data: &[u8]) -> Option<BreakPoint> {
        let offset = png::detect_png_break(data)?;
        Some(BreakPoint {
            break_offset: offset as Offset,
            confidence: BreakConfidence::Probable,
            signature: ContinuationSignature::PngIdat,
            last_rst_index: None,
        })
    }

    fn matches_continuation(cluster_data: &[u8]) -> bool {
        png::matches_png_continuation(cluster_data)
    }
}
