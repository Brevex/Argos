use argos::fs::ext4;

fn make_superblock(
    block_size_log: u32,
    inodes_count: u32,
    blocks_count: u32,
    blocks_per_group: u32,
    inodes_per_group: u32,
    inode_size: u16,
    feature_incompat: u32,
) -> Vec<u8> {
    let mut sb = vec![0u8; 1024];

    sb[0x00..0x04].copy_from_slice(&inodes_count.to_le_bytes());

    sb[0x04..0x08].copy_from_slice(&blocks_count.to_le_bytes());

    sb[0x18..0x1C].copy_from_slice(&block_size_log.to_le_bytes());

    sb[0x20..0x24].copy_from_slice(&blocks_per_group.to_le_bytes());

    sb[0x28..0x2C].copy_from_slice(&inodes_per_group.to_le_bytes());

    sb[0x38..0x3A].copy_from_slice(&0xEF53u16.to_le_bytes());

    sb[0x58..0x5A].copy_from_slice(&inode_size.to_le_bytes());

    sb[0x60..0x64].copy_from_slice(&feature_incompat.to_le_bytes());

    sb
}

fn make_extent_leaf(entries: &[(u32, u16, u16, u32)]) -> Vec<u8> {
    let count = entries.len() as u16;
    let max = count;
    let depth = 0u16;

    let mut data = vec![0u8; 12 + entries.len() * 12];

    data[0..2].copy_from_slice(&0xF30Au16.to_le_bytes());
    data[2..4].copy_from_slice(&count.to_le_bytes());
    data[4..6].copy_from_slice(&max.to_le_bytes());
    data[6..8].copy_from_slice(&depth.to_le_bytes());

    for (i, &(ee_block, ee_len, ee_start_hi, ee_start_lo)) in entries.iter().enumerate() {
        let off = 12 + i * 12;
        data[off..off + 4].copy_from_slice(&ee_block.to_le_bytes());
        data[off + 4..off + 6].copy_from_slice(&ee_len.to_le_bytes());
        data[off + 6..off + 8].copy_from_slice(&ee_start_hi.to_le_bytes());
        data[off + 8..off + 12].copy_from_slice(&ee_start_lo.to_le_bytes());
    }

    data
}

#[test]
fn test_superblock_valid_4k() {
    let sb = make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0040);
    let info = ext4::parse_superblock_bytes(&sb).expect("should parse");
    assert_eq!(info.block_size, 4096);
    assert_eq!(info.inodes_count, 1024);
    assert_eq!(info.inodes_per_group, 1024);
    assert_eq!(info.inode_size, 256);
    assert!(info.has_extents);
    assert!(!info.is_64bit);
    assert_eq!(info.desc_size, 32);
    assert_eq!(info.group_count, 1);

    assert_eq!(info.gdt_offset, 4096);
}

#[test]
fn test_superblock_valid_1k() {
    let sb = make_superblock(0, 2048, 65536, 8192, 2048, 128, 0x0040);
    let info = ext4::parse_superblock_bytes(&sb).expect("should parse");
    assert_eq!(info.block_size, 1024);
    assert_eq!(info.group_count, 8);

    assert_eq!(info.gdt_offset, 2048);
}

#[test]
fn test_superblock_64bit() {
    let mut sb = make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0040 | 0x0080);

    sb[0xFE..0x100].copy_from_slice(&64u16.to_le_bytes());

    let info = ext4::parse_superblock_bytes(&sb).expect("should parse");
    assert!(info.is_64bit);
    assert_eq!(info.desc_size, 64);
}

#[test]
fn test_superblock_bad_magic() {
    let mut sb = make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0040);
    sb[0x38] = 0x00;
    sb[0x39] = 0x00;
    assert!(ext4::parse_superblock_bytes(&sb).is_none());
}

#[test]
fn test_superblock_bad_block_size() {
    let sb = make_superblock(7, 1024, 32768, 32768, 1024, 256, 0x0040);
    assert!(ext4::parse_superblock_bytes(&sb).is_none());
}

#[test]
fn test_superblock_inode_size_too_small() {
    let sb = make_superblock(2, 1024, 32768, 32768, 1024, 64, 0x0040);
    assert!(ext4::parse_superblock_bytes(&sb).is_none());
}

#[test]
fn test_superblock_no_extents_feature() {
    let sb = make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0000);
    let info = ext4::parse_superblock_bytes(&sb).expect("should parse");
    assert!(!info.has_extents);
}

#[test]
fn test_extent_single_leaf() {
    let data = make_extent_leaf(&[(0, 10, 0, 100)]);
    let extents = ext4::parse_extent_leaves_raw(&data, 4096, 0).expect("should parse");
    assert_eq!(extents.len(), 1);
    assert_eq!(extents[0], (100 * 4096, 10 * 4096));
}

#[test]
fn test_extent_multiple_leaves_non_contiguous() {
    let data = make_extent_leaf(&[(0, 5, 0, 100), (5, 3, 0, 200)]);
    let extents = ext4::parse_extent_leaves_raw(&data, 4096, 0).expect("should parse");
    assert_eq!(extents.len(), 2);
    assert_eq!(extents[0], (100 * 4096, 5 * 4096));
    assert_eq!(extents[1], (200 * 4096, 3 * 4096));
}

#[test]
fn test_extent_contiguous_coalescing() {
    let data = make_extent_leaf(&[(0, 5, 0, 100), (5, 5, 0, 105)]);
    let extents = ext4::parse_extent_leaves_raw(&data, 4096, 0).expect("should parse");
    assert_eq!(extents.len(), 1);
    assert_eq!(extents[0], (100 * 4096, 10 * 4096));
}

#[test]
fn test_extent_with_partition_offset() {
    let partition_offset = 1048576u64;
    let data = make_extent_leaf(&[(0, 8, 0, 50)]);
    let extents =
        ext4::parse_extent_leaves_raw(&data, 4096, partition_offset).expect("should parse");
    assert_eq!(extents.len(), 1);
    assert_eq!(extents[0], (partition_offset + 50 * 4096, 8 * 4096));
}

#[test]
fn test_extent_hi_bits() {
    let data = make_extent_leaf(&[(0, 2, 1, 0)]);
    let extents = ext4::parse_extent_leaves_raw(&data, 4096, 0).expect("should parse");
    let expected_block = (1u64 << 32) * 4096;
    assert_eq!(extents[0].0, expected_block);
}

#[test]
fn test_extent_uninitialized_skipped() {
    let data = make_extent_leaf(&[(0, 5, 0, 0)]);
    assert!(ext4::parse_extent_leaves_raw(&data, 4096, 0).is_none());
}

#[test]
fn test_extent_bad_magic() {
    let mut data = make_extent_leaf(&[(0, 10, 0, 100)]);
    data[0] = 0x00;
    data[1] = 0x00;
    assert!(ext4::parse_extent_leaves_raw(&data, 4096, 0).is_none());
}

#[test]
fn test_extent_zero_entries() {
    let mut data = make_extent_leaf(&[(0, 10, 0, 100)]);

    data[2] = 0;
    data[3] = 0;
    assert!(ext4::parse_extent_leaves_raw(&data, 4096, 0).is_none());
}

#[test]
fn test_extent_uninitialized_len_bit_masked() {
    let data = make_extent_leaf(&[(0, 0x8005u16, 0, 100)]);
    let extents = ext4::parse_extent_leaves_raw(&data, 4096, 0).expect("should parse");
    assert_eq!(extents[0].1, 5 * 4096);
}

#[test]
fn test_block_to_offset() {
    let sb = make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0040);
    let info = ext4::parse_superblock_bytes(&sb).unwrap();
    assert_eq!(info.block_to_offset(0), 0);
    assert_eq!(info.block_to_offset(1), 4096);
    assert_eq!(info.block_to_offset(100), 100 * 4096);
}

#[test]
fn test_block_to_offset_with_partition() {
    let mut info =
        ext4::parse_superblock_bytes(&make_superblock(2, 1024, 32768, 32768, 1024, 256, 0x0040))
            .unwrap();
    info.partition_offset = 1_048_576;
    assert_eq!(info.block_to_offset(0), 1_048_576);
    assert_eq!(info.block_to_offset(10), 1_048_576 + 10 * 4096);
}
