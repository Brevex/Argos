use crate::formats::{jpeg, png};
use crate::types::{Fragment, FragmentKind, FragmentMap, Offset};

#[inline]
pub fn entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }

    let mut freq = [0u32; 256];

    for &byte in data {
        freq[byte as usize] += 1;
    }

    let total = data.len() as f32;
    let mut h = 0.0f32;

    for &count in &freq {
        if count > 0 {
            let p = count as f32 / total;
            h -= p * p.log2();
        }
    }
    h
}

pub const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[inline]
#[allow(dead_code)]
pub fn looks_like_compressed_image(data: &[u8]) -> bool {
    let ent = entropy(data);
    ent > 7.0 && ent < 8.0
}

pub fn scan_block(offset: Offset, data: &[u8], map: &mut FragmentMap) {
    scan_jpeg_headers(offset, data, map);
    scan_jpeg_footers(offset, data, map);
    scan_png_headers(offset, data, map);
    scan_png_footers(offset, data, map);
}

fn scan_jpeg_headers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    if data.len() < 10 {
        return;
    }

    for i in 0..data.len().saturating_sub(10) {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            if jpeg::validate_jpeg_header(&data[i..]).is_some() {
                let end = (i + 1024).min(data.len());
                let ent = entropy(&data[i..end]);

                map.push(Fragment::new(
                    base_offset + i as u64,
                    0,
                    FragmentKind::JpegHeader,
                    ent,
                ));
            }
        }
    }
}

fn scan_jpeg_footers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    if data.len() < 2 {
        return;
    }

    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            let is_valid = if i > 0 {
                let start = i.saturating_sub(100);
                let ent = entropy(&data[start..i]);
                ent > 6.0
            } else {
                true
            };

            if is_valid {
                map.push(Fragment::new(
                    base_offset + i as u64,
                    2,
                    FragmentKind::JpegFooter,
                    0.0,
                ));
            }
        }
    }
}

fn scan_png_headers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    if data.len() < 33 {
        return;
    }

    for i in 0..data.len().saturating_sub(33) {
        if data[i] == 0x89 && &data[i..i + 8] == &PNG_MAGIC {
            if png::validate_png_header(&data[i..]).is_some() {
                let end = (i + 1024).min(data.len());
                let ent = entropy(&data[i..end]);

                map.push(Fragment::new(
                    base_offset + i as u64,
                    0,
                    FragmentKind::PngHeader,
                    ent,
                ));
            }
        }
    }
}

fn scan_png_footers(base_offset: Offset, data: &[u8], map: &mut FragmentMap) {
    if data.len() < 12 {
        return;
    }

    for i in 0..data.len().saturating_sub(12) {
        if &data[i..i + 4] == &[0x00, 0x00, 0x00, 0x00] && &data[i + 4..i + 8] == b"IEND" {
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(b"IEND");
            let calculated = hasher.finalize();
            let stored = u32::from_be_bytes([data[i + 8], data[i + 9], data[i + 10], data[i + 11]]);

            if calculated == stored {
                map.push(Fragment::new(
                    base_offset + i as u64,
                    12,
                    FragmentKind::PngIend,
                    0.0,
                ));
            }
        }
    }
}
