use memchr::memmem;

use crate::formats::png::PNG_SIGNATURE;
use crate::types::{calculate_entropy, Fragment, FragmentKind, FragmentMap, Offset};

const JPEG_SOI_PATTERN: [u8; 3] = [0xFF, 0xD8, 0xFF];
const JPEG_EOI_PATTERN: [u8; 2] = [0xFF, 0xD9];
const PNG_IEND_PATTERN: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44];
const ENTROPY_SAMPLE_SIZE: usize = 1024;

pub fn scan_block(offset: Offset, data: &[u8], map: &mut FragmentMap) {
    scan_jpeg_headers(offset, data, map);
    scan_jpeg_footers(offset, data, map);
    scan_png_headers(offset, data, map);
    scan_png_footers(offset, data, map);
}

fn scan_jpeg_headers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    let finder = memmem::Finder::new(&JPEG_SOI_PATTERN);
    for pos in finder.find_iter(data) {
        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
        let entropy = calculate_entropy(&data[pos..sample_end]);
        map.push(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::JpegHeader,
            entropy,
        ));
    }
}

fn scan_jpeg_footers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    let finder = memmem::Finder::new(&JPEG_EOI_PATTERN);
    for pos in finder.find_iter(data) {
        map.push(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::JpegFooter,
            0.0,
        ));
    }
}

fn scan_png_headers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    let finder = memmem::Finder::new(&PNG_SIGNATURE);
    for pos in finder.find_iter(data) {
        let sample_end = (pos + ENTROPY_SAMPLE_SIZE).min(data.len());
        let entropy = calculate_entropy(&data[pos..sample_end]);
        map.push(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::PngHeader,
            entropy,
        ));
    }
}

fn scan_png_footers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    let finder = memmem::Finder::new(&PNG_IEND_PATTERN);
    for pos in finder.find_iter(data) {
        map.push(Fragment::new(
            base_offset + pos as u64,
            FragmentKind::PngIend,
            0.0,
        ));
    }
}
