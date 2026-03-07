use super::{read_exact, FsHintMap};
use crate::io::{AlignedBuffer, DiskReader};

const EXT4_MAGIC: u16 = 0xEF53;
const SUPERBLOCK_OFFSET: u64 = 1024;
const SUPERBLOCK_SIZE: usize = 1024;
const INCOMPAT_EXTENTS: u32 = 0x0040;
const INCOMPAT_64BIT: u32 = 0x0080;
const EXTENT_MAGIC: u16 = 0xF30A;
const MAX_INODES_SCANNED: usize = 1_000_000;
const MAX_EXTENT_DEPTH: u16 = 5;
const MIN_USEFUL_SIZE: u64 = 50 * 1024;

#[derive(Debug, Clone)]
pub struct Ext4Info {
    pub partition_offset: u64,
    pub block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub inode_size: u16,
    pub inodes_count: u32,
    pub group_count: u32,
    pub has_extents: bool,
    pub is_64bit: bool,
    pub desc_size: u16,
    pub gdt_offset: u64,
}

impl Ext4Info {
    #[inline]
    pub fn block_to_offset(&self, block: u64) -> u64 {
        self.partition_offset + block * self.block_size as u64
    }
}

#[derive(Debug, Clone, Copy)]
struct GroupDesc {
    inode_table_block: u64,
}

pub fn detect_ext4(
    reader: &DiskReader,
    partition_offset: u64,
    buffer: &mut AlignedBuffer,
) -> Option<Ext4Info> {
    let sb_disk_offset = partition_offset + SUPERBLOCK_OFFSET;

    let mut sb = [0u8; SUPERBLOCK_SIZE];
    if !read_exact(reader, sb_disk_offset, &mut sb, buffer) {
        return None;
    }

    let magic = u16::from_le_bytes([sb[0x38], sb[0x39]]);
    if magic != EXT4_MAGIC {
        return None;
    }

    let s_log_block_size = u32::from_le_bytes([sb[0x18], sb[0x19], sb[0x1A], sb[0x1B]]);
    let block_size = 1024u32.checked_shl(s_log_block_size)?;

    if !(1024..=65536).contains(&block_size) || !block_size.is_power_of_two() {
        return None;
    }

    let inodes_count = u32::from_le_bytes([sb[0x00], sb[0x01], sb[0x02], sb[0x03]]);
    let blocks_count_lo = u32::from_le_bytes([sb[0x04], sb[0x05], sb[0x06], sb[0x07]]);
    let blocks_per_group = u32::from_le_bytes([sb[0x20], sb[0x21], sb[0x22], sb[0x23]]);
    let inodes_per_group = u32::from_le_bytes([sb[0x28], sb[0x29], sb[0x2A], sb[0x2B]]);
    let inode_size = u16::from_le_bytes([sb[0x58], sb[0x59]]);

    if blocks_per_group == 0 || inodes_per_group == 0 || inode_size < 128 {
        return None;
    }

    let s_feature_incompat = u32::from_le_bytes([sb[0x60], sb[0x61], sb[0x62], sb[0x63]]);
    let has_extents = s_feature_incompat & INCOMPAT_EXTENTS != 0;
    let is_64bit = s_feature_incompat & INCOMPAT_64BIT != 0;

    let desc_size = if is_64bit {
        let ds = u16::from_le_bytes([sb[0xFE], sb[0xFF]]);
        if ds >= 64 {
            ds
        } else {
            32
        }
    } else {
        32
    };

    let group_count = (blocks_count_lo + blocks_per_group - 1) / blocks_per_group;
    if group_count == 0 {
        return None;
    }

    let gdt_block = if block_size == 1024 { 2 } else { 1 };
    let gdt_offset = partition_offset + gdt_block as u64 * block_size as u64;

    Some(Ext4Info {
        partition_offset,
        block_size,
        blocks_per_group,
        inodes_per_group,
        inode_size,
        inodes_count,
        group_count,
        has_extents,
        is_64bit,
        desc_size,
        gdt_offset,
    })
}

fn read_gdt(reader: &DiskReader, info: &Ext4Info, buffer: &mut AlignedBuffer) -> Vec<GroupDesc> {
    let total_bytes = info.group_count as usize * info.desc_size as usize;
    let mut raw = vec![0u8; total_bytes];

    if !read_exact(reader, info.gdt_offset, &mut raw, buffer) {
        return Vec::new();
    }

    let mut groups = Vec::with_capacity(info.group_count as usize);
    for i in 0..info.group_count as usize {
        let off = i * info.desc_size as usize;
        if off + 32 > raw.len() {
            break;
        }

        let lo = u32::from_le_bytes([raw[off + 8], raw[off + 9], raw[off + 10], raw[off + 11]]);
        let hi = if info.is_64bit && info.desc_size >= 64 && off + 44 <= raw.len() {
            u32::from_le_bytes([raw[off + 40], raw[off + 41], raw[off + 42], raw[off + 43]])
        } else {
            0
        };

        let inode_table_block = lo as u64 | ((hi as u64) << 32);
        groups.push(GroupDesc { inode_table_block });
    }

    groups
}

fn parse_extent_tree_inline(data: &[u8], info: &Ext4Info) -> Option<Vec<(u64, u64)>> {
    if data.len() < 12 {
        return None;
    }

    let eh_magic = u16::from_le_bytes([data[0], data[1]]);
    if eh_magic != EXTENT_MAGIC {
        return None;
    }

    let eh_entries = u16::from_le_bytes([data[2], data[3]]);
    let eh_depth = u16::from_le_bytes([data[6], data[7]]);

    if eh_depth > MAX_EXTENT_DEPTH || eh_depth != 0 || eh_entries == 0 {
        return None;
    }

    parse_extent_leaves(data, eh_entries, info)
}

fn parse_extent_leaves(data: &[u8], count: u16, info: &Ext4Info) -> Option<Vec<(u64, u64)>> {
    let mut extents = Vec::with_capacity(count as usize);

    for i in 0..count as usize {
        let off = 12 + i * 12;
        if off + 12 > data.len() {
            break;
        }

        let ee_len = u16::from_le_bytes([data[off + 4], data[off + 5]]);
        let ee_start_hi = u16::from_le_bytes([data[off + 6], data[off + 7]]);
        let ee_start_lo =
            u32::from_le_bytes([data[off + 8], data[off + 9], data[off + 10], data[off + 11]]);

        let phys_block = ee_start_lo as u64 | ((ee_start_hi as u64) << 32);
        if phys_block == 0 {
            continue;
        }

        let len_blocks = (ee_len & 0x7FFF) as u64;
        if len_blocks == 0 {
            continue;
        }

        let abs_offset = info.block_to_offset(phys_block);
        let length = len_blocks * info.block_size as u64;

        if let Some(last) = extents.last_mut() {
            let (last_off, last_len): &mut (u64, u64) = last;
            if *last_off + *last_len == abs_offset {
                *last_len += length;
                continue;
            }
        }

        extents.push((abs_offset, length));
    }

    if extents.is_empty() {
        None
    } else {
        Some(extents)
    }
}

fn parse_extent_tree_block(
    reader: &DiskReader,
    info: &Ext4Info,
    block_num: u64,
    remaining_depth: u16,
    buffer: &mut AlignedBuffer,
) -> Option<Vec<(u64, u64)>> {
    if remaining_depth == 0 || remaining_depth > MAX_EXTENT_DEPTH {
        return None;
    }

    let block_offset = info.block_to_offset(block_num);
    let block_size = info.block_size as usize;
    let mut block_buf = vec![0u8; block_size];

    if !read_exact(reader, block_offset, &mut block_buf, buffer) {
        return None;
    }

    if block_buf.len() < 12 {
        return None;
    }

    let eh_magic = u16::from_le_bytes([block_buf[0], block_buf[1]]);
    if eh_magic != EXTENT_MAGIC {
        return None;
    }

    let eh_entries = u16::from_le_bytes([block_buf[2], block_buf[3]]);
    let eh_depth = u16::from_le_bytes([block_buf[6], block_buf[7]]);

    if eh_entries == 0 {
        return None;
    }

    if eh_depth == 0 {
        return parse_extent_leaves(&block_buf, eh_entries, info);
    }

    let mut all_extents = Vec::new();

    for i in 0..eh_entries as usize {
        let off = 12 + i * 12;
        if off + 12 > block_buf.len() {
            break;
        }

        let ei_leaf_lo = u32::from_le_bytes([
            block_buf[off + 4],
            block_buf[off + 5],
            block_buf[off + 6],
            block_buf[off + 7],
        ]);
        let ei_leaf_hi = u16::from_le_bytes([block_buf[off + 8], block_buf[off + 9]]);
        let child_block = ei_leaf_lo as u64 | ((ei_leaf_hi as u64) << 32);

        if child_block == 0 {
            continue;
        }

        if let Some(child_extents) =
            parse_extent_tree_block(reader, info, child_block, remaining_depth - 1, buffer)
        {
            for extent in child_extents {
                if let Some(last) = all_extents.last_mut() {
                    let (last_off, last_len): &mut (u64, u64) = last;
                    if *last_off + *last_len == extent.0 {
                        *last_len += extent.1;
                        continue;
                    }
                }
                all_extents.push(extent);
            }
        }
    }

    if all_extents.is_empty() {
        None
    } else {
        Some(all_extents)
    }
}

pub fn parse_full_extent_tree(
    inode_iblock: &[u8],
    info: &Ext4Info,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
) -> Option<Vec<(u64, u64)>> {
    if inode_iblock.len() < 12 {
        return None;
    }

    let eh_magic = u16::from_le_bytes([inode_iblock[0], inode_iblock[1]]);
    if eh_magic != EXTENT_MAGIC {
        return None;
    }

    let eh_entries = u16::from_le_bytes([inode_iblock[2], inode_iblock[3]]);
    let eh_depth = u16::from_le_bytes([inode_iblock[6], inode_iblock[7]]);

    if eh_depth > MAX_EXTENT_DEPTH || eh_entries == 0 {
        return None;
    }

    if eh_depth == 0 {
        return parse_extent_leaves(inode_iblock, eh_entries, info);
    }

    let mut all_extents = Vec::new();

    for i in 0..eh_entries as usize {
        let off = 12 + i * 12;
        if off + 12 > inode_iblock.len() {
            break;
        }

        let ei_leaf_lo = u32::from_le_bytes([
            inode_iblock[off + 4],
            inode_iblock[off + 5],
            inode_iblock[off + 6],
            inode_iblock[off + 7],
        ]);
        let ei_leaf_hi = u16::from_le_bytes([inode_iblock[off + 8], inode_iblock[off + 9]]);
        let child_block = ei_leaf_lo as u64 | ((ei_leaf_hi as u64) << 32);

        if child_block == 0 {
            continue;
        }

        if let Some(child_extents) =
            parse_extent_tree_block(reader, info, child_block, eh_depth, buffer)
        {
            for extent in child_extents {
                if let Some(last) = all_extents.last_mut() {
                    let (last_off, last_len): &mut (u64, u64) = last;
                    if *last_off + *last_len == extent.0 {
                        *last_len += extent.1;
                        continue;
                    }
                }
                all_extents.push(extent);
            }
        }
    }

    if all_extents.is_empty() {
        None
    } else {
        Some(all_extents)
    }
}

pub fn collect_ext4_hints_deep(
    reader: &DiskReader,
    info: &Ext4Info,
    buffer: &mut AlignedBuffer,
    hints: &mut FsHintMap,
) {
    if !info.has_extents {
        return;
    }

    let groups = read_gdt(reader, info, buffer);
    if groups.is_empty() {
        return;
    }

    let inode_size = info.inode_size as usize;
    let inodes_per_group = info.inodes_per_group as usize;
    let table_bytes = inodes_per_group * inode_size;
    let mut table_buf = vec![0u8; table_bytes];
    let mut total_scanned = 0usize;

    for group in &groups {
        if total_scanned >= MAX_INODES_SCANNED {
            break;
        }

        let table_offset = info.block_to_offset(group.inode_table_block);
        if table_offset + table_bytes as u64 > reader.size() {
            continue;
        }

        if !read_exact(reader, table_offset, &mut table_buf, buffer) {
            continue;
        }

        for i in 0..inodes_per_group {
            if total_scanned >= MAX_INODES_SCANNED {
                break;
            }
            total_scanned += 1;

            let off = i * inode_size;
            if off + 128 > table_buf.len() {
                break;
            }

            let inode = &table_buf[off..off + inode_size.min(table_buf.len() - off)];
            if let Some(hint) = parse_deleted_inode_deep(inode, info, reader, buffer) {
                hints.entry(hint.data_start).or_insert(hint);
            }
        }
    }
}

fn parse_deleted_inode_deep(
    inode: &[u8],
    info: &Ext4Info,
    reader: &DiskReader,
    buffer: &mut AlignedBuffer,
) -> Option<super::FsHint> {
    if inode.len() < 128 {
        return None;
    }

    let i_mode = u16::from_le_bytes([inode[0x00], inode[0x01]]);
    if i_mode & 0xF000 != 0x8000 {
        return None;
    }

    let i_links_count = u16::from_le_bytes([inode[0x1A], inode[0x1B]]);
    if i_links_count != 0 {
        return None;
    }

    let i_size_lo = u32::from_le_bytes([inode[0x04], inode[0x05], inode[0x06], inode[0x07]]);
    let i_size_hi = if inode.len() >= 0x70 {
        u32::from_le_bytes([inode[0x6C], inode[0x6D], inode[0x6E], inode[0x6F]])
    } else {
        0
    };
    let file_size = i_size_lo as u64 | ((i_size_hi as u64) << 32);

    if file_size < MIN_USEFUL_SIZE {
        return None;
    }

    let i_flags = u32::from_le_bytes([inode[0x20], inode[0x21], inode[0x22], inode[0x23]]);
    if i_flags & 0x0008_0000 == 0 {
        return None;
    }

    let i_block = &inode[0x28..0x28 + 60];
    let extents = parse_full_extent_tree(i_block, info, reader, buffer)?;
    if extents.is_empty() {
        return None;
    }

    let data_start = extents[0].0;
    Some(super::FsHint {
        data_start,
        data_size: file_size,
        extents,
    })
}

pub fn parse_superblock_bytes(sb: &[u8]) -> Option<Ext4Info> {
    if sb.len() < 256 {
        return None;
    }

    let magic = u16::from_le_bytes([sb[0x38], sb[0x39]]);
    if magic != EXT4_MAGIC {
        return None;
    }

    let s_log_block_size = u32::from_le_bytes([sb[0x18], sb[0x19], sb[0x1A], sb[0x1B]]);
    let block_size = 1024u32.checked_shl(s_log_block_size)?;

    if !(1024..=65536).contains(&block_size) || !block_size.is_power_of_two() {
        return None;
    }

    let inodes_count = u32::from_le_bytes([sb[0x00], sb[0x01], sb[0x02], sb[0x03]]);
    let blocks_count_lo = u32::from_le_bytes([sb[0x04], sb[0x05], sb[0x06], sb[0x07]]);
    let blocks_per_group = u32::from_le_bytes([sb[0x20], sb[0x21], sb[0x22], sb[0x23]]);
    let inodes_per_group = u32::from_le_bytes([sb[0x28], sb[0x29], sb[0x2A], sb[0x2B]]);
    let inode_size = u16::from_le_bytes([sb[0x58], sb[0x59]]);

    if blocks_per_group == 0 || inodes_per_group == 0 || inode_size < 128 {
        return None;
    }

    let s_feature_incompat = u32::from_le_bytes([sb[0x60], sb[0x61], sb[0x62], sb[0x63]]);
    let has_extents = s_feature_incompat & INCOMPAT_EXTENTS != 0;
    let is_64bit = s_feature_incompat & INCOMPAT_64BIT != 0;

    let desc_size = if is_64bit {
        let ds = u16::from_le_bytes([sb[0xFE], sb[0xFF]]);
        if ds >= 64 {
            ds
        } else {
            32
        }
    } else {
        32
    };

    let group_count = (blocks_count_lo + blocks_per_group - 1) / blocks_per_group;

    let gdt_block: u64 = if block_size == 1024 { 2 } else { 1 };
    let gdt_offset = gdt_block * block_size as u64;

    Some(Ext4Info {
        partition_offset: 0,
        block_size,
        blocks_per_group,
        inodes_per_group,
        inode_size,
        inodes_count,
        group_count,
        has_extents,
        is_64bit,
        desc_size,
        gdt_offset,
    })
}

pub fn parse_extent_leaves_raw(
    data: &[u8],
    block_size: u32,
    partition_offset: u64,
) -> Option<Vec<(u64, u64)>> {
    let info = Ext4Info {
        partition_offset,
        block_size,
        blocks_per_group: 0,
        inodes_per_group: 0,
        inode_size: 256,
        inodes_count: 0,
        group_count: 0,
        has_extents: true,
        is_64bit: false,
        desc_size: 32,
        gdt_offset: 0,
    };
    parse_extent_tree_inline(data, &info)
}
