mod parser;
mod validator;

pub use parser::{ChunkType, IhdrData, PngChunk, PngParser, PngStructure};
pub use validator::{PngCorruptionReason, PngValidationResult, PngValidator};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_signature() {
        assert_eq!(
            PNG_SIGNATURE,
            [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn test_chunk_type_constants() {
        assert_eq!(&IHDR, b"IHDR");
        assert_eq!(&IDAT, b"IDAT");
        assert_eq!(&IEND, b"IEND");
        assert_eq!(&PLTE, b"PLTE");
    }

    #[test]
    fn test_crc32_empty() {
        assert_eq!(crc32(&[]), 0x00000000);
    }

    #[test]
    fn test_crc32_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn test_chunk_crc() {
        let ihdr_data = [
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        let crc = chunk_crc(&IHDR, &ihdr_data);
        assert_ne!(crc, 0);
    }

    #[test]
    fn test_is_critical_chunk() {
        assert!(is_critical_chunk(b"IHDR"));
        assert!(is_critical_chunk(b"IDAT"));
        assert!(is_critical_chunk(b"IEND"));
        assert!(is_critical_chunk(b"PLTE"));
        assert!(!is_critical_chunk(b"tEXt"));
        assert!(!is_critical_chunk(b"gAMA"));
    }

    #[test]
    fn test_is_ancillary_chunk() {
        assert!(!is_ancillary_chunk(b"IHDR"));
        assert!(is_ancillary_chunk(b"tEXt"));
        assert!(is_ancillary_chunk(b"gAMA"));
        assert!(is_ancillary_chunk(b"sRGB"));
    }

    #[test]
    fn test_is_safe_to_copy() {
        assert!(is_safe_to_copy(b"tEXt"));
        assert!(!is_safe_to_copy(b"cHRM"));
    }
}
