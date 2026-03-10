#![allow(dead_code)]

use std::fs;
use tempfile::TempDir;

use argos::io::DiskReader;

pub fn high_entropy_data(size: usize, seed: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    let s = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    for i in 0..size {
        v.push(
            (((i as u64)
                .wrapping_mul(131)
                .wrapping_add(s)
                .wrapping_add(17))
                % 251) as u8,
        );
    }
    v
}

pub fn create_test_jpeg(seed: u8) -> Vec<u8> {
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
    jpeg.extend_from_slice(&[0x02, 0x00]);
    jpeg.extend_from_slice(&[0x02, 0x80]);
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

pub fn create_test_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::new();

    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr_payload = Vec::new();

    ihdr_payload.extend_from_slice(&width.to_be_bytes());
    ihdr_payload.extend_from_slice(&height.to_be_bytes());
    ihdr_payload.push(8);
    ihdr_payload.push(2);
    ihdr_payload.extend_from_slice(&[0, 0, 0]);

    png.extend_from_slice(&make_png_chunk(b"IHDR", &ihdr_payload));

    let idat_data: Vec<u8> = (0..200).map(|i| ((i * 37 + 13) % 251) as u8).collect();

    png.extend_from_slice(&make_png_chunk(b"IDAT", &idat_data));
    png.extend_from_slice(&make_png_chunk(b"IEND", &[]));

    png
}

pub fn make_png_chunk(chunk_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(payload);

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(chunk_type);
    hasher.update(payload);

    let crc = hasher.finalize();
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

pub fn disk_from_bytes(data: &[u8]) -> (TempDir, DiskReader) {
    let dir = tempfile::tempdir().unwrap();
    let disk_path = dir.path().join("test_disk.img");
    fs::write(&disk_path, data).unwrap();
    let reader = DiskReader::open_regular(&disk_path).unwrap();
    (dir, reader)
}

pub fn create_test_disk(size: usize, embeddings: &[(usize, &[u8])]) -> Vec<u8> {
    let mut disk = Vec::with_capacity(size);
    for i in 0..size {
        disk.push(((i.wrapping_mul(97).wrapping_add(13)) % 256) as u8);
    }
    for &(offset, data) in embeddings {
        if offset + data.len() <= disk.len() {
            disk[offset..offset + data.len()].copy_from_slice(data);
        }
    }
    disk
}

pub fn make_fat32_boot_sector(
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    fat_size_sectors: u32,
    total_sectors: u32,
    root_cluster: u32,
) -> Vec<u8> {
    let mut sector = vec![0u8; 512];

    sector[0] = 0xEB;
    sector[1] = 0x58;
    sector[2] = 0x90;
    sector[3..11].copy_from_slice(b"MSDOS5.0");
    sector[0x0B..0x0D].copy_from_slice(&bytes_per_sector.to_le_bytes());
    sector[0x0D] = sectors_per_cluster;
    sector[0x0E..0x10].copy_from_slice(&reserved_sectors.to_le_bytes());
    sector[0x10] = num_fats;
    sector[0x20..0x24].copy_from_slice(&total_sectors.to_le_bytes());
    sector[0x24..0x28].copy_from_slice(&fat_size_sectors.to_le_bytes());
    sector[0x2C..0x30].copy_from_slice(&root_cluster.to_le_bytes());
    sector[82..87].copy_from_slice(b"FAT32");
    sector[510] = 0x55;
    sector[511] = 0xAA;

    sector
}

pub fn make_ntfs_boot_sector(
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    mft_cluster: u64,
    record_size_raw: i8,
) -> Vec<u8> {
    let mut sector = vec![0u8; 512];

    sector[0] = 0xEB;
    sector[1] = 0x52;
    sector[2] = 0x90;
    sector[3..11].copy_from_slice(b"NTFS    ");
    sector[0x0B..0x0D].copy_from_slice(&bytes_per_sector.to_le_bytes());
    sector[0x0D] = sectors_per_cluster;
    sector[0x30..0x38].copy_from_slice(&mft_cluster.to_le_bytes());
    sector[0x40] = record_size_raw as u8;

    sector
}
