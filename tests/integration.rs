use std::fs;
use tempfile::tempdir;

use argos::analysis::scan_block;
use argos::carving::{linear_carve, RecoveryStats};
use argos::extraction::extract_all;
use argos::io::{DiskReader, DiskScanner};
use argos::types::{FragmentMap, ImageFormat};

fn create_test_jpeg() -> Vec<u8> {
    let mut jpeg = Vec::new();

    jpeg.extend_from_slice(&[0xFF, 0xD8]);

    jpeg.extend_from_slice(&[0xFF, 0xE0]);
    jpeg.extend_from_slice(&[0x00, 0x10]);
    jpeg.extend_from_slice(b"JFIF\x00");
    jpeg.extend_from_slice(&[0x01, 0x01]);
    jpeg.extend_from_slice(&[0x00]);
    jpeg.extend_from_slice(&[0x00, 0x48]);
    jpeg.extend_from_slice(&[0x00, 0x48]);
    jpeg.extend_from_slice(&[0x00, 0x00]);

    jpeg.extend_from_slice(&[0xFF, 0xDB]);
    jpeg.extend_from_slice(&[0x00, 0x43]);
    jpeg.extend_from_slice(&[0x00]);
    for _ in 0..64 {
        jpeg.push(16);
    }

    jpeg.extend_from_slice(&[0xFF, 0xC0]);
    jpeg.extend_from_slice(&[0x00, 0x0B]);
    jpeg.extend_from_slice(&[0x08]);
    jpeg.extend_from_slice(&[0x01, 0x00]);
    jpeg.extend_from_slice(&[0x01, 0x00]);
    jpeg.extend_from_slice(&[0x01]);
    jpeg.extend_from_slice(&[0x01, 0x11, 0x00]);

    jpeg.extend_from_slice(&[0xFF, 0xC4]);
    jpeg.extend_from_slice(&[0x00, 0x1F]);
    jpeg.extend_from_slice(&[0x00]);
    for i in 0u8..28 {
        jpeg.push(i);
    }

    jpeg.extend_from_slice(&[0xFF, 0xDA]);
    jpeg.extend_from_slice(&[0x00, 0x08]);
    jpeg.extend_from_slice(&[0x01]);
    jpeg.extend_from_slice(&[0x01, 0x00]);
    jpeg.extend_from_slice(&[0x00, 0x3F, 0x00]);

    while jpeg.len() < 2500 {
        jpeg.push(((jpeg.len() * 7) % 254) as u8);
    }

    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

fn create_test_disk(size_mb: usize, jpeg_offsets: &[usize]) -> Vec<u8> {
    let mut disk = vec![0u8; size_mb * 1024 * 1024];
    let jpeg = create_test_jpeg();

    for &offset in jpeg_offsets {
        if offset + jpeg.len() <= disk.len() {
            disk[offset..offset + jpeg.len()].copy_from_slice(&jpeg);
        }
    }

    disk
}

#[test]
fn test_full_recovery_pipeline() {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("test_disk.img");
    let output_dir = dir.path().join("recovered");

    let jpeg_offsets = vec![1024 * 1024, 3 * 1024 * 1024, 7 * 1024 * 1024];

    let disk_data = create_test_disk(10, &jpeg_offsets);
    fs::write(&disk_path, &disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((offset, data)) = scanner.next_block().unwrap() {
        scan_block(offset, data, &mut map);
    }

    assert!(map.len() > 0, "Should find fragments");

    map.sort_by_offset();
    let recovered = linear_carve(&map);

    let stats = RecoveryStats::from_recovered(&recovered);
    assert_eq!(stats.jpeg_linear, 3, "Should recover 3 JPEGs");

    let extracted = extract_all(&recovered, &disk_path, &output_dir, None).unwrap();
    assert_eq!(extracted.len(), 3, "Should extract 3 files");

    for path in &extracted {
        assert!(path.exists(), "File should exist: {:?}", path);
    }

    for path in &extracted {
        let data = fs::read(path).unwrap();
        assert!(data.len() > 2048, "File too small");
        assert_eq!(&data[0..2], &[0xFF, 0xD8], "Should start with SOI");
        assert_eq!(
            &data[data.len() - 2..],
            &[0xFF, 0xD9],
            "Should end with EOI"
        );
    }
}

#[test]
fn test_empty_disk_no_false_positives() {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("empty_disk.img");

    let disk_data = vec![0u8; 1024 * 1024];
    fs::write(&disk_path, &disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((offset, data)) = scanner.next_block().unwrap() {
        scan_block(offset, data, &mut map);
    }

    map.sort_by_offset();
    let recovered = linear_carve(&map);

    assert_eq!(
        recovered.len(),
        0,
        "Empty disk should produce no false positives"
    );
}

#[test]
fn test_disk_with_noise() {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("noisy_disk.img");

    let mut disk_data = Vec::with_capacity(2 * 1024 * 1024);
    for i in 0..2 * 1024 * 1024 {
        disk_data.push((i * 17 % 256) as u8);
    }

    let jpeg = create_test_jpeg();
    let offset = 1024 * 1024;
    disk_data[offset..offset + jpeg.len()].copy_from_slice(&jpeg);

    fs::write(&disk_path, &disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((offset, data)) = scanner.next_block().unwrap() {
        scan_block(offset, data, &mut map);
    }

    map.sort_by_offset();
    let recovered = linear_carve(&map);

    let stats = RecoveryStats::from_recovered(&recovered);
    assert!(stats.jpeg_linear >= 1, "Should recover at least 1 JPEG");
}

#[test]
fn test_multiple_formats() {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("multi_format.img");

    let jpeg = create_test_jpeg();
    let mut disk_data = vec![0u8; 2 * 1024 * 1024];
    disk_data[1024 * 1024..1024 * 1024 + jpeg.len()].copy_from_slice(&jpeg);

    fs::write(&disk_path, &disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((offset, data)) = scanner.next_block().unwrap() {
        scan_block(offset, data, &mut map);
    }

    map.sort_by_offset();
    let recovered = linear_carve(&map);

    for file in &recovered {
        match file.format {
            ImageFormat::Jpeg => {
                assert!(!file.fragments.is_empty());
            }
            ImageFormat::Png => {
                assert!(!file.fragments.is_empty());
            }
        }
    }
}

#[test]
fn test_scanner_handles_partial_blocks() {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("partial.img");

    let disk_data = vec![0xAAu8; 1024 * 1024 + 500];
    fs::write(&disk_path, &disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let mut total_read = 0u64;
    while let Some((_, data)) = scanner.next_block().unwrap() {
        total_read += data.len() as u64;
    }

    assert!(
        total_read >= disk_data.len() as u64,
        "Should read at least all bytes (with possible overlap)"
    );
}

#[test]
fn test_fragment_map_sorting() {
    let mut map = FragmentMap::new();

    map.push(argos::types::Fragment::new(
        1000,
        argos::types::FragmentKind::JpegHeader,
        7.5,
    ));
    map.push(argos::types::Fragment::new(
        500,
        argos::types::FragmentKind::JpegHeader,
        7.6,
    ));
    map.push(argos::types::Fragment::new(
        2000,
        argos::types::FragmentKind::JpegFooter,
        0.0,
    ));

    map.sort_by_offset();

    let offsets: Vec<u64> = map.fragments.iter().map(|f| f.offset).collect();
    assert_eq!(offsets, vec![500, 1000, 2000]);
}
