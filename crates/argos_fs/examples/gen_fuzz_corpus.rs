//! Writes fixture-built seed inputs into the fuzz corpus directories.
//!
//! Run from the repository root: `cargo run -p argos_fs --example gen_fuzz_corpus`.

use std::fs;
use std::path::Path;

use argos_fs::fixture::{
    APFS_BLOCK, EXT4_BLOCK, FAT_CLUSTER, FilePlan, NTFS_CLUSTER, apfs_container, exfat_volume,
    ext4_dir_block, ext4_volume, fat32_volume, gpt_image, ntfs_indx, ntfs_volume, truncated,
    usn_journal, with_u16_le, with_u32_le, zero_filled,
};

fn main() -> std::io::Result<()> {
    let base = Path::new("crates/argos_fs/fuzz/corpus");
    let file = FilePlan::new("seed.jpg", 64 * EXT4_BLOCK, 2 * EXT4_BLOCK);

    let part = base.join("part_scan");
    fs::create_dir_all(&part)?;
    let gpt = gpt_image(256 * 1024, &[64..=511]);
    fs::write(part.join("gpt"), &gpt)?;
    fs::write(part.join("gpt-truncated"), truncated(&gpt, gpt.len() / 2))?;
    // An entry count claiming the u32 maximum.
    fs::write(
        part.join("gpt-overflow"),
        with_u32_le(&gpt, 512 + 80, u32::MAX),
    )?;

    let ntfs_dir = base.join("ntfs_record");
    fs::create_dir_all(&ntfs_dir)?;
    let ntfs_file = FilePlan::new("seed.jpg", 512 * 1024, 4096);
    let ntfs = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &ntfs_file);
    let record_at = 64 * NTFS_CLUSTER + 1024;
    fs::write(ntfs_dir.join("record"), &ntfs[record_at..record_at + 1024])?;
    fs::write(ntfs_dir.join("boot"), &ntfs[..512])?;
    fs::write(ntfs_dir.join("indx"), ntfs_indx(&[("seed.jpg", 42)]))?;
    fs::write(ntfs_dir.join("usn"), usn_journal(&[("seed.jpg", 42)]))?;

    let ext4_dir = base.join("ext4_parse");
    fs::create_dir_all(&ext4_dir)?;
    let ext4 = ext4_volume(256 * EXT4_BLOCK, &file);
    fs::write(ext4_dir.join("volume"), &ext4)?;
    fs::write(
        ext4_dir.join("dirblock"),
        ext4_dir_block(&[("seed.jpg", 12)]),
    )?;
    // An inode size field claiming the u16 maximum.
    fs::write(
        ext4_dir.join("overflow"),
        with_u16_le(&ext4, EXT4_BLOCK + 88, u16::MAX),
    )?;

    let fat_dir = base.join("fat_dir");
    fs::create_dir_all(&fat_dir)?;
    let fat_file = FilePlan::new("seed.jpg", 80 * FAT_CLUSTER, 4096);
    fs::write(
        fat_dir.join("fat32"),
        fat32_volume(96 * FAT_CLUSTER, &fat_file),
    )?;
    fs::write(
        fat_dir.join("exfat"),
        exfat_volume(96 * FAT_CLUSTER, &fat_file),
    )?;

    let apfs_dir = base.join("apfs_open");
    fs::create_dir_all(&apfs_dir)?;
    let apfs_file = FilePlan::new("seed.jpg", 32 * APFS_BLOCK, APFS_BLOCK);
    let apfs = apfs_container(64 * APFS_BLOCK, &apfs_file);
    fs::write(apfs_dir.join("container"), &apfs)?;
    fs::write(apfs_dir.join("truncated"), truncated(&apfs, apfs.len() / 3))?;

    let sweep_dir = base.join("residue_sweep");
    fs::create_dir_all(&sweep_dir)?;
    fs::write(sweep_dir.join("ext4"), &ext4)?;
    fs::write(sweep_dir.join("zeros"), zero_filled(64 * 1024))?;

    println!("seed corpus written under {}", base.display());
    Ok(())
}
