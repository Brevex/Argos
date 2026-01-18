pub mod carving;
pub mod io;
pub mod jpeg;
pub mod png;
pub mod scanners;
pub mod statistics;

pub use carving::{CarveDecision, SkipReason, SmartCarveResult, SmartCarver, SmartCarverConfig};
pub use io::{DiskReader, MmapReader, Reader};
pub use jpeg::{HuffmanDecoder, JpegParser, JpegValidator, RestartMarkerScanner, ValidationResult};
pub use png::{PngParser, PngValidationResult, PngValidator};
pub use scanners::{JpegScanner, PngScanner};
pub use statistics::{ImageClassification, ImageClassifier, ImageStatistics};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Offset {offset} is out of bounds (max: {max})")]
    OutOfBounds { offset: u64, max: u64 },

    #[error("Invalid buffer size: expected {expected}, got {actual}")]
    InvalidBufferSize { expected: usize, actual: usize },

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Jpeg,
    Png,
    Unknown,
}

impl FileType {
    #[must_use]
    #[inline]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Unknown => "bin",
        }
    }

    #[must_use]
    #[inline]
    pub const fn footer_size(self) -> u64 {
        match self {
            Self::Jpeg => 2,
            Self::Png => 8,
            Self::Unknown => 0,
        }
    }

    #[must_use]
    #[inline]
    pub const fn header_bytes(self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD8, 0xFF],
            Self::Png => &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            Self::Unknown => &[],
        }
    }

    #[must_use]
    #[inline]
    pub const fn footer_bytes(self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD9],
            Self::Png => &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82],
            Self::Unknown => &[],
        }
    }

    #[must_use]
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

pub trait BlockSource {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize>;
    fn size(&self) -> u64;
}

pub trait FileScanner: Send + Sync {
    fn scan_headers(&self, buffer: &[u8]) -> Vec<usize>;
    fn scan_footers(&self, buffer: &[u8]) -> Vec<usize>;
    fn file_type(&self) -> FileType;

    #[inline]
    fn scan_headers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.scan_headers(buffer) {
            callback(offset);
        }
    }

    #[inline]
    fn scan_footers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.scan_footers(buffer) {
            callback(offset);
        }
    }
}

pub fn get_image_dimensions(data: &[u8]) -> Option<(usize, usize)> {
    imagesize::blob_size(data)
        .ok()
        .map(|size| (size.width, size.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_type_extension() {
        assert_eq!(FileType::Jpeg.extension(), "jpg");
        assert_eq!(FileType::Png.extension(), "png");
        assert_eq!(FileType::Unknown.extension(), "bin");
    }

    #[test]
    fn test_file_type_footer_size() {
        assert_eq!(FileType::Jpeg.footer_size(), 2);
        assert_eq!(FileType::Png.footer_size(), 8);
        assert_eq!(FileType::Unknown.footer_size(), 0);
    }

    #[test]
    fn test_file_type_header_bytes() {
        assert_eq!(FileType::Jpeg.header_bytes(), &[0xFF, 0xD8, 0xFF]);
        assert_eq!(FileType::Png.header_bytes().len(), 8);
    }

    #[test]
    fn test_file_type_display() {
        assert_eq!(format!("{}", FileType::Jpeg), "JPEG");
        assert_eq!(format!("{}", FileType::Png), "PNG");
    }

    #[test]
    fn test_core_error_display() {
        let err = CoreError::InvalidFormat("test".into());
        assert!(err.to_string().contains("Invalid format"));
    }
}
