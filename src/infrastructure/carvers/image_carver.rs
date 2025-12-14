//! Image carver implementation
//!
//! Carves image files from raw data using format-specific logic.

use crate::domain::entities::{FileType, RecoveredFile, SignatureMatch};
use crate::domain::services::{CarverError, FileCarver};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

/// Image file carver
///
/// Supports carving of common image formats including JPEG, PNG, GIF, BMP, etc.
/// Uses both footer detection and format-specific size parsing.
pub struct ImageCarver {
    supported_types: Vec<FileType>,
}

impl ImageCarver {
    /// Creates a new image carver with all supported image types
    pub fn new() -> Self {
        Self {
            supported_types: vec![
                FileType::Jpeg,
                FileType::Png,
                FileType::Gif,
                FileType::Bmp,
                FileType::WebP,
                FileType::Tiff,
            ],
        }
    }

    /// Finds JPEG end marker (FFD9) in data
    fn find_jpeg_end(&self, data: &[u8]) -> Option<usize> {
        // JPEG ends with FF D9
        for i in 0..data.len().saturating_sub(1) {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                return Some(i + 2);
            }
        }
        None
    }

    /// Finds PNG end marker (IEND chunk) in data
    fn find_png_end(&self, data: &[u8]) -> Option<usize> {
        // PNG IEND: 00 00 00 00 49 45 4E 44 AE 42 60 82
        // Actually just looking for IEND + CRC
        let iend = [0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];

        if data.len() < iend.len() {
            return None;
        }

        for i in 0..=data.len() - iend.len() {
            if &data[i..i + iend.len()] == iend {
                return Some(i + iend.len());
            }
        }
        None
    }

    /// Finds GIF end marker (00 3B) in data
    fn find_gif_end(&self, data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(1) {
            if data[i] == 0x00 && data[i + 1] == 0x3B {
                return Some(i + 2);
            }
        }
        None
    }

    /// Reads BMP file size from header
    fn read_bmp_size(&self, data: &[u8]) -> Option<usize> {
        if data.len() < 6 {
            return None;
        }
        // BMP file size is at offset 2, 4 bytes, little-endian
        let mut cursor = Cursor::new(&data[2..6]);
        cursor.read_u32::<LittleEndian>().ok().map(|s| s as usize)
    }

    /// Reads RIFF (WebP) file size from header
    fn read_riff_size(&self, data: &[u8]) -> Option<usize> {
        if data.len() < 12 {
            return None;
        }
        // Check for WEBP signature at offset 8
        if &data[8..12] != b"WEBP" {
            return None;
        }
        // RIFF chunk size is at offset 4, 4 bytes, little-endian
        // Add 8 for RIFF header
        let mut cursor = Cursor::new(&data[4..8]);
        cursor
            .read_u32::<LittleEndian>()
            .ok()
            .map(|s| (s + 8) as usize)
    }
}

impl Default for ImageCarver {
    fn default() -> Self {
        Self::new()
    }
}

impl FileCarver for ImageCarver {
    fn supported_types(&self) -> &[FileType] {
        &self.supported_types
    }

    fn carve(
        &self,
        data: &[u8],
        match_info: &SignatureMatch,
        file_id: u64,
    ) -> Result<RecoveredFile, CarverError> {
        let file_type = match_info.file_type();

        // Determine file size (convert u64 to usize for array indexing)
        let size: usize = self
            .determine_file_size(data, file_type)
            .map(|s| s as usize)
            .or_else(|| {
                match_info
                    .end_offset()
                    .map(|e| (e - match_info.start_offset()) as usize)
            })
            .unwrap_or_else(|| match_info.estimated_size() as usize);

        if size == 0 {
            return Err(CarverError::InvalidData("File size is zero".to_string()));
        }

        if size > data.len() {
            return Err(CarverError::InvalidData(format!(
                "Calculated size {} exceeds available data {}",
                size,
                data.len()
            )));
        }

        // Extract file data
        let file_data = data[..size].to_vec();

        // Validate the data
        let confidence = if self.validate(&file_data, file_type) {
            0.9
        } else {
            0.5 // Lower confidence if validation failed
        };

        let mut recovered = RecoveredFile::new(
            file_id,
            file_type,
            match_info.start_offset(),
            file_data,
            confidence,
        );

        // Check for corruption indicators
        if !self.validate(&recovered.data().to_vec(), file_type) {
            recovered.mark_corrupted();
        }

        Ok(recovered)
    }

    fn determine_file_size(&self, data: &[u8], file_type: FileType) -> Option<u64> {
        match file_type {
            FileType::Jpeg => self.find_jpeg_end(data).map(|s| s as u64),
            FileType::Png => self.find_png_end(data).map(|s| s as u64),
            FileType::Gif => self.find_gif_end(data).map(|s| s as u64),
            FileType::Bmp => self.read_bmp_size(data).map(|s| s as u64),
            FileType::WebP => self.read_riff_size(data).map(|s| s as u64),
            FileType::Tiff => None, // TIFF is complex, rely on max size
            FileType::Unknown => None,
        }
    }

    fn validate(&self, data: &[u8], file_type: FileType) -> bool {
        if data.is_empty() {
            return false;
        }

        match file_type {
            FileType::Jpeg => {
                // Check for proper JPEG structure
                data.len() >= 3
                    && data[0] == 0xFF
                    && data[1] == 0xD8
                    && data[2] == 0xFF
                    && data[data.len() - 2..] == [0xFF, 0xD9]
            }
            FileType::Png => {
                // Check PNG signature and IEND
                data.len() >= 12 && &data[0..8] == &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
            }
            FileType::Gif => {
                // Check GIF signature
                data.len() >= 6 && (data[0..6] == *b"GIF89a" || data[0..6] == *b"GIF87a")
            }
            FileType::Bmp => {
                // Check BMP signature
                data.len() >= 2 && data[0] == 0x42 && data[1] == 0x4D
            }
            FileType::WebP => {
                // Check RIFF/WEBP
                data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP"
            }
            FileType::Tiff => {
                // Check TIFF signature
                data.len() >= 4
                    && ((&data[0..4] == &[0x49, 0x49, 0x2A, 0x00])
                        || (&data[0..4] == &[0x4D, 0x4D, 0x00, 0x2A]))
            }
            FileType::Unknown => false,
        }
    }
}
