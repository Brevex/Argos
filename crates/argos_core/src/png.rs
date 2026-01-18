use crate::error::{CoreError, Result};

pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
pub const IHDR: [u8; 4] = *b"IHDR";
pub const IDAT: [u8; 4] = *b"IDAT";
pub const IEND: [u8; 4] = *b"IEND";
pub const PLTE: [u8; 4] = *b"PLTE";
pub const TEXT: [u8; 4] = *b"tEXt";
pub const ZTXT: [u8; 4] = *b"zTXt";
pub const ITXT: [u8; 4] = *b"iTXt";
pub const GAMA: [u8; 4] = *b"gAMA";
pub const CHRM: [u8; 4] = *b"cHRM";
pub const SRGB: [u8; 4] = *b"sRGB";
pub const ICCP: [u8; 4] = *b"iCCP";
pub const MIN_PNG_SIZE: usize = 8 + 25 + 12;

const CRC_TABLE: [u32; 256] = generate_crc_table();

const fn generate_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let poly: u32 = 0xEDB88320;
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

#[inline]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }
    !crc
}

#[inline]
pub fn chunk_crc(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in chunk_type {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }
    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }
    !crc
}

#[inline]
pub const fn is_critical_chunk(chunk_type: &[u8; 4]) -> bool {
    chunk_type[0] & 0x20 == 0
}

#[inline]
pub const fn is_ancillary_chunk(chunk_type: &[u8; 4]) -> bool {
    chunk_type[0] & 0x20 != 0
}

#[inline]
pub const fn is_safe_to_copy(chunk_type: &[u8; 4]) -> bool {
    chunk_type[3] & 0x20 != 0
}

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
        is_critical_chunk(&self.to_bytes())
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
            2 | 4 | 6 => matches!(self.bit_depth, 8 | 16),
            3 => matches!(self.bit_depth, 1 | 2 | 4 | 8),
            _ => false,
        };
        valid_bit_depth && self.compression == 0 && self.filter == 0 && self.interlace <= 1
    }
}

#[derive(Debug, Clone, Default)]
pub struct PngStructure {
    pub chunks: Vec<PngChunk>,
    pub ihdr: Option<IhdrData>,
    pub idat_offsets: Vec<u64>,
    pub idat_total_size: u64,
    pub corruption_point: Option<u64>,
    pub invalid_crc_count: usize,
    pub valid_end_offset: u64,
}

pub struct PngParser;

impl PngParser {
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    pub fn parse(&self, data: &[u8]) -> Result<PngStructure> {
        if data.len() < MIN_PNG_SIZE {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngCorruptionReason {
    MissingIhdr,
    MissingIend,
    InvalidIhdr,
    CrcMismatch {
        chunk_offset: u64,
        chunk_type: [u8; 4],
    },
    InvalidChunkLength {
        offset: u64,
    },
    IdatDecompressionError,
    UnexpectedEof,
    InvalidChunkOrder,
}

#[derive(Debug, Clone)]
pub enum PngValidationResult {
    Valid(PngStructure),
    RecoverableCrcErrors {
        structure: PngStructure,
        errors: Vec<PngCorruptionReason>,
    },
    CorruptedAt {
        offset: u64,
        reason: PngCorruptionReason,
        partial_structure: PngStructure,
    },
    Truncated {
        last_valid_offset: u64,
        partial_structure: PngStructure,
    },
    InvalidHeader,
}

impl PngValidationResult {
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Valid(_) | Self::RecoverableCrcErrors { .. })
    }

    pub fn structure(&self) -> Option<&PngStructure> {
        match self {
            Self::Valid(s) => Some(s),
            Self::RecoverableCrcErrors { structure, .. } => Some(structure),
            Self::CorruptedAt {
                partial_structure, ..
            } => Some(partial_structure),
            Self::Truncated {
                partial_structure, ..
            } => Some(partial_structure),
            Self::InvalidHeader => None,
        }
    }
}

pub struct PngValidator {
    parser: PngParser,
}

impl PngValidator {
    pub fn new() -> Self {
        Self {
            parser: PngParser::new(),
        }
    }

    pub fn validate(&self, data: &[u8]) -> PngValidationResult {
        let structure = match self.parser.parse(data) {
            Ok(s) => s,
            Err(_) => return PngValidationResult::InvalidHeader,
        };

        let has_ihdr = structure
            .chunks
            .iter()
            .any(|c| matches!(c.chunk_type, ChunkType::Ihdr));
        if !has_ihdr {
            return PngValidationResult::CorruptedAt {
                offset: 8,
                reason: PngCorruptionReason::MissingIhdr,
                partial_structure: structure,
            };
        }

        if let Some(ref ihdr) = structure.ihdr {
            if !ihdr.is_valid() {
                return PngValidationResult::CorruptedAt {
                    offset: 8,
                    reason: PngCorruptionReason::InvalidIhdr,
                    partial_structure: structure,
                };
            }
        }

        let has_iend = structure
            .chunks
            .iter()
            .any(|c| matches!(c.chunk_type, ChunkType::Iend));
        if !has_iend {
            return PngValidationResult::Truncated {
                last_valid_offset: structure.valid_end_offset,
                partial_structure: structure,
            };
        }

        if let Some(offset) = structure.corruption_point {
            return PngValidationResult::CorruptedAt {
                offset,
                reason: PngCorruptionReason::InvalidChunkLength { offset },
                partial_structure: structure,
            };
        }

        let crc_errors: Vec<PngCorruptionReason> = structure
            .chunks
            .iter()
            .filter(|c| !c.crc_valid)
            .map(|c| PngCorruptionReason::CrcMismatch {
                chunk_offset: c.offset,
                chunk_type: c.chunk_type_bytes,
            })
            .collect();

        if !crc_errors.is_empty() {
            return PngValidationResult::RecoverableCrcErrors {
                structure,
                errors: crc_errors,
            };
        }

        PngValidationResult::Valid(structure)
    }

    pub fn repair_crc_value(&self, data: &mut [u8], chunk: &PngChunk) -> bool {
        let crc_offset = chunk.crc_offset() as usize;
        if crc_offset + 4 > data.len() {
            return false;
        }
        let correct_crc = chunk.calculated_crc.to_be_bytes();
        data[crc_offset..crc_offset + 4].copy_from_slice(&correct_crc);
        true
    }
}

impl Default for PngValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_png() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&PNG_SIGNATURE);
        let ihdr_data = [
            0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        data.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
        data.extend_from_slice(&IHDR);
        data.extend_from_slice(&ihdr_data);
        data.extend_from_slice(&chunk_crc(&IHDR, &ihdr_data).to_be_bytes());
        let idat_data = [0x08, 0xD7, 0x63, 0x60, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01];
        data.extend_from_slice(&(idat_data.len() as u32).to_be_bytes());
        data.extend_from_slice(&IDAT);
        data.extend_from_slice(&idat_data);
        data.extend_from_slice(&chunk_crc(&IDAT, &idat_data).to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&IEND);
        data.extend_from_slice(&chunk_crc(&IEND, &[]).to_be_bytes());
        data
    }

    #[test]
    fn test_parse_minimal_png() {
        let data = create_valid_png();
        let structure = PngParser::new().parse(&data).unwrap();
        assert_eq!(structure.chunks.len(), 3);
        assert!(structure.ihdr.is_some());
        assert_eq!(structure.invalid_crc_count, 0);
    }

    #[test]
    fn test_validate_valid_png() {
        let data = create_valid_png();
        let result = PngValidator::new().validate(&data);
        assert!(matches!(result, PngValidationResult::Valid(_)));
    }

    #[test]
    fn test_validate_invalid_header() {
        let data = vec![0x00; 100];
        let result = PngValidator::new().validate(&data);
        assert!(matches!(result, PngValidationResult::InvalidHeader));
    }

    #[test]
    fn test_validate_truncated() {
        let mut data = create_valid_png();
        data.truncate(data.len() - 12);
        let result = PngValidator::new().validate(&data);
        assert!(matches!(result, PngValidationResult::Truncated { .. }));
    }

    #[test]
    fn test_crc32_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn test_chunk_type_roundtrip() {
        for ct in [
            ChunkType::Ihdr,
            ChunkType::Idat,
            ChunkType::Iend,
            ChunkType::Plte,
        ] {
            assert_eq!(ct, ChunkType::from_bytes(&ct.to_bytes()));
        }
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
        assert!(!IhdrData { width: 0, ..valid }.is_valid());
        assert!(!IhdrData {
            bit_depth: 5,
            ..valid
        }
        .is_valid());
    }
}
