//! Image file signatures
//!
//! Detailed signature definitions for various image formats.

use crate::domain::entities::{FileSignature, FileType};

/// Creates the JPEG signature
pub fn jpeg_signature() -> FileSignature {
    FileSignature::new(
        FileType::Jpeg,
        vec![0xFF, 0xD8, 0xFF],
        Some(vec![0xFF, 0xD9]),
        50 * 1024 * 1024, // 50MB max
    )
}

/// Creates the PNG signature
pub fn png_signature() -> FileSignature {
    FileSignature::new(
        FileType::Png,
        vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        Some(vec![0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]),
        100 * 1024 * 1024, // 100MB max
    )
}

/// Creates GIF signatures (both 87a and 89a)
pub fn gif_signatures() -> Vec<FileSignature> {
    vec![
        FileSignature::new(
            FileType::Gif,
            vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61], // GIF89a
            Some(vec![0x00, 0x3B]),
            50 * 1024 * 1024,
        ),
        FileSignature::new(
            FileType::Gif,
            vec![0x47, 0x49, 0x46, 0x38, 0x37, 0x61], // GIF87a
            Some(vec![0x00, 0x3B]),
            50 * 1024 * 1024,
        ),
    ]
}

/// Creates the BMP signature
pub fn bmp_signature() -> FileSignature {
    FileSignature::new(
        FileType::Bmp,
        vec![0x42, 0x4D], // "BM"
        None,             // Size is in the header
        100 * 1024 * 1024,
    )
}

/// Creates WebP signature
pub fn webp_signature() -> FileSignature {
    // RIFF....WEBP
    FileSignature::new(
        FileType::WebP,
        vec![0x52, 0x49, 0x46, 0x46], // "RIFF"
        None,
        100 * 1024 * 1024,
    )
}

/// Creates TIFF signatures (little-endian and big-endian)
pub fn tiff_signatures() -> Vec<FileSignature> {
    vec![
        FileSignature::new(
            FileType::Tiff,
            vec![0x49, 0x49, 0x2A, 0x00], // Little-endian "II*\0"
            None,
            500 * 1024 * 1024,
        ),
        FileSignature::new(
            FileType::Tiff,
            vec![0x4D, 0x4D, 0x00, 0x2A], // Big-endian "MM\0*"
            None,
            500 * 1024 * 1024,
        ),
    ]
}

/// Returns all image signatures
pub fn all_image_signatures() -> Vec<FileSignature> {
    let mut signatures = vec![
        jpeg_signature(),
        png_signature(),
        bmp_signature(),
        webp_signature(),
    ];
    signatures.extend(gif_signatures());
    signatures.extend(tiff_signatures());
    signatures
}
