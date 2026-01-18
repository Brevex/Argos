use super::parser::{JpegParser, JpegStructure, MarkerType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptionReason {
    InvalidMarkerSequence,
    HuffmanDecodeError,
    UnexpectedEof,
    InvalidSegmentLength,
    DcCoefficientDiscontinuity,
    MissingRequiredMarker(String),
    RestartSequenceError,
}

#[derive(Debug, Clone)]
pub enum ValidationResult {
    Valid(JpegStructure),
    CorruptedAt {
        offset: u64,
        reason: CorruptionReason,
        partial_structure: JpegStructure,
    },
    Truncated {
        last_valid_offset: u64,
        partial_structure: JpegStructure,
    },
    InvalidHeader,
}

impl ValidationResult {
    #[inline]
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    pub fn structure(&self) -> Option<&JpegStructure> {
        match self {
            Self::Valid(s) => Some(s),
            Self::CorruptedAt {
                partial_structure, ..
            } => Some(partial_structure),
            Self::Truncated {
                partial_structure, ..
            } => Some(partial_structure),
            Self::InvalidHeader => None,
        }
    }

    pub fn corruption_offset(&self) -> Option<u64> {
        match self {
            Self::Valid(_) => None,
            Self::CorruptedAt { offset, .. } => Some(*offset),
            Self::Truncated {
                last_valid_offset, ..
            } => Some(*last_valid_offset),
            Self::InvalidHeader => Some(0),
        }
    }
}

pub struct JpegValidator {
    parser: JpegParser,
}

impl JpegValidator {
    #[inline]
    pub fn new() -> Self {
        Self {
            parser: JpegParser::new(),
        }
    }

    pub fn validate(&self, data: &[u8]) -> ValidationResult {
        let structure = match self.parser.parse(data) {
            Ok(s) => s,
            Err(_) => return ValidationResult::InvalidHeader,
        };

        let has_eoi = structure
            .markers
            .iter()
            .any(|m| matches!(m.marker_type, MarkerType::Eoi));

        if !has_eoi {
            return ValidationResult::Truncated {
                last_valid_offset: structure.valid_end_offset,
                partial_structure: structure,
            };
        }

        if let Some(offset) = structure.corruption_point {
            return ValidationResult::CorruptedAt {
                offset,
                reason: CorruptionReason::InvalidSegmentLength,
                partial_structure: structure,
            };
        }

        if let Some((offset, reason)) = self.validate_marker_sequence(&structure) {
            return ValidationResult::CorruptedAt {
                offset,
                reason,
                partial_structure: structure,
            };
        }

        if structure.restart_interval > 0 {
            if let Some((offset, reason)) = self.validate_restart_sequence(&structure) {
                return ValidationResult::CorruptedAt {
                    offset,
                    reason,
                    partial_structure: structure,
                };
            }
        }

        ValidationResult::Valid(structure)
    }

    #[inline]
    pub fn parse_structure(&self, data: &[u8]) -> Result<JpegStructure, crate::error::CoreError> {
        self.parser.parse(data)
    }

    fn validate_marker_sequence(
        &self,
        structure: &JpegStructure,
    ) -> Option<(u64, CorruptionReason)> {
        let mut seen_soi = false;
        let mut seen_dqt = false;
        let mut seen_sof = false;
        let mut _seen_dht = false;
        let mut seen_sos = false;

        for marker in &structure.markers {
            match marker.marker_type {
                MarkerType::Soi => {
                    if seen_soi {
                        if seen_sos {
                            return Some((marker.offset, CorruptionReason::InvalidMarkerSequence));
                        }
                    }
                    seen_soi = true;
                }
                MarkerType::Dqt => seen_dqt = true,
                MarkerType::Dht => _seen_dht = true,
                MarkerType::Sof(_) => {
                    if !seen_dqt {
                        return Some((
                            marker.offset,
                            CorruptionReason::MissingRequiredMarker("DQT before SOF".into()),
                        ));
                    }
                    seen_sof = true;
                }
                MarkerType::Sos => {
                    if !seen_sof {
                        return Some((
                            marker.offset,
                            CorruptionReason::MissingRequiredMarker("SOF before SOS".into()),
                        ));
                    }
                    seen_sos = true;
                }
                MarkerType::Eoi => if !seen_sos {},
                _ => {}
            }
        }

        None
    }

    fn validate_restart_sequence(
        &self,
        structure: &JpegStructure,
    ) -> Option<(u64, CorruptionReason)> {
        let rst_markers: Vec<_> = structure
            .markers
            .iter()
            .filter(|m| matches!(m.marker_type, MarkerType::Rst(_)))
            .collect();

        if rst_markers.is_empty() {
            return None;
        }

        let mut expected = 0u8;
        for marker in rst_markers {
            if let MarkerType::Rst(n) = marker.marker_type {
                if n != expected {
                    return Some((marker.offset, CorruptionReason::RestartSequenceError));
                }
                expected = (expected + 1) % 8;
            }
        }

        None
    }

    pub fn validate_entropy_stream(&self, data: &[u8], sos_offset: u64) -> Option<u64> {
        let start = sos_offset as usize;
        if start >= data.len() {
            return Some(sos_offset);
        }

        if start + 4 > data.len() {
            return Some(sos_offset);
        }

        let sos_length = u16::from_be_bytes([data[start + 2], data[start + 3]]) as usize;
        let entropy_start = start + 2 + sos_length;

        if entropy_start >= data.len() {
            return Some(entropy_start as u64);
        }

        let mut pos = entropy_start;
        let mut consecutive_ff = 0;

        while pos < data.len() - 1 {
            if data[pos] == 0xFF {
                consecutive_ff += 1;

                if consecutive_ff > 4 {
                    return Some(pos as u64 - 3);
                }

                let next = data[pos + 1];
                if next == 0x00 {
                    consecutive_ff = 0;
                    pos += 2;
                } else if next == 0xD9 {
                    return None;
                } else if next >= 0xD0 && next <= 0xD7 {
                    consecutive_ff = 0;
                    pos += 2;
                } else if next == 0xFF {
                    pos += 1;
                } else {
                    return Some(pos as u64);
                }
            } else {
                consecutive_ff = 0;
                pos += 1;
            }
        }

        Some(pos as u64)
    }
}

impl Default for JpegValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_jpeg() -> Vec<u8> {
        vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00,
        ]
        .into_iter()
        .chain(vec![0x10; 64])
        .chain(vec![
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0x01, 0x11, 0x00, 0xFF,
            0xC4, 0x00, 0x1F, 0x00,
        ])
        .chain(vec![0x00; 28])
        .chain(vec![
            0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
        ])
        .chain(vec![0x00; 10])
        .chain(vec![0xFF, 0xD9])
        .collect()
    }

    #[test]
    fn test_validate_valid_jpeg() {
        let data = create_valid_jpeg();
        let validator = JpegValidator::new();
        let result = validator.validate(&data);

        assert!(result.is_valid(), "Expected valid JPEG: {:?}", result);
    }

    #[test]
    fn test_validate_truncated_jpeg() {
        let mut data = create_valid_jpeg();
        data.truncate(data.len() - 10);

        let validator = JpegValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(result, ValidationResult::Truncated { .. }));
    }

    #[test]
    fn test_validate_invalid_header() {
        let data = vec![0x00, 0x00, 0x00, 0x00];
        let validator = JpegValidator::new();
        let result = validator.validate(&data);

        assert!(matches!(result, ValidationResult::InvalidHeader));
    }

    #[test]
    fn test_corruption_offset() {
        let validator = JpegValidator::new();

        let valid = create_valid_jpeg();
        assert!(validator.validate(&valid).corruption_offset().is_none());

        let invalid = vec![0x00, 0x00, 0x00, 0x00];
        assert_eq!(validator.validate(&invalid).corruption_offset(), Some(0));
    }

    #[test]
    fn test_validation_result_structure() {
        let data = create_valid_jpeg();
        let validator = JpegValidator::new();
        let result = validator.validate(&data);

        let structure = result.structure().unwrap();
        assert!(structure.image_width > 0);
        assert!(structure.image_height > 0);
    }
}
