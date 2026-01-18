use super::{is_sof_marker, is_standalone_marker};
use crate::error::{CoreError, Result};

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
            Self::Sof(b) => *b,
            Self::Other(b) => *b,
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

#[derive(Debug, Clone)]
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

impl Default for JpegStructure {
    fn default() -> Self {
        Self {
            markers: Vec::new(),
            sos_offset: None,
            image_width: 0,
            image_height: 0,
            is_progressive: false,
            restart_interval: 0,
            thumbnail: None,
            corruption_point: None,
            valid_end_offset: 0,
        }
    }
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

            if length < 2 {
                structure.corruption_point = Some(pos as u64);
                break;
            }

            if pos + 2 + length as usize > data.len() {
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
                    self.parse_sof(data, pos + 4, &mut structure);
                    if marker_byte == 0xC2 {
                        structure.is_progressive = true;
                    }
                }
                MarkerType::Sos => {
                    structure.sos_offset = Some(marker_offset);
                    pos += 2 + length as usize;
                    while pos < data.len() - 1 {
                        if data[pos] == 0xFF && data[pos + 1] != 0x00 {
                            let next_marker = data[pos + 1];
                            if next_marker == 0xD9 {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Eoi,
                                    offset: pos as u64,
                                    length: 0,
                                });
                                structure.valid_end_offset = pos as u64 + 2;
                                return Ok(structure);
                            } else if super::is_restart_marker(next_marker) {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Rst(next_marker - 0xD0),
                                    offset: pos as u64,
                                    length: 0,
                                });
                                pos += 2;
                            } else if next_marker == 0xFF {
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

    fn parse_sof(&self, data: &[u8], offset: usize, structure: &mut JpegStructure) {
        if offset + 5 > data.len() {
            return;
        }

        structure.image_height = u16::from_be_bytes([data[offset + 1], data[offset + 2]]);
        structure.image_width = u16::from_be_bytes([data[offset + 3], data[offset + 4]]);
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
        let search_start = offset + 6;

        for i in search_start..segment_end.saturating_sub(2) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_minimal_jpeg() -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
            0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
        ]
        .into_iter()
        .chain(vec![0x00; 64])
        .chain(vec![0x01])
        .chain(vec![
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x64, 0x00, 0xC8, 0x01, 0x01, 0x11, 0x00, 0xFF,
            0xC4, 0x00, 0x1F,
        ])
        .chain(vec![0x00; 28])
        .chain(vec![0x01])
        .chain(vec![
            0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x00, 0x00, 0xFF, 0xD9,
        ])
        .collect()
    }

    #[test]
    fn test_parse_minimal_jpeg() {
        let data = create_minimal_jpeg();
        let parser = JpegParser::new();
        let structure = parser.parse(&data).unwrap();

        assert_eq!(structure.image_width, 200);
        assert_eq!(structure.image_height, 100);
        assert!(structure.sos_offset.is_some());
        assert!(!structure.is_progressive);
        assert!(structure.corruption_point.is_none());
    }

    #[test]
    fn test_parse_invalid_data() {
        let parser = JpegParser::new();
        assert!(parser.parse(&[0xFF]).is_err());

        assert!(parser.parse(&[0x00, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn test_marker_type_roundtrip() {
        for byte in 0u8..=255 {
            let marker_type = MarkerType::from_byte(byte);
            assert_eq!(marker_type.to_byte(), byte);
        }
    }

    #[test]
    fn test_marker_total_size() {
        let standalone = JpegMarker {
            marker_type: MarkerType::Soi,
            offset: 0,
            length: 0,
        };
        assert_eq!(standalone.total_size(), 2);

        let with_payload = JpegMarker {
            marker_type: MarkerType::App(0),
            offset: 2,
            length: 16,
        };
        assert_eq!(with_payload.total_size(), 18);
    }

    #[test]
    fn test_truncated_jpeg_detection() {
        let mut data = create_minimal_jpeg();
        data.truncate(data.len() / 2);

        let parser = JpegParser::new();
        let structure = parser.parse(&data).unwrap();

        assert!(
            structure.corruption_point.is_some()
                || structure
                    .markers
                    .last()
                    .map(|m| !matches!(m.marker_type, MarkerType::Eoi))
                    .unwrap_or(true)
        );
    }
}
