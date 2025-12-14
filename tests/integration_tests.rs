//! Integration tests
//!
//! End-to-end tests that verify the complete recovery workflow.

use argos::domain::entities::FileType;
use argos::domain::repositories::BlockDeviceReader;
use argos::domain::services::{FileCarver, SignatureRegistry};
use argos::infrastructure::block_device::LinuxBlockDevice;
use argos::infrastructure::carvers::ImageCarver;
use rstest::*;
use std::fs;
use std::io::Write;
use tempfile::TempDir;

// ============================================================================
// Fixtures
// ============================================================================

/// Creates a temporary disk image with embedded image signatures
#[fixture]
fn disk_image_with_jpeg() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_disk.img");

    // Create a 64KB "disk" with some JPEG data embedded
    let mut data = vec![0u8; 65536];

    // Place a minimal JPEG signature at offset 4096
    let jpeg_start = 4096;
    data[jpeg_start] = 0xFF;
    data[jpeg_start + 1] = 0xD8;
    data[jpeg_start + 2] = 0xFF;
    data[jpeg_start + 3] = 0xE0;

    // JPEG end marker
    data[jpeg_start + 100] = 0xFF;
    data[jpeg_start + 101] = 0xD9;

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&data).unwrap();
    file.sync_all().unwrap();

    (dir, path)
}

/// Creates a test device with multiple image signatures
#[fixture]
fn disk_image_with_multiple_images() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("multi_image.img");

    let mut data = vec![0u8; 131072]; // 128KB

    // JPEG at offset 1024
    let jpeg_offset = 1024;
    data[jpeg_offset..jpeg_offset + 4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]);
    data[jpeg_offset + 200..jpeg_offset + 202].copy_from_slice(&[0xFF, 0xD9]);

    // PNG at offset 8192
    let png_offset = 8192;
    data[png_offset..png_offset + 8]
        .copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    // PNG IEND chunk at offset 8500
    data[8500..8508].copy_from_slice(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]);

    // GIF at offset 16384
    let gif_offset = 16384;
    data[gif_offset..gif_offset + 6].copy_from_slice(b"GIF89a");
    data[gif_offset + 100..gif_offset + 102].copy_from_slice(&[0x00, 0x3B]);

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&data).unwrap();
    file.sync_all().unwrap();

    (dir, path)
}

// ============================================================================
// Scan Integration Tests
// ============================================================================

#[rstest]
fn test_scan_finds_jpeg_signature(disk_image_with_jpeg: (TempDir, std::path::PathBuf)) {
    let (_dir, path) = disk_image_with_jpeg;
    let path_str = path.to_str().unwrap();

    let device = LinuxBlockDevice::open(path_str).unwrap();
    let registry = SignatureRegistry::default();

    // Scan chunk by chunk
    let mut found_jpeg = false;
    device
        .read_chunks(0, 4096, |_offset, chunk| {
            let matches = registry.find_matches(chunk);
            if matches.iter().any(|m| m.file_type() == FileType::Jpeg) {
                found_jpeg = true;
                return false; // Stop scanning
            }
            true // Continue scanning
        })
        .unwrap();

    assert!(found_jpeg, "Should find JPEG signature in disk image");
}

#[rstest]
fn test_scan_finds_multiple_signatures(
    disk_image_with_multiple_images: (TempDir, std::path::PathBuf),
) {
    let (_dir, path) = disk_image_with_multiple_images;
    let path_str = path.to_str().unwrap();

    let device = LinuxBlockDevice::open(path_str).unwrap();
    let registry = SignatureRegistry::default();

    let mut found_types = Vec::new();

    device
        .read_chunks(0, 1024, |_offset, chunk| {
            let matches = registry.find_matches(chunk);
            for m in matches {
                if !found_types.contains(&m.file_type()) {
                    found_types.push(m.file_type());
                }
            }
            true // Continue scanning
        })
        .unwrap();

    assert!(
        found_types.contains(&FileType::Jpeg),
        "Should find JPEG: {:?}",
        found_types
    );
    assert!(
        found_types.contains(&FileType::Png),
        "Should find PNG: {:?}",
        found_types
    );
    assert!(
        found_types.contains(&FileType::Gif),
        "Should find GIF: {:?}",
        found_types
    );
}

// ============================================================================
// Carving Integration Tests
// ============================================================================

#[rstest]
fn test_carve_jpeg_from_data() {
    let carver = ImageCarver::new();

    // Create data with JPEG signature
    let mut data = vec![0u8; 1024];
    data[0] = 0xFF;
    data[1] = 0xD8;
    data[2] = 0xFF;
    data[3] = 0xE0;

    // End marker at position 200
    data[200] = 0xFF;
    data[201] = 0xD9;

    // Determine size
    let size = carver.determine_file_size(&data, FileType::Jpeg);
    assert_eq!(size, Some(202)); // Position after end marker

    // Validate carved data
    let carved = &data[..size.unwrap() as usize];
    assert!(carver.validate(carved, FileType::Jpeg));
}
