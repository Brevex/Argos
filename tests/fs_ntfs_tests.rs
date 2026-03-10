mod helpers;

use argos::fs::ntfs;
use argos::io::AlignedBuffer;

use helpers::{disk_from_bytes, make_ntfs_boot_sector};

fn make_ntfs_disk_image(boot_sector: &[u8]) -> Vec<u8> {
    let mut disk = vec![0u8; 2 * 1024 * 1024];
    disk[..boot_sector.len()].copy_from_slice(boot_sector);
    disk
}

#[test]
fn test_detect_ntfs_valid_boot_sector() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -10);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = ntfs::detect_ntfs(&reader, 0, &mut buffer);
    assert!(info.is_some(), "Valid NTFS should be detected");
    let info = info.unwrap();
    assert_eq!(info.bytes_per_sector, 512);
    assert_eq!(info.sectors_per_cluster, 8);
    assert_eq!(info.cluster_size, 4096);
    assert_eq!(info.mft_cluster, 2);
    assert_eq!(info.mft_record_size, 1024);
}

#[test]
fn test_detect_ntfs_invalid_oem() {
    let mut bs = make_ntfs_boot_sector(512, 8, 2, -10);
    bs[3..11].copy_from_slice(b"NotNTFS!");
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_ntfs_record_size_positive() {
    let bs = make_ntfs_boot_sector(512, 8, 2, 1);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = ntfs::detect_ntfs(&reader, 0, &mut buffer);
    assert!(info.is_some());
    assert_eq!(info.unwrap().mft_record_size, 4096);
}

#[test]
fn test_detect_ntfs_record_size_negative() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -9);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = ntfs::detect_ntfs(&reader, 0, &mut buffer);
    assert!(info.is_some());
    assert_eq!(info.unwrap().mft_record_size, 512);
}

#[test]
fn test_detect_ntfs_record_size_too_small() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -7);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_ntfs_record_size_too_large() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -17);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_ntfs_zero_bps() {
    let mut bs = make_ntfs_boot_sector(512, 8, 2, -10);
    bs[0x0B] = 0;
    bs[0x0C] = 0;
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_ntfs_non_power_of_two_bps() {
    let mut bs = make_ntfs_boot_sector(512, 8, 2, -10);
    bs[0x0B..0x0D].copy_from_slice(&300u16.to_le_bytes());
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_ntfs_zero_spc() {
    let mut bs = make_ntfs_boot_sector(512, 8, 2, -10);
    bs[0x0D] = 0; // Zero sectors per cluster
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(ntfs::detect_ntfs(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_ntfs_cluster_to_offset() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -10);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = ntfs::detect_ntfs(&reader, 0, &mut buffer).unwrap();
    assert_eq!(info.cluster_to_offset(0), 0);
    assert_eq!(info.cluster_to_offset(1), 4096);
    assert_eq!(info.cluster_to_offset(10), 40960);
}

#[test]
fn test_ntfs_cluster_to_offset_with_partition() {
    let bs = make_ntfs_boot_sector(512, 8, 2, -10);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let mut info = ntfs::detect_ntfs(&reader, 0, &mut buffer).unwrap();
    info.partition_offset = 1_048_576;
    assert_eq!(info.cluster_to_offset(0), 1_048_576);
    assert_eq!(info.cluster_to_offset(10), 1_048_576 + 40960);
}

#[test]
fn test_detect_ntfs_4096_bps() {
    let bs = make_ntfs_boot_sector(4096, 1, 2, -10);
    let disk = make_ntfs_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = ntfs::detect_ntfs(&reader, 0, &mut buffer);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.bytes_per_sector, 4096);
    assert_eq!(info.cluster_size, 4096);
}
