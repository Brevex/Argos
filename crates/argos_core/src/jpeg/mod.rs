mod huffman;
mod restart;

pub use huffman::HuffmanDecoder;
pub use restart::{RestartMarkerInfo, RestartMarkerScanner};

use crate::error::{CoreError, Result};

pub const SOI: [u8; 2] = [0xFF, 0xD8];
pub const EOI: [u8; 2] = [0xFF, 0xD9];
pub const SOS: u8 = 0xDA;
pub const DQT: u8 = 0xDB;
pub const DHT: u8 = 0xC4;
pub const SOF0: u8 = 0xC0;
pub const SOF1: u8 = 0xC1;
pub const SOF2: u8 = 0xC2;
pub const SOF3: u8 = 0xC3;
pub const DRI: u8 = 0xDD;
pub const RST0: u8 = 0xD0;
pub const RST1: u8 = 0xD1;
pub const RST2: u8 = 0xD2;
pub const RST3: u8 = 0xD3;
pub const RST4: u8 = 0xD4;
pub const RST5: u8 = 0xD5;
pub const RST6: u8 = 0xD6;
pub const RST7: u8 = 0xD7;
pub const APP0: u8 = 0xE0;
pub const APP1: u8 = 0xE1;
pub const APP2: u8 = 0xE2;
pub const COM: u8 = 0xFE;

#[inline]
pub const fn is_restart_marker(marker: u8) -> bool {
    marker >= RST0 && marker <= RST7
}
#[inline]
pub const fn is_sof_marker(marker: u8) -> bool {
    matches!(marker, SOF0 | SOF1 | SOF2 | SOF3 | 0xC5..=0xCF)
}
#[inline]
pub const fn is_app_marker(marker: u8) -> bool {
    marker >= 0xE0 && marker <= 0xEF
}
#[inline]
pub const fn is_standalone_marker(marker: u8) -> bool {
    matches!(marker, 0xD8 | 0xD9) || is_restart_marker(marker) || marker == 0x01
}
#[inline]
pub const fn restart_marker_index(marker: u8) -> Option<u8> {
    if is_restart_marker(marker) {
        Some(marker - RST0)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerType {
    Soi,
    Eoi,
    Sos,
    Dqt,
    Dht,
    Sof(u8),
    Dri,
    App(u8),
    Com,
    Rst(u8),
    Other(u8),
}

impl MarkerType {
    #[inline]
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0xD8 => Self::Soi,
            0xD9 => Self::Eoi,
            0xDA => Self::Sos,
            0xDB => Self::Dqt,
            0xC4 => Self::Dht,
            0xDD => Self::Dri,
            0xFE => Self::Com,
            b if b >= 0xD0 && b <= 0xD7 => Self::Rst(b - 0xD0),
            b if b >= 0xE0 && b <= 0xEF => Self::App(b - 0xE0),
            b if is_sof_marker(b) => Self::Sof(b),
            b => Self::Other(b),
        }
    }

    #[inline]
    pub fn to_byte(&self) -> u8 {
        match self {
            Self::Soi => 0xD8,
            Self::Eoi => 0xD9,
            Self::Sos => 0xDA,
            Self::Dqt => 0xDB,
            Self::Dht => 0xC4,
            Self::Dri => 0xDD,
            Self::Com => 0xFE,
            Self::Rst(n) => 0xD0 + n,
            Self::App(n) => 0xE0 + n,
            Self::Sof(b) | Self::Other(b) => *b,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct JpegMarker {
    pub marker_type: MarkerType,
    pub offset: u64,
    pub length: u16,
}

impl JpegMarker {
    #[inline]
    pub const fn total_size(&self) -> u64 {
        if self.length == 0 {
            2
        } else {
            2 + self.length as u64
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThumbnailInfo {
    pub soi_offset: u64,
    pub eoi_offset: Option<u64>,
    pub width: Option<u16>,
    pub height: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct JpegStructure {
    pub markers: Vec<JpegMarker>,
    pub sos_offset: Option<u64>,
    pub image_width: u16,
    pub image_height: u16,
    pub is_progressive: bool,
    pub restart_interval: u16,
    pub thumbnail: Option<ThumbnailInfo>,
    pub corruption_point: Option<u64>,
    pub valid_end_offset: u64,
}

pub struct JpegParser;

impl JpegParser {
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    pub fn parse(&self, data: &[u8]) -> Result<JpegStructure> {
        if data.len() < 4 {
            return Err(CoreError::InvalidFormat("Data too short for JPEG".into()));
        }
        if data[0] != 0xFF || data[1] != 0xD8 {
            return Err(CoreError::InvalidFormat("Missing JPEG SOI marker".into()));
        }

        let mut structure = JpegStructure::default();
        structure.markers.push(JpegMarker {
            marker_type: MarkerType::Soi,
            offset: 0,
            length: 0,
        });
        let mut pos: usize = 2;

        while pos < data.len() - 1 {
            if data[pos] != 0xFF {
                pos += 1;
                continue;
            }
            while pos < data.len() - 1 && data[pos + 1] == 0xFF {
                pos += 1;
            }
            if pos >= data.len() - 1 {
                break;
            }

            let marker_byte = data[pos + 1];
            if marker_byte == 0x00 {
                pos += 2;
                continue;
            }

            let marker_type = MarkerType::from_byte(marker_byte);
            let marker_offset = pos as u64;

            if is_standalone_marker(marker_byte) {
                structure.markers.push(JpegMarker {
                    marker_type,
                    offset: marker_offset,
                    length: 0,
                });
                if matches!(marker_type, MarkerType::Eoi) {
                    structure.valid_end_offset = pos as u64 + 2;
                    break;
                }
                pos += 2;
                continue;
            }

            if pos + 3 >= data.len() {
                structure.corruption_point = Some(pos as u64);
                break;
            }
            let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);
            if length < 2 || pos + 2 + length as usize > data.len() {
                structure.corruption_point = Some(pos as u64);
                break;
            }

            structure.markers.push(JpegMarker {
                marker_type,
                offset: marker_offset,
                length,
            });

            match marker_type {
                MarkerType::Sof(_) => {
                    if pos + 9 <= data.len() {
                        structure.image_height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                        structure.image_width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                    }
                    if marker_byte == 0xC2 {
                        structure.is_progressive = true;
                    }
                }
                MarkerType::Sos => {
                    structure.sos_offset = Some(marker_offset);
                    pos += 2 + length as usize;
                    while pos < data.len() - 1 {
                        if data[pos] == 0xFF && data[pos + 1] != 0x00 {
                            let next = data[pos + 1];
                            if next == 0xD9 {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Eoi,
                                    offset: pos as u64,
                                    length: 0,
                                });
                                structure.valid_end_offset = pos as u64 + 2;
                                return Ok(structure);
                            } else if is_restart_marker(next) {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Rst(next - 0xD0),
                                    offset: pos as u64,
                                    length: 0,
                                });
                                pos += 2;
                            } else if next == 0xFF {
                                pos += 1;
                            } else {
                                break;
                            }
                        } else {
                            pos += 1;
                        }
                    }
                    continue;
                }
                MarkerType::Dri => {
                    if length >= 4 && pos + 5 < data.len() {
                        structure.restart_interval =
                            u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
                    }
                }
                MarkerType::App(1) => {
                    self.parse_app1_exif(data, pos + 4, length - 2, &mut structure);
                }
                _ => {}
            }
            pos += 2 + length as usize;
        }

        if structure.valid_end_offset == 0 {
            structure.valid_end_offset = pos as u64;
        }
        Ok(structure)
    }

    fn parse_app1_exif(
        &self,
        data: &[u8],
        offset: usize,
        length: u16,
        structure: &mut JpegStructure,
    ) {
        if length < 14 || offset + 14 > data.len() {
            return;
        }
        if &data[offset..offset + 6] != b"Exif\0\0" {
            return;
        }
        let segment_end = offset + length as usize;
        for i in (offset + 6)..segment_end.saturating_sub(2) {
            if data[i] == 0xFF && data[i + 1] == 0xD8 {
                let mut thumbnail = ThumbnailInfo {
                    soi_offset: i as u64,
                    eoi_offset: None,
                    width: None,
                    height: None,
                };
                for j in (i + 2)..segment_end.saturating_sub(1) {
                    if data[j] == 0xFF && data[j + 1] == 0xD9 {
                        thumbnail.eoi_offset = Some(j as u64);
                        break;
                    }
                }
                structure.thumbnail = Some(thumbnail);
                break;
            }
        }
    }
}

impl Default for JpegParser {
    fn default() -> Self {
        Self::new()
    }
}

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
            Self::Valid(s)
            | Self::CorruptedAt {
                partial_structure: s,
                ..
            }
            | Self::Truncated {
                partial_structure: s,
                ..
            } => Some(s),
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
    pub fn parse_structure(&self, data: &[u8]) -> Result<JpegStructure> {
        self.parser.parse(data)
    }

    fn validate_marker_sequence(
        &self,
        structure: &JpegStructure,
    ) -> Option<(u64, CorruptionReason)> {
        let (mut seen_soi, mut seen_dqt, mut seen_sof, mut seen_sos) = (false, false, false, false);
        for marker in &structure.markers {
            match marker.marker_type {
                MarkerType::Soi => {
                    if seen_soi && seen_sos {
                        return Some((marker.offset, CorruptionReason::InvalidMarkerSequence));
                    }
                    seen_soi = true;
                }
                MarkerType::Dqt => seen_dqt = true,
                MarkerType::Dht => {}
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
        let result = JpegValidator::new().validate(&create_valid_jpeg());
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_truncated() {
        let mut data = create_valid_jpeg();
        data.truncate(data.len() - 10);
        assert!(matches!(
            JpegValidator::new().validate(&data),
            ValidationResult::Truncated { .. }
        ));
    }

    #[test]
    fn test_validate_invalid_header() {
        assert!(matches!(
            JpegValidator::new().validate(&[0x00; 4]),
            ValidationResult::InvalidHeader
        ));
    }

    #[test]
    fn test_marker_type_roundtrip() {
        for byte in 0u8..=255 {
            assert_eq!(MarkerType::from_byte(byte).to_byte(), byte);
        }
    }

    #[test]
    fn test_restart_marker_detection() {
        assert!(is_restart_marker(RST0));
        assert!(is_restart_marker(RST7));
        assert!(!is_restart_marker(0xD8));
    }
}
