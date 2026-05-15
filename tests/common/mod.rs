#![allow(dead_code)]

use std::io::Write;

pub const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
pub const JPEG_EOI: [u8; 2] = [0xFF, 0xD9];
pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
pub const PNG_IEND_TAG: [u8; 4] = [0x49, 0x45, 0x4E, 0x44];

pub const MARKER_DQT: u8 = 0xDB;
pub const MARKER_DHT: u8 = 0xC4;
pub const MARKER_SOF0: u8 = 0xC0;
pub const MARKER_SOF2: u8 = 0xC2;
pub const MARKER_SOS: u8 = 0xDA;

pub fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(0xFF);
    out.push(marker);
    let len = (body.len() + 2) as u16;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(body);
    out
}

pub fn single_symbol_dht(class: u8) -> Vec<u8> {
    single_symbol_dht_with_id(class, 0)
}

pub fn single_symbol_dht_with_id(class: u8, id: u8) -> Vec<u8> {
    let mut body = Vec::with_capacity(18);
    body.push((class << 4) | id);
    body.push(0x01);
    body.extend_from_slice(&[0u8; 15]);
    body.push(0x00);
    body
}

pub fn baseline_dqt() -> Vec<u8> {
    let mut body = Vec::with_capacity(65);
    body.push(0x00);
    body.extend_from_slice(&[0x01; 64]);
    body
}

pub fn baseline_sof0_8x8_grayscale() -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x08);
    body.extend_from_slice(&8u16.to_be_bytes());
    body.extend_from_slice(&8u16.to_be_bytes());
    body.push(0x01);
    body.extend_from_slice(&[0x01, 0x11, 0x00]);
    body
}

pub fn baseline_sos_single_component() -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x01);
    body.extend_from_slice(&[0x01, 0x00]);
    body.extend_from_slice(&[0x00, 0x3F, 0x00]);
    body
}

pub fn minimal_baseline_jpeg() -> Vec<u8> {
    baseline_jpeg_with_entropy(&[0x00])
}

pub fn baseline_jpeg_with_entropy(entropy: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&JPEG_SOI);
    data.extend_from_slice(&segment(MARKER_DQT, &baseline_dqt()));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht(0)));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht(1)));
    data.extend_from_slice(&segment(MARKER_SOF0, &baseline_sof0_8x8_grayscale()));
    data.extend_from_slice(&segment(MARKER_SOS, &baseline_sos_single_component()));
    data.extend_from_slice(entropy);
    data.extend_from_slice(&JPEG_EOI);
    data
}

pub fn baseline_jpeg_with_nonzero_huffman_selectors() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&JPEG_SOI);
    data.extend_from_slice(&segment(MARKER_DQT, &baseline_dqt()));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht_with_id(0, 1)));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht_with_id(1, 1)));
    data.extend_from_slice(&segment(MARKER_SOF0, &baseline_sof0_8x8_grayscale()));
    let mut sos = Vec::new();
    sos.push(0x01);
    sos.extend_from_slice(&[0x01, 0x11]);
    sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
    data.extend_from_slice(&segment(MARKER_SOS, &sos));
    data.push(0x00);
    data.extend_from_slice(&JPEG_EOI);
    data
}

pub fn baseline_jpeg_with_stuffed_entropy() -> Vec<u8> {
    baseline_jpeg_with_entropy(&[0x00, 0xFF, 0x00, 0x00])
}

pub fn multi_block_baseline_jpeg(block_size: usize, blocks: usize) -> Vec<u8> {
    let target = block_size * blocks;
    let mut entropy = vec![0x11; target.saturating_sub(256)];
    entropy.insert(0, 0x00);
    let mut jpeg = baseline_jpeg_with_entropy(&entropy);
    while jpeg.len() <= block_size && entropy.len() > 1 {
        entropy.pop();
        jpeg = baseline_jpeg_with_entropy(&entropy);
    }
    jpeg
}

pub fn progressive_jpeg() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&JPEG_SOI);
    data.extend_from_slice(&segment(MARKER_DQT, &baseline_dqt()));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht(0)));
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht(1)));
    data.extend_from_slice(&segment(MARKER_SOF2, &baseline_sof0_8x8_grayscale()));
    data.extend_from_slice(&segment(MARKER_SOS, &baseline_sos_single_component()));
    data.push(0x00);
    data.extend_from_slice(&JPEG_EOI);
    data
}

fn crc32_for(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(chunk_type);
    hasher.update(data);
    hasher.finalize()
}

pub fn png_chunk(chunk_type: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let len = body.len() as u32;
    let crc = crc32_for(chunk_type, body);
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(body);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

pub fn valid_png() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&PNG_SIGNATURE);
    let ihdr = [
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00,
    ];
    data.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    let idat = [0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01];
    data.extend_from_slice(&png_chunk(b"IDAT", &idat));
    data.extend_from_slice(&png_chunk(b"IEND", &[]));
    data
}

pub fn synthetic_device(
    prefix_garbage: usize,
    padding_garbage: usize,
    suffix_garbage: usize,
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(std::iter::repeat_n(0xABu8, prefix_garbage));
    data.extend_from_slice(&minimal_baseline_jpeg());
    data.extend(std::iter::repeat_n(0xABu8, padding_garbage));
    data.extend_from_slice(&valid_png());
    data.extend(std::iter::repeat_n(0xABu8, suffix_garbage));
    data
}

pub fn sector_aligned_device(block_size: usize, placements: &[(usize, &[u8])]) -> Vec<u8> {
    let end = placements
        .iter()
        .map(|(offset, bytes)| offset + bytes.len())
        .max()
        .unwrap_or(0);
    let len = end.div_ceil(block_size).max(1) * block_size;
    let mut data = vec![0xABu8; len];
    for (offset, bytes) in placements {
        data[*offset..*offset + bytes.len()].copy_from_slice(bytes);
    }
    data
}

pub fn write_to(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(data)?;
    file.flush()
}
