//! Core type definitions for file format identification.
//!
//! This module provides strongly-typed enums to replace magic strings
//! and ensure compile-time safety for file type handling.

/// Represents a recoverable file format detected by signature scanning.
///
/// Using an enum instead of strings ensures:
/// - Compile-time checking of all match arms
/// - No typos in format identification
/// - Zero-cost abstraction (enum variants are integers)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Jpeg,
    Png,
    Tiff,
    Bmp,
    Gif,
    WebP,
    Unknown,
}

impl FileType {
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Bmp => "bmp",
            Self::Gif => "gif",
            Self::WebP => "webp",
            Self::Unknown => "bin",
        }
    }

    /// Returns the size in bytes of the footer/trailer for this format.
    /// Some formats like JPEG and PNG have specific end-of-file markers.
    /// Returns 0 for formats without defined footers.
    #[must_use]
    pub const fn footer_size(&self) -> u64 {
        match self {
            Self::Jpeg => 2,
            Self::Png => 8,
            Self::Gif => 1,
            _ => 0,
        }
    }

    /// Returns the magic bytes (header signature) for this format.
    #[must_use]
    pub const fn header_bytes(&self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD8, 0xFF],
            Self::Png => &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            Self::Tiff => &[0x49, 0x49, 0x2A, 0x00],
            Self::Bmp => &[0x42, 0x4D],
            Self::Gif => &[0x47, 0x49, 0x46, 0x38],
            Self::WebP => &[0x52, 0x49, 0x46, 0x46],
            Self::Unknown => &[],
        }
    }

    /// Returns the footer/trailer bytes for this format, if any.
    #[must_use]
    pub const fn footer_bytes(&self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD9],
            Self::Png => &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82],
            Self::Gif => &[0x3B],
            _ => &[],
        }
    }

    /// Returns the human-readable name of this format.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Gif => "GIF",
            Self::WebP => "WebP",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension() {
        assert_eq!(FileType::Jpeg.extension(), "jpg");
        assert_eq!(FileType::Png.extension(), "png");
        assert_eq!(FileType::Unknown.extension(), "bin");
    }

    #[test]
    fn test_footer_size() {
        assert_eq!(FileType::Jpeg.footer_size(), 2);
        assert_eq!(FileType::Png.footer_size(), 8);
        assert_eq!(FileType::Bmp.footer_size(), 0);
    }

    #[test]
    fn test_header_bytes() {
        assert_eq!(FileType::Jpeg.header_bytes(), &[0xFF, 0xD8, 0xFF]);
        assert_eq!(FileType::Png.header_bytes().len(), 8);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", FileType::Jpeg), "JPEG");
        assert_eq!(format!("{}", FileType::Png), "PNG");
    }
}
