mod helpers;

use argos::fs::fat32;
use argos::io::AlignedBuffer;

use helpers::{disk_from_bytes, make_fat32_boot_sector};

fn make_fat32_disk_image(boot_sector: &[u8]) -> Vec<u8> {
    let mut disk = vec![0u8; 1024 * 1024];
    disk[..boot_sector.len()].copy_from_slice(boot_sector);
    disk
}

#[test]
fn test_detect_fat32_valid_boot_sector() {
    let bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = fat32::detect_fat32(&reader, 0, &mut buffer);
    assert!(info.is_some(), "Valid FAT32 should be detected");
    let info = info.unwrap();
    assert_eq!(info.bytes_per_sector, 512);
    assert_eq!(info.sectors_per_cluster, 8);
    assert_eq!(info.reserved_sectors, 32);
    assert_eq!(info.num_fats, 2);
    assert_eq!(info.fat_size_sectors, 1024);
    assert_eq!(info.root_cluster, 2);
    assert_eq!(info.cluster_size, 4096);
}

#[test]
fn test_detect_fat32_invalid_signature() {
    let mut bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    bs[510] = 0x00;
    bs[511] = 0x00;
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_fat32_invalid_fat32_string() {
    let mut bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    bs[82..87].copy_from_slice(b"FAT16");
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_fat32_zero_sectors_per_cluster() {
    let mut bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    bs[0x0D] = 0;
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_fat32_non_power_of_two_bps() {
    let mut bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    bs[0x0B..0x0D].copy_from_slice(&300u16.to_le_bytes());
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_fat32_zero_num_fats() {
    let mut bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    bs[0x10] = 0;
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_detect_fat32_invalid_root_cluster() {
    let bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 0);
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    assert!(fat32::detect_fat32(&reader, 0, &mut buffer).is_none());
}

#[test]
fn test_cluster_to_offset() {
    let bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = fat32::detect_fat32(&reader, 0, &mut buffer).unwrap();
    let offset = info.cluster_to_offset(2);
    assert_eq!(offset, info.data_start);
    let offset3 = info.cluster_to_offset(3);
    assert_eq!(offset3, info.data_start + info.cluster_size as u64);
}

#[test]
fn test_is_valid_cluster() {
    let bs = make_fat32_boot_sector(512, 8, 32, 2, 1024, 65536, 2);
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = fat32::detect_fat32(&reader, 0, &mut buffer).unwrap();
    assert!(!info.is_valid_cluster(0), "Cluster 0 is reserved");
    assert!(!info.is_valid_cluster(1), "Cluster 1 is reserved");
    assert!(
        info.is_valid_cluster(2),
        "Cluster 2 is the first valid data cluster"
    );
    assert!(
        !info.is_valid_cluster(0x0FFF_FFF8),
        "End-of-chain marker is not a valid cluster"
    );
}

#[test]
fn test_detect_fat32_4096_bps() {
    let bs = make_fat32_boot_sector(4096, 1, 32, 2, 256, 8192, 2);
    let disk = make_fat32_disk_image(&bs);
    let (_dir, reader) = disk_from_bytes(&disk);
    let mut buffer = AlignedBuffer::new();

    let info = fat32::detect_fat32(&reader, 0, &mut buffer);
    assert!(info.is_some(), "FAT32 with 4096 bps should be valid");
    let info = info.unwrap();
    assert_eq!(info.bytes_per_sector, 4096);
    assert_eq!(info.cluster_size, 4096);
}
