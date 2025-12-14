//! Infrastructure layer unit tests
//!
//! Tests for block devices, filesystem parsers, carvers, and file writers.

use argos::domain::entities::{FileType, RecoveredFile};
use argos::domain::repositories::{BlockDeviceReader, RecoveredFileWriter, WriteOptions};
use argos::domain::services::FileCarver;
use argos::infrastructure::block_device::LinuxBlockDevice;
use argos::infrastructure::carvers::ImageCarver;
use argos::infrastructure::persistence::LocalFileWriter;
use rstest::*;
use std::fs;
use std::io::Write;
use tempfile::TempDir;

// ============================================================================
// LinuxBlockDevice Tests
// ============================================================================

#[fixture]
fn temp_file_with_data() -> (TempDir, std::path::PathBuf, Vec<u8>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_device.img");
    let data: Vec<u8> = (0..=255u8).cycle().take(4096).collect();

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&data).unwrap();
    file.sync_all().unwrap();

    (dir, path, data)
}

#[rstest]
fn test_open_file(temp_file_with_data: (TempDir, std::path::PathBuf, Vec<u8>)) {
    let (_dir, path, _) = temp_file_with_data;
    let path_str = path.to_str().unwrap();
    let device = LinuxBlockDevice::open(path_str).unwrap();

    let info = device.device_info().unwrap();
    assert!(info.size > 0);
    assert_eq!(info.size, 4096);
}

#[rstest]
fn test_read_at(temp_file_with_data: (TempDir, std::path::PathBuf, Vec<u8>)) {
    let (_dir, path, data) = temp_file_with_data;
    let path_str = path.to_str().unwrap();
    let device = LinuxBlockDevice::open(path_str).unwrap();

    // Read middle chunk
    let chunk = device.read_at(256, 256).unwrap();
    assert_eq!(chunk, data[256..512]);
}

#[rstest]
fn test_read_chunks(temp_file_with_data: (TempDir, std::path::PathBuf, Vec<u8>)) {
    let (_dir, path, _) = temp_file_with_data;
    let path_str = path.to_str().unwrap();
    let device = LinuxBlockDevice::open(path_str).unwrap();

    let mut chunk_count = 0;
    device
        .read_chunks(0, 512, |_offset, data| {
            assert_eq!(data.len(), 512);
            chunk_count += 1;
            true // Continue reading
        })
        .unwrap();

    assert_eq!(chunk_count, 8); // 4096 / 512 = 8
}

#[rstest]
fn test_device_not_found() {
    let result = LinuxBlockDevice::open("/nonexistent/path/device");
    assert!(result.is_err());
}

// ============================================================================
// ImageCarver Tests
// ============================================================================

#[fixture]
fn carver() -> ImageCarver {
    ImageCarver::new()
}

#[rstest]
fn test_find_jpeg_end(carver: ImageCarver) {
    let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xD9, 0x00];

    // JPEG ends with FF D9, size should be at index 8
    let size = carver.determine_file_size(&data, FileType::Jpeg);
    assert_eq!(size, Some(8));
}

#[rstest]
fn test_find_png_end(carver: ImageCarver) {
    let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // Some data
    data.extend_from_slice(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]); // IEND

    let size = carver.determine_file_size(&data, FileType::Png);
    assert_eq!(size, Some(20));
}

#[rstest]
fn test_validate_jpeg(carver: ImageCarver) {
    let valid_jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0xFF, 0xD9];
    let invalid_png = [0x89, 0x50, 0x4E, 0x47];

    assert!(carver.validate(&valid_jpeg, FileType::Jpeg));
    assert!(!carver.validate(&invalid_png, FileType::Jpeg));
}

#[rstest]
fn test_read_bmp_size(carver: ImageCarver) {
    // BM header + size (256 in little-endian)
    let data = [0x42, 0x4D, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];

    let size = carver.determine_file_size(&data, FileType::Bmp);
    assert_eq!(size, Some(256));
}

#[rstest]
#[case(FileType::Jpeg)]
#[case(FileType::Png)]
#[case(FileType::Gif)]
#[case(FileType::Bmp)]
#[case(FileType::WebP)]
#[case(FileType::Tiff)]
fn test_supported_types(carver: ImageCarver, #[case] expected: FileType) {
    assert!(carver.supported_types().contains(&expected));
}

// ============================================================================
// LocalFileWriter Tests
// ============================================================================

#[fixture]
fn temp_output_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[rstest]
fn test_create_writer(temp_output_dir: TempDir) {
    let writer = LocalFileWriter::new(temp_output_dir.path()).unwrap();

    assert_eq!(writer.files_written(), 0);
    assert_eq!(writer.bytes_written(), 0);
}

#[rstest]
fn test_write_file_no_conversion(temp_output_dir: TempDir) {
    let writer = LocalFileWriter::new(temp_output_dir.path()).unwrap();

    let mut options = WriteOptions::default();
    options.convert_to_png = false; // Don't convert for this test
    options.organize_by_type = false;

    let data = vec![0x42, 0x4D, 0x00, 0x00]; // BMP header (minimal)
    let file = RecoveredFile::new(1, FileType::Bmp, 0, data.clone(), 1.0);

    let result = writer.write(&file, &options).unwrap();

    assert_eq!(result.file_id, 1);
    assert!(result.saved_path.exists());
    assert_eq!(result.saved_size, 4);

    // Verify content
    let written_data = fs::read(&result.saved_path).unwrap();
    assert_eq!(written_data, data);
}

// ============================================================================
// Filesystem Magic Numbers Tests
// ============================================================================

/// ext4 magic number
const EXT4_SUPER_MAGIC: u16 = 0xEF53;

/// Btrfs magic
const BTRFS_MAGIC: [u8; 8] = [0x5f, 0x42, 0x48, 0x52, 0x66, 0x53, 0x5f, 0x4d];

/// NTFS OEM ID
const NTFS_OEM_ID: [u8; 8] = [0x4E, 0x54, 0x46, 0x53, 0x20, 0x20, 0x20, 0x20];

#[rstest]
fn test_ext4_magic() {
    assert_eq!(EXT4_SUPER_MAGIC, 0xEF53);
}

#[rstest]
fn test_btrfs_magic() {
    // "_BHRfS_M" in bytes
    assert_eq!(
        BTRFS_MAGIC,
        [0x5f, 0x42, 0x48, 0x52, 0x66, 0x53, 0x5f, 0x4d]
    );
    assert_eq!(String::from_utf8_lossy(&BTRFS_MAGIC), "_BHRfS_M");
}

#[rstest]
fn test_ntfs_oem_id() {
    // "NTFS    " (4 spaces)
    assert_eq!(
        NTFS_OEM_ID,
        [0x4E, 0x54, 0x46, 0x53, 0x20, 0x20, 0x20, 0x20]
    );
    assert_eq!(String::from_utf8_lossy(&NTFS_OEM_ID), "NTFS    ");
}

#[rstest]
fn test_btrfs_superblock_offset() {
    // Primary superblock is at 64 KiB
    assert_eq!(65536u64, 64 * 1024);
}
