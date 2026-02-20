use std::sync::LazyLock;

use memchr::memmem;

use crate::formats::jpeg::quick_jpeg_dimensions;
use crate::formats::png::{validate_png_header, IEND_CRC, PNG_SIGNATURE};
use crate::types::{
    calculate_entropy, categorize_dimensions, DimensionVerdict, Fragment, FragmentCollector,
    FragmentKind, Offset, LOW_ENTROPY_THRESHOLD,
};

const JPEG_SOI_PATTERN: [u8; 3] = [0xFF, 0xD8, 0xFF];
const JPEG_EOI_PATTERN: [u8; 2] = [0xFF, 0xD9];
const PNG_IEND_PATTERN: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44];
const ENTROPY_SAMPLE_SIZE: usize = 1024;
const EOI_CONTEXT_WINDOW: usize = 256;
const EOI_MIN_CONTEXT_ENTROPY: f32 = 6.0;

static JPEG_SOI_FINDER: LazyLock<memmem::Finder<'static>> =
    LazyLock::new(|| memmem::Finder::new(&JPEG_SOI_PATTERN));
static JPEG_EOI_FINDER: LazyLock<memmem::Finder<'static>> =
    LazyLock::new(|| memmem::Finder::new(&JPEG_EOI_PATTERN));
static PNG_SIG_FINDER: LazyLock<memmem::Finder<'static>> =
    LazyLock::new(|| memmem::Finder::new(&PNG_SIGNATURE));
static PNG_IEND_FINDER: LazyLock<memmem::Finder<'static>> =
    LazyLock::new(|| memmem::Finder::new(&PNG_IEND_PATTERN));

pub fn scan_block(offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    scan_jpeg_headers(offset, data, collector);
    scan_jpeg_footers(offset, data, collector);
    scan_png_headers(offset, data, collector);
    scan_png_footers(offset, data, collector);
}

fn scan_jpeg_headers(base_offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    for pos in JPEG_SOI_FINDER.find_iter(data) {
        let verdict = if let Some((w, h)) = quick_jpeg_dimensions(&data[pos..]) {
            let v = categorize_dimensions(w as u32, h as u32);
            match v {
                DimensionVerdict::Photo => v,
                DimensionVerdict::Asset | DimensionVerdict::TooSmall => continue,
            }
        } else {
            DimensionVerdict::Photo
        };

        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
        let entropy = calculate_entropy(&data[pos..sample_end]);

        if entropy < LOW_ENTROPY_THRESHOLD {
            continue;
        }

        collector.collect(Fragment::with_verdict(
            base_offset + pos as u64,
            FragmentKind::JpegHeader,
            entropy,
            verdict,
        ));
    }
}

fn scan_jpeg_footers(base_offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    for pos in JPEG_EOI_FINDER.find_iter(data) {
        if pos > 0 && data[pos - 1] == 0x00 {
            continue;
        }

        if pos >= EOI_CONTEXT_WINDOW {
            let context = &data[pos - EOI_CONTEXT_WINDOW..pos];
            let context_entropy = calculate_entropy(context);
            if context_entropy < EOI_MIN_CONTEXT_ENTROPY {
                continue;
            }
            if !is_valid_scan_context(context) {
                continue;
            }
        }

        collector.collect(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::JpegFooter,
            0.0,
        ));
    }
}

#[inline]
fn is_valid_scan_context(context: &[u8]) -> bool {
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

fn scan_png_headers(base_offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    for pos in PNG_SIG_FINDER.find_iter(data) {
        let verdict = if let Some(info) = validate_png_header(&data[pos..]) {
            let v = categorize_dimensions(info.width, info.height);
            match v {
                DimensionVerdict::Photo => v,
                DimensionVerdict::Asset | DimensionVerdict::TooSmall => continue,
            }
        } else {
            DimensionVerdict::Photo
        };

        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
        let entropy = calculate_entropy(&data[pos..sample_end]);

        if entropy < LOW_ENTROPY_THRESHOLD {
            continue;
        }

        collector.collect(Fragment::with_verdict(
            base_offset + pos as u64,
            FragmentKind::PngHeader,
            entropy,
            verdict,
        ));
    }
}

fn scan_png_footers(base_offset: Offset, data: &[u8], collector: &mut impl FragmentCollector) {
    let expected_crc = IEND_CRC.to_be_bytes();
    for pos in PNG_IEND_FINDER.find_iter(data) {
        if pos + 12 > data.len() || data[pos + 8..pos + 12] != expected_crc {
            continue;
        }
        collector.collect(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::PngIend,
            0.0,
        ));
    }
}
