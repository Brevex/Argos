use super::chunk_crc;
use super::parser::{ChunkType, PngChunk, PngParser, PngStructure};

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
            let critical_crc_errors = structure
                .chunks
                .iter()
                .filter(|c| !c.crc_valid && c.chunk_type.is_critical())
                .count();

            if critical_crc_errors > 0 {
                return PngValidationResult::RecoverableCrcErrors {
                    structure,
                    errors: crc_errors,
                };
            } else {
                return PngValidationResult::RecoverableCrcErrors {
                    structure,
                    errors: crc_errors,
                };
            }
        }

        PngValidationResult::Valid(structure)
    }

    pub fn attempt_crc_repair(
        &self,
        data: &mut [u8],
        chunk: &PngChunk,
        max_corrupt_bytes: usize,
    ) -> Option<usize> {
        if max_corrupt_bytes == 0 {
            return None;
        }

        let data_start = chunk.data_offset() as usize;
        let data_end = data_start + chunk.length as usize;

        if data_end > data.len() {
            return None;
        }

        let chunk_data = &mut data[data_start..data_end];
        let expected_crc = chunk.stored_crc;

        for i in 0..chunk_data.len() {
            let original = chunk_data[i];
            for candidate in 0..=255u8 {
                if candidate == original {
                    continue;
                }
                chunk_data[i] = candidate;
                if chunk_crc(&chunk.chunk_type_bytes, chunk_data) == expected_crc {
                    return Some(i);
                }
            }
            chunk_data[i] = original;
        }

        if max_corrupt_bytes >= 2 && chunk_data.len() <= 64 {
            for i in 0..chunk_data.len() {
                for j in (i + 1)..chunk_data.len() {
                    let orig_i = chunk_data[i];
                    let orig_j = chunk_data[j];

                    for ci in 0..=255u8 {
                        chunk_data[i] = ci;
                        for cj in 0..=255u8 {
                            chunk_data[j] = cj;
                            if chunk_crc(&chunk.chunk_type_bytes, chunk_data) == expected_crc {
                                return Some(i);
                            }
                        }
                    }

                    chunk_data[i] = orig_i;
                    chunk_data[j] = orig_j;
                }
            }
        }

        None
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
    use super::super::{chunk_crc, IDAT, IEND, IHDR, PNG_SIGNATURE};
    use super::*;

    fn create_valid_png() -> Vec<u8> {
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
    fn test_validate_valid_png() {
        let data = create_valid_png();
        let validator = PngValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(result, PngValidationResult::Valid(_)));
    }

    #[test]
    fn test_validate_invalid_header() {
        let data = vec![0x00; 100];
        let validator = PngValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(result, PngValidationResult::InvalidHeader));
    }

    #[test]
    fn test_validate_truncated() {
        let mut data = create_valid_png();
        data.truncate(data.len() - 12);

        let validator = PngValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(result, PngValidationResult::Truncated { .. }));
    }

    #[test]
    fn test_validate_crc_error() {
        let mut data = create_valid_png();
        data[16] ^= 0xFF;

        let validator = PngValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(
            result,
            PngValidationResult::RecoverableCrcErrors { .. }
        ));
    }

    #[test]
    fn test_result_is_usable() {
        let valid = PngValidationResult::Valid(PngStructure::default());
        assert!(valid.is_usable());

        let recoverable = PngValidationResult::RecoverableCrcErrors {
            structure: PngStructure::default(),
            errors: vec![],
        };
        assert!(recoverable.is_usable());

        let invalid = PngValidationResult::InvalidHeader;
        assert!(!invalid.is_usable());
    }

    #[test]
    fn test_repair_crc_value() {
        let mut data = create_valid_png();
        let validator = PngValidator::new();

        let structure = validator.parser.parse(&data).unwrap();
        let ihdr_chunk = structure
            .chunks
            .iter()
            .find(|c| matches!(c.chunk_type, ChunkType::Ihdr))
            .unwrap();

        let crc_offset = ihdr_chunk.crc_offset() as usize;
        data[crc_offset] ^= 0xFF;

        let corrupted = validator.parser.parse(&data).unwrap();
        assert!(corrupted.invalid_crc_count > 0);

        let ihdr_chunk_corrupted = corrupted
            .chunks
            .iter()
            .find(|c| matches!(c.chunk_type, ChunkType::Ihdr))
            .unwrap();
        assert!(validator.repair_crc_value(&mut data, ihdr_chunk_corrupted));

        let repaired = validator.parser.parse(&data).unwrap();
        assert_eq!(repaired.invalid_crc_count, 0);
    }
}
