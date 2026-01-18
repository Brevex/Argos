mod huffman;
mod parser;
mod restart;
mod validator;

pub use huffman::HuffmanDecoder;
pub use parser::{JpegMarker, JpegParser, JpegStructure, MarkerType};
pub use restart::{RestartMarkerInfo, RestartMarkerScanner};
pub use validator::{CorruptionReason, JpegValidator, ValidationResult};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_marker_detection() {
        assert!(is_restart_marker(RST0));
        assert!(is_restart_marker(RST7));
        assert!(is_restart_marker(0xD4));
        assert!(!is_restart_marker(0xD8));
        assert!(!is_restart_marker(0xDA));
    }

    #[test]
    fn test_sof_marker_detection() {
        assert!(is_sof_marker(SOF0));
        assert!(is_sof_marker(SOF2));
        assert!(is_sof_marker(0xC9));
        assert!(!is_sof_marker(0xDA));
        assert!(!is_sof_marker(0xDB));
    }

    #[test]
    fn test_app_marker_detection() {
        assert!(is_app_marker(APP0));
        assert!(is_app_marker(APP1));
        assert!(is_app_marker(0xEF));
        assert!(!is_app_marker(0xDF));
        assert!(!is_app_marker(0xF0));
    }

    #[test]
    fn test_standalone_marker_detection() {
        assert!(is_standalone_marker(0xD8));
        assert!(is_standalone_marker(0xD9));
        assert!(is_standalone_marker(RST0));
        assert!(is_standalone_marker(RST7));
        assert!(!is_standalone_marker(SOS));
        assert!(!is_standalone_marker(DQT));
    }

    #[test]
    fn test_restart_marker_index() {
        assert_eq!(restart_marker_index(RST0), Some(0));
        assert_eq!(restart_marker_index(RST7), Some(7));
        assert_eq!(restart_marker_index(0xD4), Some(4));
        assert_eq!(restart_marker_index(0xDA), None);
    }

    #[test]
    fn test_marker_constants() {
        assert_eq!(SOI, [0xFF, 0xD8]);
        assert_eq!(EOI, [0xFF, 0xD9]);
        assert_eq!(SOS, 0xDA);
        assert_eq!(DQT, 0xDB);
        assert_eq!(DHT, 0xC4);
        assert_eq!(SOF0, 0xC0);
        assert_eq!(SOF2, 0xC2);
    }
}
