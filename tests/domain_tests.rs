//! Domain layer unit tests
//!
//! Tests for entities, repositories traits, and domain services.

use argos::domain::entities::{FileType, RecoveredFile, ScanProgress, ScanResult, SignatureMatch};
use argos::domain::repositories::{DeviceInfo, FileSystemType};
use argos::domain::services::SignatureRegistry;
use rstest::*;
use std::time::Duration;

// ============================================================================
// FileType Tests
// ============================================================================

#[rstest]
#[case(FileType::Jpeg, "jpg")]
#[case(FileType::Png, "png")]
#[case(FileType::Gif, "gif")]
#[case(FileType::Bmp, "bmp")]
#[case(FileType::WebP, "webp")]
#[case(FileType::Tiff, "tiff")]
#[case(FileType::Unknown, "bin")]
fn test_file_type_extension(#[case] file_type: FileType, #[case] expected: &str) {
    assert_eq!(file_type.extension(), expected);
}

// ============================================================================
// RecoveredFile Tests
// ============================================================================

#[fixture]
fn sample_recovered_file() -> RecoveredFile {
    RecoveredFile::new(
        42,
        FileType::Jpeg,
        1000,
        vec![0xFF, 0xD8, 0xFF, 0xE0], // Sample JPEG data
        0.95,
    )
}

#[rstest]
fn test_recovered_file_creation(sample_recovered_file: RecoveredFile) {
    assert_eq!(sample_recovered_file.id(), 42);
    assert_eq!(sample_recovered_file.file_type(), FileType::Jpeg);
    assert_eq!(sample_recovered_file.size(), 4);
    assert!((sample_recovered_file.confidence() - 0.95).abs() < 0.001);
}

#[rstest]
fn test_suggested_filename(sample_recovered_file: RecoveredFile) {
    let filename = sample_recovered_file.suggested_filename();
    assert!(filename.contains("recovered"));
    assert!(filename.ends_with(".jpg"));
}

#[rstest]
#[case(0, "0 bytes")]
#[case(1023, "1023 bytes")]
#[case(1024, "1.00 KB")]
#[case(1048576, "1.00 MB")]
fn test_size_human(#[case] size: usize, #[case] expected: &str) {
    let file = RecoveredFile::new(1, FileType::Bmp, 0, vec![0; size], 1.0);
    assert_eq!(file.size_human(), expected);
}

// ============================================================================
// ScanProgress Tests
// ============================================================================

#[rstest]
fn test_scan_progress_percentage_zero() {
    let progress = ScanProgress::new(100);
    assert_eq!(progress.percentage(), 0.0);
}

#[rstest]
fn test_scan_progress_update() {
    let mut progress = ScanProgress::new(1000);
    progress.update(500, 5, 100);
    assert_eq!(progress.percentage(), 50.0);
}

#[rstest]
fn test_scan_progress_complete() {
    let mut progress = ScanProgress::new(1000);
    progress.update(1000, 10, 100);
    assert_eq!(progress.percentage(), 100.0);
}

// ============================================================================
// ScanResult Tests
// ============================================================================

#[rstest]
fn test_scan_result_creation() {
    let result = ScanResult::new("/dev/sda".to_string(), 1024, Duration::from_secs(10));
    assert_eq!(result.matches().len(), 0);
    assert_eq!(result.total_bytes(), 1024);
}

#[rstest]
fn test_scan_result_add_match() {
    let mut result = ScanResult::new("/dev/sda".to_string(), 1024 * 1024, Duration::from_secs(10));

    let match1 = SignatureMatch::new(FileType::Jpeg, 100, Some(1000), 900);
    let match2 = SignatureMatch::new(FileType::Jpeg, 2000, Some(3000), 1000);
    let match3 = SignatureMatch::new(FileType::Png, 5000, None, 2000);

    result.add_match(match1);
    result.add_match(match2);
    result.add_match(match3);

    assert_eq!(result.total_matches(), 3);
    assert_eq!(result.count_for_type(FileType::Jpeg), 2);
    assert_eq!(result.count_for_type(FileType::Png), 1);
}

// ============================================================================
// DeviceInfo Tests
// ============================================================================

#[rstest]
fn test_device_info_block_count() {
    let info = DeviceInfo {
        path: "/dev/sda".into(),
        size: 1024 * 1024 * 100, // 100 MB
        block_size: 512,
        read_only: true,
        model: Some("Test Drive".to_string()),
        serial: None,
    };

    assert_eq!(info.block_count(), 204800);
}

// ============================================================================
// FileSystemType Tests
// ============================================================================

#[rstest]
#[case(FileSystemType::Ext4, "ext4")]
#[case(FileSystemType::Btrfs, "Btrfs")]
#[case(FileSystemType::Ntfs, "NTFS")]
#[case(FileSystemType::Raw, "Raw")]
fn test_filesystem_type_name(#[case] fs_type: FileSystemType, #[case] expected: &str) {
    assert_eq!(fs_type.name(), expected);
}

#[rstest]
#[case(FileSystemType::Ext4, true)]
#[case(FileSystemType::Ntfs, true)]
#[case(FileSystemType::Raw, false)]
fn test_supports_deleted_entries(#[case] fs_type: FileSystemType, #[case] expected: bool) {
    assert_eq!(fs_type.supports_deleted_entries(), expected);
}

// ============================================================================
// SignatureRegistry Tests
// ============================================================================

#[fixture]
fn default_registry() -> SignatureRegistry {
    SignatureRegistry::default()
}

#[rstest]
fn test_find_jpeg_match(default_registry: SignatureRegistry) {
    let jpeg_data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    let matches = default_registry.find_matches(&jpeg_data);

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file_type(), FileType::Jpeg);
}

#[rstest]
fn test_find_png_match(default_registry: SignatureRegistry) {
    let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
    let matches = default_registry.find_matches(&png_data);

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file_type(), FileType::Png);
}

#[rstest]
fn test_no_match_for_unknown_data(default_registry: SignatureRegistry) {
    let unknown_data = [0x00, 0x01, 0x02, 0x03];
    let matches = default_registry.find_matches(&unknown_data);

    assert!(matches.is_empty());
}
