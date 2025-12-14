//! File signature entity
//!
//! Represents the magic bytes (header and optional footer) that identify
//! a specific file type. This is the foundation of file carving.

use std::fmt;

/// Types of files that can be recovered
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    /// JPEG image format
    Jpeg,
    /// PNG image format
    Png,
    /// GIF image format
    Gif,
    /// BMP image format
    Bmp,
    /// WebP image format
    WebP,
    /// TIFF image format
    Tiff,
    /// Unknown or unsupported format
    Unknown,
}

impl FileType {
    /// Returns the typical file extension for this file type
    pub fn extension(&self) -> &'static str {
        match self {
            FileType::Jpeg => "jpg",
            FileType::Png => "png",
            FileType::Gif => "gif",
            FileType::Bmp => "bmp",
            FileType::WebP => "webp",
            FileType::Tiff => "tiff",
            FileType::Unknown => "bin",
        }
    }

    /// Returns a human-readable name for this file type
    pub fn name(&self) -> &'static str {
        match self {
            FileType::Jpeg => "JPEG Image",
            FileType::Png => "PNG Image",
            FileType::Gif => "GIF Image",
            FileType::Bmp => "BMP Image",
            FileType::WebP => "WebP Image",
            FileType::Tiff => "TIFF Image",
            FileType::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A file signature containing magic bytes for file identification
#[derive(Debug, Clone)]
pub struct FileSignature {
    /// The type of file this signature identifies
    file_type: FileType,
    /// The header bytes (magic bytes at the start of the file)
    header: Vec<u8>,
    /// Optional footer bytes (magic bytes at the end of the file)
    footer: Option<Vec<u8>>,
    /// Maximum expected file size in bytes (for carving limits)
    max_size: u64,
    /// Offset from the start where the header should be found (usually 0)
    header_offset: usize,
}

impl FileSignature {
    /// Creates a new file signature
    pub fn new(
        file_type: FileType,
        header: Vec<u8>,
        footer: Option<Vec<u8>>,
        max_size: u64,
    ) -> Self {
        Self {
            file_type,
            header,
            footer,
            max_size,
            header_offset: 0,
        }
    }

    /// Creates a new file signature with a custom header offset
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.header_offset = offset;
        self
    }

    /// Returns the file type this signature identifies
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the header bytes
    pub fn header(&self) -> &[u8] {
        &self.header
    }

    /// Returns the footer bytes if present
    pub fn footer(&self) -> Option<&[u8]> {
        self.footer.as_deref()
    }

    /// Returns the maximum expected file size
    pub fn max_size(&self) -> u64 {
        self.max_size
    }

    /// Returns the header offset
    pub fn header_offset(&self) -> usize {
        self.header_offset
    }

    /// Checks if the given data starts with this signature's header
    pub fn matches_header(&self, data: &[u8]) -> bool {
        if data.len() < self.header.len() + self.header_offset {
            return false;
        }
        let start = self.header_offset;
        let end = start + self.header.len();
        &data[start..end] == self.header.as_slice()
    }

    /// Finds the footer position in the given data, if the signature has a footer
    pub fn find_footer(&self, data: &[u8]) -> Option<usize> {
        self.footer.as_ref().and_then(|footer| {
            data.windows(footer.len())
                .position(|window| window == footer.as_slice())
                .map(|pos| pos + footer.len())
        })
    }
}

/// Represents a match found during scanning
#[derive(Debug, Clone)]
pub struct SignatureMatch {
    /// The type of file found
    file_type: FileType,
    /// Byte offset where the file starts
    start_offset: u64,
    /// Byte offset where the file ends (if footer was found)
    end_offset: Option<u64>,
    /// Estimated size in bytes
    estimated_size: u64,
    /// Confidence level (0.0 - 1.0)
    confidence: f32,
}

impl SignatureMatch {
    /// Creates a new signature match
    pub fn new(
        file_type: FileType,
        start_offset: u64,
        end_offset: Option<u64>,
        estimated_size: u64,
    ) -> Self {
        let confidence = if end_offset.is_some() { 0.9 } else { 0.6 };
        Self {
            file_type,
            start_offset,
            end_offset,
            estimated_size,
            confidence,
        }
    }

    /// Creates a match with a custom confidence level
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Returns the file type
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the start offset
    pub fn start_offset(&self) -> u64 {
        self.start_offset
    }

    /// Returns the end offset if known
    pub fn end_offset(&self) -> Option<u64> {
        self.end_offset
    }

    /// Returns the estimated size
    pub fn estimated_size(&self) -> u64 {
        self.estimated_size
    }

    /// Returns the confidence level
    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    /// Returns the actual size if end offset is known
    pub fn actual_size(&self) -> Option<u64> {
        self.end_offset.map(|end| end - self.start_offset)
    }
}
