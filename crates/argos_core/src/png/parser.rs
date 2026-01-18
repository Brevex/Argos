use super::{chunk_crc, PNG_SIGNATURE};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkType {
    Ihdr,
    Plte,
    Idat,
    Iend,
    Gama,
    Chrm,
    Srgb,
    Iccp,
    Text,
    Ztxt,
    Itxt,
    Bkgd,
    Phys,
    Time,
    Other([u8; 4]),
}

impl ChunkType {
    pub fn from_bytes(bytes: &[u8; 4]) -> Self {
        match bytes {
            b"IHDR" => Self::Ihdr,
            b"PLTE" => Self::Plte,
            b"IDAT" => Self::Idat,
            b"IEND" => Self::Iend,
            b"gAMA" => Self::Gama,
            b"cHRM" => Self::Chrm,
            b"sRGB" => Self::Srgb,
            b"iCCP" => Self::Iccp,
            b"tEXt" => Self::Text,
            b"zTXt" => Self::Ztxt,
            b"iTXt" => Self::Itxt,
            b"bKGD" => Self::Bkgd,
            b"pHYs" => Self::Phys,
            b"tIME" => Self::Time,
            _ => Self::Other(*bytes),
        }
    }

    pub fn to_bytes(&self) -> [u8; 4] {
        match self {
            Self::Ihdr => *b"IHDR",
            Self::Plte => *b"PLTE",
            Self::Idat => *b"IDAT",
            Self::Iend => *b"IEND",
            Self::Gama => *b"gAMA",
            Self::Chrm => *b"cHRM",
            Self::Srgb => *b"sRGB",
            Self::Iccp => *b"iCCP",
            Self::Text => *b"tEXt",
            Self::Ztxt => *b"zTXt",
            Self::Itxt => *b"iTXt",
            Self::Bkgd => *b"bKGD",
            Self::Phys => *b"pHYs",
            Self::Time => *b"tIME",
            Self::Other(b) => *b,
        }
    }

    #[inline]
    pub fn is_critical(&self) -> bool {
        super::is_critical_chunk(&self.to_bytes())
    }
}

#[derive(Debug, Clone)]
pub struct PngChunk {
    pub length: u32,
    pub chunk_type: ChunkType,
    pub chunk_type_bytes: [u8; 4],
    pub offset: u64,
    pub stored_crc: u32,
    pub calculated_crc: u32,
    pub crc_valid: bool,
}

impl PngChunk {
    #[inline]
    pub const fn total_size(&self) -> u64 {
        4 + 4 + self.length as u64 + 4
    }

    #[inline]
    pub const fn data_offset(&self) -> u64 {
        self.offset + 8
    }

    #[inline]
    pub const fn crc_offset(&self) -> u64 {
        self.offset + 8 + self.length as u64
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IhdrData {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub compression: u8,
    pub filter: u8,
    pub interlace: u8,
}

impl IhdrData {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 13 {
            return None;
        }

        Some(Self {
            width: u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            height: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            bit_depth: data[8],
            color_type: data[9],
            compression: data[10],
            filter: data[11],
            interlace: data[12],
        })
    }

    pub fn is_valid(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }

        let valid_bit_depth = match self.color_type {
            0 => matches!(self.bit_depth, 1 | 2 | 4 | 8 | 16),
            2 => matches!(self.bit_depth, 8 | 16),
            3 => matches!(self.bit_depth, 1 | 2 | 4 | 8),
            4 => matches!(self.bit_depth, 8 | 16),
            6 => matches!(self.bit_depth, 8 | 16),
            _ => false,
        };

        if !valid_bit_depth {
            return false;
        }

        if self.compression != 0 || self.filter != 0 {
            return false;
        }

        if self.interlace > 1 {
            return false;
        }

        true
    }
}

#[derive(Debug, Clone)]
pub struct PngStructure {
    pub chunks: Vec<PngChunk>,
    pub ihdr: Option<IhdrData>,
    pub idat_offsets: Vec<u64>,
    pub idat_total_size: u64,
    pub corruption_point: Option<u64>,
    pub invalid_crc_count: usize,
    pub valid_end_offset: u64,
}

impl Default for PngStructure {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            ihdr: None,
            idat_offsets: Vec::new(),
            idat_total_size: 0,
            corruption_point: None,
            invalid_crc_count: 0,
            valid_end_offset: 0,
        }
    }
}

pub struct PngParser;

impl PngParser {
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    pub fn parse(&self, data: &[u8]) -> Result<PngStructure> {
        if data.len() < super::MIN_PNG_SIZE {
            return Err(CoreError::InvalidFormat("Data too short for PNG".into()));
        }

        if data[..8] != PNG_SIGNATURE {
            return Err(CoreError::InvalidFormat("Missing PNG signature".into()));
        }

        let mut structure = PngStructure::default();
        let mut pos: usize = 8;

        while pos + 12 <= data.len() {
            let length =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);

            if length > 0x7FFFFFFF {
                structure.corruption_point = Some(pos as u64);
                break;
            }

            let chunk_total = 4 + 4 + length as usize + 4;
            if pos + chunk_total > data.len() {
                structure.corruption_point = Some(pos as u64);
                break;
            }

            let chunk_type_bytes: [u8; 4] =
                [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
            let chunk_type = ChunkType::from_bytes(&chunk_type_bytes);

            let data_start = pos + 8;
            let data_end = data_start + length as usize;
            let chunk_data = &data[data_start..data_end];

            let stored_crc = u32::from_be_bytes([
                data[data_end],
                data[data_end + 1],
                data[data_end + 2],
                data[data_end + 3],
            ]);

            let calculated_crc = chunk_crc(&chunk_type_bytes, chunk_data);
            let crc_valid = stored_crc == calculated_crc;

            if !crc_valid {
                structure.invalid_crc_count += 1;
            }

            let chunk = PngChunk {
                length,
                chunk_type,
                chunk_type_bytes,
                offset: pos as u64,
                stored_crc,
                calculated_crc,
                crc_valid,
            };

            match chunk_type {
                ChunkType::Ihdr => {
                    if let Some(ihdr) = IhdrData::from_bytes(chunk_data) {
                        structure.ihdr = Some(ihdr);
                    }
                }
                ChunkType::Idat => {
                    structure.idat_offsets.push(chunk.data_offset());
                    structure.idat_total_size += length as u64;
                }
                ChunkType::Iend => {
                    structure.chunks.push(chunk);
                    structure.valid_end_offset = (pos + chunk_total) as u64;
                    return Ok(structure);
                }
                _ => {}
            }

            structure.chunks.push(chunk);
            pos += chunk_total;
        }

        if structure.valid_end_offset == 0 {
            structure.valid_end_offset = pos as u64;
        }

        Ok(structure)
    }

    pub fn extract_chunk_data<'a>(&self, data: &'a [u8], chunk: &PngChunk) -> Option<&'a [u8]> {
        let start = chunk.data_offset() as usize;
        let end = start + chunk.length as usize;

        if end <= data.len() {
            Some(&data[start..end])
        } else {
            None
        }
    }
}

impl Default for PngParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{IDAT, IEND, IHDR};
    use super::*;

    fn create_minimal_png() -> Vec<u8> {
        let mut data = Vec::new();

        data.extend_from_slice(&PNG_SIGNATURE);

        let ihdr_data = [
            0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        let ihdr_len = (ihdr_data.len() as u32).to_be_bytes();
        let ihdr_crc = chunk_crc(&IHDR, &ihdr_data).to_be_bytes();

        data.extend_from_slice(&ihdr_len);
        data.extend_from_slice(&IHDR);
        data.extend_from_slice(&ihdr_data);
        data.extend_from_slice(&ihdr_crc);

        let idat_data = [0x08, 0xD7, 0x63, 0x60, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01];
        let idat_len = (idat_data.len() as u32).to_be_bytes();
        let idat_crc = chunk_crc(&IDAT, &idat_data).to_be_bytes();

        data.extend_from_slice(&idat_len);
        data.extend_from_slice(&IDAT);
        data.extend_from_slice(&idat_data);
        data.extend_from_slice(&idat_crc);

        let iend_len = 0u32.to_be_bytes();
        let iend_crc = chunk_crc(&IEND, &[]).to_be_bytes();

        data.extend_from_slice(&iend_len);
        data.extend_from_slice(&IEND);
        data.extend_from_slice(&iend_crc);

        data
    }

    #[test]
    fn test_parse_minimal_png() {
        let data = create_minimal_png();
        let parser = PngParser::new();
        let structure = parser.parse(&data).unwrap();

        assert_eq!(structure.chunks.len(), 3);
        assert!(structure.ihdr.is_some());

        let ihdr = structure.ihdr.unwrap();
        assert_eq!(ihdr.width, 16);
        assert_eq!(ihdr.height, 16);
        assert_eq!(ihdr.bit_depth, 8);
        assert_eq!(ihdr.color_type, 2);

        assert_eq!(structure.idat_offsets.len(), 1);
        assert!(structure.corruption_point.is_none());
        assert_eq!(structure.invalid_crc_count, 0);
    }

    #[test]
    fn test_parse_invalid_signature() {
        let parser = PngParser::new();
        let data = vec![0x00; 100];
        assert!(parser.parse(&data).is_err());
    }

    #[test]
    fn test_parse_too_short() {
        let parser = PngParser::new();
        let data = PNG_SIGNATURE.to_vec();
        assert!(parser.parse(&data).is_err());
    }

    #[test]
    fn test_ihdr_validation() {
        let valid = IhdrData {
            width: 100,
            height: 100,
            bit_depth: 8,
            color_type: 2,
            compression: 0,
            filter: 0,
            interlace: 0,
        };
        assert!(valid.is_valid());

        let invalid_width = IhdrData { width: 0, ..valid };
        assert!(!invalid_width.is_valid());

        let invalid_bit_depth = IhdrData {
            bit_depth: 5,
            ..valid
        };
        assert!(!invalid_bit_depth.is_valid());

        let invalid_compression = IhdrData {
            compression: 1,
            ..valid
        };
        assert!(!invalid_compression.is_valid());
    }

    #[test]
    fn test_chunk_type_roundtrip() {
        let types = [
            ChunkType::Ihdr,
            ChunkType::Idat,
            ChunkType::Iend,
            ChunkType::Plte,
            ChunkType::Other(*b"xxxx"),
        ];

        for chunk_type in types {
            let bytes = chunk_type.to_bytes();
            let parsed = ChunkType::from_bytes(&bytes);
            assert_eq!(chunk_type, parsed);
        }
    }

    #[test]
    fn test_crc_validation() {
        let mut data = create_minimal_png();
        let parser = PngParser::new();

        let structure = parser.parse(&data).unwrap();
        assert_eq!(structure.invalid_crc_count, 0);

        data[16] ^= 0xFF;
        let corrupted = parser.parse(&data).unwrap();
        assert!(corrupted.invalid_crc_count > 0);
    }
}
