use crate::types::PngMetadata;

pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

const SCREEN_PPM_LOWER: u32 = 2500;
const SCREEN_PPM_UPPER: u32 = 3200;
const IDAT_MIN_RATIO: u64 = 100;

pub const IEND_CHUNK_TYPE: &[u8; 4] = b"IEND";

pub const IEND_CRC: u32 = 0xAE426082;

#[derive(Debug, Clone, Copy)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub metadata: PngMetadata,
    pub idat_count: usize,
    pub idat_total_bytes: u64,
}

#[inline]
pub fn validate_png_header(data: &[u8]) -> Option<PngInfo> {
    if data.len() < 33 {
        return None;
    }

    if data[..8] != PNG_SIGNATURE {
        return None;
    }

    if &data[12..16] != b"IHDR" {
        return None;
    }

    let ihdr_len = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    if ihdr_len != 13 {
        return None;
    }

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data[12..29]);
    let calculated = hasher.finalize();
    let stored = u32::from_be_bytes([data[29], data[30], data[31], data[32]]);

    if calculated != stored {
        return None;
    }

    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

    let mut metadata = PngMetadata::default();
    let mut idat_count = 0usize;
    let mut idat_total_bytes = 0u64;
    let mut unique_chunk_types = 0u8;

    if let Some(iter) = PngChunkIterator::new(data) {
        for (_, chunk_type, payload) in iter {
            match &chunk_type {
                b"IDAT" => {
                    idat_count += 1;
                    idat_total_bytes += payload.len() as u64;
                }
                b"tEXt" | b"iTXt" | b"zTXt" => {
                    metadata.has_text_chunks = true;
                    unique_chunk_types += 1;
                }
                b"iCCP" => {
                    metadata.has_icc_profile = true;
                    unique_chunk_types += 1;
                }
                b"pHYs" => {
                    metadata.has_physical_dimensions = true;
                    unique_chunk_types += 1;
                    if payload.len() >= 9 {
                        let ppu_x =
                            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        let unit = payload[8];
                        if unit == 1 {
                            metadata.is_screen_resolution =
                                (SCREEN_PPM_LOWER..=SCREEN_PPM_UPPER).contains(&ppu_x);
                        }
                    }
                }
                b"gAMA" | b"cHRM" | b"sRGB" | b"sBIT" | b"bKGD" | b"hIST" | b"tRNS" | b"sPLT"
                | b"tIME" => {
                    unique_chunk_types += 1;
                }
                _ => {}
            }
        }
    }

    metadata.chunk_variety = unique_chunk_types;

    Some(PngInfo {
        width,
        height,
        metadata,
        idat_count,
        idat_total_bytes,
    })
}

pub fn validate_png_full(data: &[u8]) -> Option<PngInfo> {
    let info = validate_png_header(data)?;

    if info.idat_count == 0 {
        return None;
    }

    let pixel_count = info.width as u64 * info.height as u64;
    if pixel_count > 0
        && info.idat_total_bytes > 0
        && info.idat_total_bytes < pixel_count / IDAT_MIN_RATIO
    {
        return None;
    }

    if !has_valid_iend(data) {
        return None;
    }

    Some(info)
}

fn has_valid_iend(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    let iend_start = data.len() - 12;
    let iend_len = u32::from_be_bytes([
        data[iend_start],
        data[iend_start + 1],
        data[iend_start + 2],
        data[iend_start + 3],
    ]);
    if iend_len != 0 || &data[iend_start + 4..iend_start + 8] != IEND_CHUNK_TYPE {
        return false;
    }
    let stored = u32::from_be_bytes([
        data[iend_start + 8],
        data[iend_start + 9],
        data[iend_start + 10],
        data[iend_start + 11],
    ]);
    IEND_CRC == stored
}

pub struct PngChunkIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PngChunkIterator<'a> {
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 8 || data[..8] != PNG_SIGNATURE {
            return None;
        }

        Some(Self { data, pos: 8 })
    }
}

impl<'a> Iterator for PngChunkIterator<'a> {
    type Item = (&'a [u8], [u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 12 > self.data.len() {
            return None;
        }

        let length = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]) as usize;

        let chunk_type: [u8; 4] = [
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ];

        let total_size = 4 + 4 + length + 4;
        if self.pos + total_size > self.data.len() {
            return None;
        }

        let chunk_data = &self.data[self.pos..self.pos + total_size];
        let payload = &self.data[self.pos + 8..self.pos + 8 + length];

        self.pos += total_size;

        Some((chunk_data, chunk_type, payload))
    }
}
