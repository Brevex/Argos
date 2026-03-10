use std::fs;
use tempfile::tempdir;

use argos::core::FragmentMap;
use argos::extraction::extract_all;
use argos::io::{DiskReader, DiskScanner};
use argos::recovery::linear_carve;
use argos::scan::scan_block;

mod helpers;

fn make_jpeg_image(width: u16, height: u16, seed: u8) -> Vec<u8> {
    let mut jpeg = Vec::new();

    jpeg.extend_from_slice(&[0xFF, 0xD8]);
    jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x10]);
    jpeg.extend_from_slice(b"Exif\x00\x00");
    jpeg.extend_from_slice(&[0x00; 8]);
    jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);

    for _ in 0..64 {
        jpeg.push(10);
    }

    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    jpeg.extend_from_slice(&height.to_be_bytes());
    jpeg.extend_from_slice(&width.to_be_bytes());
    jpeg.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
    jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x1F, 0x00]);

    for i in 0u8..28 {
        jpeg.push(i.wrapping_mul(37));
    }

    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);

    while jpeg.len() < 55_000 {
        let idx = jpeg.len();
        jpeg.push(((idx.wrapping_mul(131).wrapping_add(seed as usize * 7 + 17)) % 251) as u8);
    }

    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

#[allow(dead_code)]
fn run_pipeline(disk_data: &[u8]) -> (Vec<argos::core::RecoveredFile>, DiskReader) {
    let dir = tempdir().unwrap();
    let disk_path = dir.path().join("test.img");

    fs::write(&disk_path, disk_data).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((off, data)) = scanner.next_block().unwrap() {
        scan_block(off, data, &mut map);
    }

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();
    let recovered = linear_carve(&map, &reader, None);

    std::mem::forget(dir);
    (recovered, reader)
}

#[test]
fn test_extract_all_creates_tier_directories() {
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    let jpeg = make_jpeg_image(640, 480, 0);
    let mut disk = vec![0u8; 2 * 1024 * 1024];

    for i in 0..disk.len() {
        disk[i] = ((i.wrapping_mul(97).wrapping_add(13)) % 256) as u8;
    }

    let offset = 4096;
    disk[offset..offset + jpeg.len()].copy_from_slice(&jpeg);

    fs::write(&disk_path, &disk).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((off, data)) = scanner.next_block().unwrap() {
        scan_block(off, data, &mut map);
    }

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();
    let recovered = linear_carve(&map, &reader, None);
    let _report = extract_all(&recovered, &reader, &output_dir, None).unwrap();

    assert!(output_dir.join("high").exists());
    assert!(output_dir.join("partial").exists());
    assert!(output_dir.join("low").exists());
}

#[test]
fn test_extract_all_progress_callback() {
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    let jpeg = make_jpeg_image(640, 480, 1);
    let mut disk = vec![0u8; 2 * 1024 * 1024];

    for i in 0..disk.len() {
        disk[i] = ((i.wrapping_mul(97).wrapping_add(13)) % 256) as u8;
    }

    disk[4096..4096 + jpeg.len()].copy_from_slice(&jpeg);
    fs::write(&disk_path, &disk).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((off, data)) = scanner.next_block().unwrap() {
        scan_block(off, data, &mut map);
    }

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();
    let recovered = linear_carve(&map, &reader, None);

    let progress_called = std::sync::atomic::AtomicUsize::new(0);
    let _report = extract_all(
        &recovered,
        &reader,
        &output_dir,
        Some(&|_current, _total| {
            progress_called.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }),
    )
    .unwrap();

    assert!(
        progress_called.load(std::sync::atomic::Ordering::Relaxed) >= recovered.len(),
        "Progress callback should be called for each file"
    );
}

#[test]
fn test_extract_all_empty_list() {
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    fs::write(&disk_path, &[0u8; 4096]).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let report = extract_all(&[], &reader, &output_dir, None).unwrap();

    assert_eq!(report.extracted.len(), 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.dedup_skipped, 0);
}

#[test]
fn test_extract_all_dedup_keeps_higher_confidence() {
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    let jpeg = make_jpeg_image(640, 480, 42);
    let mut disk = vec![0u8; 4 * 1024 * 1024];

    for i in 0..disk.len() {
        disk[i] = ((i.wrapping_mul(97).wrapping_add(13)) % 256) as u8;
    }

    disk[4096..4096 + jpeg.len()].copy_from_slice(&jpeg);
    disk[2 * 1024 * 1024..2 * 1024 * 1024 + jpeg.len()].copy_from_slice(&jpeg);
    fs::write(&disk_path, &disk).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((off, data)) = scanner.next_block().unwrap() {
        scan_block(off, data, &mut map);
    }

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();
    let recovered = linear_carve(&map, &reader, None);
    let report = extract_all(&recovered, &reader, &output_dir, None).unwrap();

    assert_eq!(report.extracted.len(), 1, "Dedup should keep only 1");
    assert!(report.dedup_skipped >= 1, "Should skip duplicates");
}

#[test]
fn test_extract_all_output_files_valid_jpeg() {
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    let jpeg = make_jpeg_image(800, 600, 99);
    let mut disk = vec![0u8; 2 * 1024 * 1024];

    for i in 0..disk.len() {
        disk[i] = ((i.wrapping_mul(97).wrapping_add(13)) % 256) as u8;
    }

    disk[4096..4096 + jpeg.len()].copy_from_slice(&jpeg);
    fs::write(&disk_path, &disk).unwrap();

    let reader = DiskReader::open(&disk_path).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut map = FragmentMap::new();

    while let Some((off, data)) = scanner.next_block().unwrap() {
        scan_block(off, data, &mut map);
    }

    map.sort_by_offset();
    map.dedup();

    let reader = scanner.into_reader();
    let recovered = linear_carve(&map, &reader, None);
    let report = extract_all(&recovered, &reader, &output_dir, None).unwrap();

    for path in &report.extracted {
        let data = fs::read(path).unwrap();
        assert!(data.len() > 100, "Extracted file should have content");
        assert_eq!(&data[0..2], &[0xFF, 0xD8], "Must start with JPEG SOI");
        assert_eq!(
            &data[data.len() - 2..],
            &[0xFF, 0xD9],
            "Must end with JPEG EOI"
        );
    }
}
