#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
}

#[allow(dead_code)]
pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[allow(dead_code)]
pub const PNG_IEND_CHUNK: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

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
    let bit_depth = data[24];
    let color_type = data[25];

    Some(PngInfo {
        width,
        height,
        bit_depth,
        color_type,
    })
}

#[inline]
#[allow(dead_code)]
pub fn find_png_iend(data: &[u8]) -> Option<usize> {
    if data.len() < 12 {
        return None;
    }

    for i in 0..=data.len().saturating_sub(12) {
        if i + 12 <= data.len() && data[i..i + 4] == [0x00, 0x00, 0x00, 0x00] {
            if &data[i + 4..i + 8] == b"IEND" {
                let mut hasher = crc32fast::Hasher::new();
                hasher.update(b"IEND");
                let calculated = hasher.finalize();
                let stored =
                    u32::from_be_bytes([data[i + 8], data[i + 9], data[i + 10], data[i + 11]]);

                if calculated == stored {
                    return Some(i);
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
pub fn validate_chunk(data: &[u8]) -> Option<([u8; 4], usize)> {
    if data.len() < 12 {
        return None;
    }

    let length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let chunk_type: [u8; 4] = [data[4], data[5], data[6], data[7]];

    let total_size = 4 + 4 + length + 4;

    if data.len() < total_size {
        return None;
    }

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data[4..8 + length]);

    let calculated = hasher.finalize();

    let crc_offset = 8 + length;
    let stored = u32::from_be_bytes([
        data[crc_offset],
        data[crc_offset + 1],
        data[crc_offset + 2],
        data[crc_offset + 3],
    ]);

    if calculated != stored {
        return None;
    }

    Some((chunk_type, total_size))
}

#[allow(dead_code)]
pub struct PngChunkIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

#[allow(dead_code)]
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
