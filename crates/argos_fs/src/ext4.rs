//! ext2/3/4 deleted-file recovery, primarily through the jbd2 journal.
//!
//! On-disk layout implemented (source: the ext4 disk layout documentation and
//! the jbd2 format): the superblock (magic `0xEF53` at offset 56 of the 1024
//! byte superblock area) and its backups, the group descriptor table, inodes
//! with extent trees (`0xF30A` header magic), linked directory blocks, and
//! jbd2 descriptor/data blocks.
//!
//! Why the journal: deleting a file on ext4 zeroes the inode's extent tree in
//! place, so an in-place inode yields nothing. The journal retains *older
//! copies* of those inode-table blocks, whose extent trees are intact — that
//! is where deleted extents come from, at tier `JournalResidue`.

use std::io::{Read, Seek};
use std::time::{Duration, SystemTime};

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};

use crate::bytes::{read_at, u16_le, u32_be, u32_le};
use crate::{DeletedFile, FsError, FsKind, Timestamps};

/// Superblock magic, at offset 56 within the superblock. Source: ext4 layout.
const SUPERBLOCK_MAGIC: u16 = 0xEF53;

/// Byte offset of the primary superblock within a volume. Source: ext4 layout
/// (the first 1024 bytes are reserved for boot code).
pub const SUPERBLOCK_OFFSET: u64 = 1024;

/// Size of the superblock structure in bytes.
const SUPERBLOCK_BYTES: usize = 1024;

/// Extent tree node header magic. Source: ext4 `ext4_extent_header`.
const EXTENT_MAGIC: u16 = 0xF30A;

/// Maximum extent-tree depth the walker follows. The format caps real trees
/// at 5 levels; deeper is corrupt, and the bound makes cycles terminate.
const MAX_EXTENT_DEPTH: u16 = 5;

/// Maximum entries honoured in one extent node, independent of the on-disk
/// count: a 4 KiB block holds at most (4096-12)/12 = 340 entries.
const MAX_EXTENT_ENTRIES: u16 = 340;

/// jbd2 block header magic (big-endian). Source: jbd2 `journal_header_t`.
const JBD2_MAGIC: u32 = 0xC03B_3998;

/// jbd2 block types used here. Source: jbd2 format.
const JBD2_DESCRIPTOR_BLOCK: u32 = 1;

/// jbd2 tag flags: last tag in the descriptor, and "same uuid as previous".
const JBD2_FLAG_SAME_UUID: u32 = 2;
const JBD2_FLAG_LAST_TAG: u32 = 8;

/// Cap on journal blocks walked, so a crafted journal cannot spin forever.
const MAX_JOURNAL_BLOCKS: u64 = 1 << 20;

/// Cap on block groups walked. A 1 KiB-block filesystem tops out near 512 Ki
/// groups at the format's maximum size; a superblock claiming more has
/// inconsistent counts, and without this bound the group walk is driven
/// directly by two unvalidated fields (A-UNTRUSTED-ONDISK).
const MAX_GROUPS: u64 = 1 << 20;

/// Cap on filesystem blocks collected from one extent tree, bounding the
/// block list a crafted inode can grow before any walk limit applies.
const MAX_TREE_BLOCKS: usize = 1 << 20;

/// Geometry of an ext filesystem, parsed from its superblock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ext4 {
    /// Absolute byte offset of the volume start.
    pub volume_offset: ByteOffset,
    /// Bytes per filesystem block.
    pub block_bytes: u64,
    /// Bytes per inode structure.
    pub inode_bytes: u16,
    /// Inodes per block group.
    pub inodes_per_group: u32,
    /// Blocks per block group.
    pub blocks_per_group: u32,
    /// Total block count.
    pub block_count: u64,
    /// Inode number holding the journal (`$sb.s_journal_inum`).
    pub journal_inode: u32,
    /// Size of a group descriptor in bytes (32 or 64).
    pub desc_bytes: u16,
}

impl Ext4 {
    /// Reads the superblock at `volume_offset`, falling back to backup
    /// superblocks at the conventional group starts when the primary fails.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn open<R: Read + Seek>(
        src: &mut R,
        volume_offset: ByteOffset,
    ) -> Result<Option<Self>, FsError> {
        let mut buf = Vec::new();
        let primary = volume_offset.get().saturating_add(SUPERBLOCK_OFFSET);
        if read_at(src, primary, SUPERBLOCK_BYTES, &mut buf)?
            && let Some(fs) = Self::from_superblock(&buf, volume_offset)
        {
            return Ok(Some(fs));
        }

        // Backup superblocks live at the start of groups 1, 3, 5^n, 7^n, 9^n
        // (sparse_super). Trying the first few recovers a wiped primary.
        for group in [1_u64, 3, 5, 7, 9, 25, 27, 49, 81] {
            // Block size is unknown without a superblock; probe the standard
            // 1/2/4 KiB block sizes.
            for block_bytes in [1024_u64, 2048, 4096] {
                let blocks_per_group = block_bytes * 8;
                let at = volume_offset
                    .get()
                    .saturating_add(group.saturating_mul(blocks_per_group) * block_bytes);
                let sb_at = if block_bytes == 1024 {
                    at.saturating_add(1024)
                } else {
                    at
                };
                if read_at(src, sb_at, SUPERBLOCK_BYTES, &mut buf)?
                    && let Some(fs) = Self::from_superblock(&buf, volume_offset)
                    && fs.block_bytes == block_bytes
                {
                    return Ok(Some(fs));
                }
            }
        }
        Ok(None)
    }

    /// Interprets `raw` as a superblock (also the residue-sweep anchor
    /// validator: self-consistency decides, never position).
    #[must_use]
    pub fn from_superblock(raw: &[u8], volume_offset: ByteOffset) -> Option<Self> {
        if u16_le(raw, 56)? != SUPERBLOCK_MAGIC {
            return None;
        }
        let log_block_size = u32_le(raw, 24)?;
        if log_block_size > 6 {
            return None;
        }
        let block_bytes = 1024_u64.checked_shl(log_block_size)?;
        let inodes_per_group = u32_le(raw, 40)?;
        let blocks_per_group = u32_le(raw, 32)?;
        if inodes_per_group == 0 || blocks_per_group == 0 {
            return None;
        }
        let inode_bytes = u16_le(raw, 88)?;
        // Pre-ext4 filesystems record 0, meaning the historical 128 bytes.
        let inode_bytes = if inode_bytes == 0 { 128 } else { inode_bytes };
        if !(128..=4096).contains(&inode_bytes) || !inode_bytes.is_power_of_two() {
            return None;
        }
        let desc_bytes = u16_le(raw, 254).filter(|&size| size >= 32).unwrap_or(32);
        Some(Self {
            volume_offset,
            block_bytes,
            inode_bytes,
            inodes_per_group,
            blocks_per_group,
            block_count: u64::from(u32_le(raw, 4)?),
            journal_inode: u32_le(raw, 224).filter(|&inum| inum != 0).unwrap_or(8),
            desc_bytes,
        })
    }

    /// Absolute byte offset of filesystem block `block`.
    fn block_at(&self, block: u64) -> Option<ByteOffset> {
        self.volume_offset
            .checked_add(block.checked_mul(self.block_bytes)?)
    }

    /// Reads the inode-table start block of group `group`.
    fn inode_table_block<R: Read + Seek>(
        &self,
        src: &mut R,
        group: u32,
    ) -> Result<Option<u64>, FsError> {
        // The group descriptor table starts in the block after the superblock.
        let gdt_block = if self.block_bytes == 1024 { 2 } else { 1 };
        let Some(gdt_at) = self.block_at(gdt_block) else {
            return Ok(None);
        };
        let Some(entry_at) = u64::from(group)
            .checked_mul(u64::from(self.desc_bytes))
            .and_then(|delta| gdt_at.checked_add(delta))
        else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        if !read_at(src, entry_at.get(), usize::from(self.desc_bytes), &mut buf)? {
            return Ok(None);
        }
        // `bg_inode_table_lo` at 8, `_hi` at 40 for 64-byte descriptors.
        let Some(lo) = u32_le(&buf, 8) else {
            return Ok(None);
        };
        let hi = if self.desc_bytes >= 64 {
            u32_le(&buf, 40).unwrap_or(0)
        } else {
            0
        };
        Ok(Some(u64::from(lo) | (u64::from(hi) << 32)))
    }

    /// Mines the jbd2 journal for stale inode-table copies and returns the
    /// deleted files whose extent trees survive there.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; corrupt journal blocks are skipped.
    pub fn recover_from_journal<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let Some(journal_blocks) = self.journal_extents(src)? else {
            return Ok(Vec::new());
        };
        // Map every inode-table block to (group, index) so a journalled copy
        // can be interpreted as inodes.
        let mut tables = Vec::new();
        let group_count = self
            .block_count
            .div_ceil(u64::from(self.blocks_per_group))
            .min(MAX_GROUPS);
        for group in 0..u32::try_from(group_count).unwrap_or(u32::MAX) {
            if let Some(start) = self.inode_table_block(src, group)? {
                let blocks = u64::from(self.inodes_per_group)
                    .saturating_mul(u64::from(self.inode_bytes))
                    .div_ceil(self.block_bytes);
                tables.push((group, start, blocks));
            }
        }

        let mut found = Vec::new();
        let mut header = Vec::new();
        let mut payload = Vec::new();
        let block_len = usize::try_from(self.block_bytes).unwrap_or(4096);

        let mut walked = 0_u64;
        let mut index = 0_usize;
        while index < journal_blocks.len() && walked < MAX_JOURNAL_BLOCKS {
            let Some(at) = self.block_at(journal_blocks[index]) else {
                break;
            };
            if !read_at(src, at.get(), block_len, &mut header)? {
                break;
            }
            walked += 1;
            index += 1;
            if u32_be(&header, 0) != Some(JBD2_MAGIC)
                || u32_be(&header, 4) != Some(JBD2_DESCRIPTOR_BLOCK)
            {
                continue;
            }
            // Tags map the data blocks that follow, in order.
            for target in descriptor_targets(&header) {
                if index >= journal_blocks.len() {
                    break;
                }
                let Some(data_at) = self.block_at(journal_blocks[index]) else {
                    break;
                };
                index += 1;
                walked += 1;
                if !tables.iter().any(|&(_, start, blocks)| {
                    start
                        .checked_add(blocks)
                        .is_some_and(|end| (start..end).contains(&target))
                }) {
                    continue;
                }
                if !read_at(src, data_at.get(), block_len, &mut payload)? {
                    break;
                }
                // The journalled block is a stale copy of an inode-table
                // block: every slot in it is an inode as it was before the
                // deletion, extent tree intact.
                let inodes_per_block = self.block_bytes / u64::from(self.inode_bytes);
                for slot in 0..inodes_per_block {
                    let at = usize::try_from(slot * u64::from(self.inode_bytes)).unwrap_or(0);
                    let Some(raw) = payload.get(at..at + usize::from(self.inode_bytes)) else {
                        break;
                    };
                    // Inode numbers are one-based within the table.
                    if let Some(file) = self.deleted_from_inode(raw, slot + 1) {
                        found.push(file);
                    }
                }
            }
        }
        Ok(found)
    }

    /// Block numbers making up the journal file, from its inode.
    fn journal_extents<R: Read + Seek>(&self, src: &mut R) -> Result<Option<Vec<u64>>, FsError> {
        let group = (self.journal_inode - 1) / self.inodes_per_group;
        let index = (self.journal_inode - 1) % self.inodes_per_group;
        let Some(table) = self.inode_table_block(src, group)? else {
            return Ok(None);
        };
        let Some(at) = self.block_at(table).and_then(|base| {
            base.checked_add(u64::from(index).checked_mul(u64::from(self.inode_bytes))?)
        }) else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        if !read_at(src, at.get(), usize::from(self.inode_bytes), &mut buf)? {
            return Ok(None);
        }
        let Some(extents) = extent_tree_blocks(&buf, self, src)? else {
            return Ok(None);
        };
        Ok(Some(extents))
    }

    /// Interprets a journalled inode copy: a deleted file keeps its extent
    /// tree here even though the in-place inode has it zeroed.
    fn deleted_from_inode(&self, raw: &[u8], inode_number: u64) -> Option<DeletedFile> {
        let links = u16_le(raw, 26)?;
        let dtime = u32_le(raw, 20)?;
        let mode = u16_le(raw, 0)?;
        // Regular file (S_IFREG), unlinked, with a deletion time recorded.
        if mode & 0xF000 != 0x8000 || links != 0 || dtime == 0 {
            return None;
        }
        let size = u64::from(u32_le(raw, 4)?) | (u64::from(u32_le(raw, 108)?) << 32);
        if size == 0 {
            return None;
        }
        let extents = self.inline_extents(raw, size)?;
        if extents.is_empty() {
            return None;
        }
        Some(DeletedFile {
            name: None,
            timestamps: Timestamps {
                created: unix_time(u32_le(raw, 12)?),
                modified: unix_time(u32_le(raw, 16)?),
            },
            size,
            extents,
            fs: FsKind::Ext4,
            confidence: Confidence::JournalResidue,
            source_object: Some(inode_number),
        })
    }

    /// Depth-0 extent entries of an inode, as absolute byte ranges. Deeper
    /// trees need index-block reads and are handled by the journal walker's
    /// caller path; here an index node yields no inline extents.
    fn inline_extents(&self, inode: &[u8], size: u64) -> Option<Vec<ByteRange>> {
        let body = inode.get(40..100)?;
        if u16_le(body, 0)? != EXTENT_MAGIC {
            return None;
        }
        let entries = u16_le(body, 2)?.min(MAX_EXTENT_ENTRIES);
        let depth = u16_le(body, 6)?;
        if depth > MAX_EXTENT_DEPTH {
            return None;
        }
        if depth != 0 {
            return Some(Vec::new());
        }
        let mut out = Vec::with_capacity(usize::from(entries));
        let mut remaining = size;
        for index in 0..usize::from(entries) {
            let at = 12 + index * 12;
            let len = u16_le(body, at + 4)?;
            // Lengths above 32768 mark uninitialized extents (no content).
            if len == 0 || len > 32768 {
                continue;
            }
            let start_hi = u16_le(body, at + 6)?;
            let start_lo = u32_le(body, at + 8)?;
            let block = u64::from(start_lo) | (u64::from(start_hi) << 32);
            let bytes = u64::from(len).checked_mul(self.block_bytes)?.min(remaining);
            out.push(ByteRange::new(self.block_at(block)?, bytes));
            remaining = remaining.saturating_sub(bytes);
        }
        Some(out)
    }
}

/// Block numbers an inode's depth-0 extent tree covers (used for the journal
/// file itself, whose tree may be one level deep).
fn extent_tree_blocks<R: Read + Seek>(
    inode: &[u8],
    fs: &Ext4,
    src: &mut R,
) -> Result<Option<Vec<u64>>, FsError> {
    let Some(body) = inode.get(40..100) else {
        return Ok(None);
    };
    if u16_le(body, 0) != Some(EXTENT_MAGIC) {
        return Ok(None);
    }
    let Some(entries) = u16_le(body, 2).map(|n| n.min(MAX_EXTENT_ENTRIES)) else {
        return Ok(None);
    };
    let Some(depth) = u16_le(body, 6) else {
        return Ok(None);
    };
    if depth > MAX_EXTENT_DEPTH {
        return Ok(None);
    }

    let mut blocks = Vec::new();
    if depth == 0 {
        collect_leaf(body, entries, &mut blocks);
        return Ok(Some(blocks));
    }

    // One index level: each entry points at a leaf block.
    let mut node = Vec::new();
    let block_len = usize::try_from(fs.block_bytes).unwrap_or(4096);
    for index in 0..usize::from(entries) {
        let at = 12 + index * 12;
        let (Some(lo), Some(hi)) = (u32_le(body, at + 4), u16_le(body, at + 8)) else {
            break;
        };
        let leaf_block = u64::from(lo) | (u64::from(hi) << 32);
        let Some(leaf_at) = fs.block_at(leaf_block) else {
            break;
        };
        if !read_at(src, leaf_at.get(), block_len, &mut node)? {
            break;
        }
        if u16_le(&node, 0) != Some(EXTENT_MAGIC) || u16_le(&node, 6) != Some(0) {
            continue;
        }
        let leaf_entries = u16_le(&node, 2).unwrap_or(0).min(MAX_EXTENT_ENTRIES);
        collect_leaf(&node, leaf_entries, &mut blocks);
    }
    Ok(Some(blocks))
}

/// Appends every filesystem block a depth-0 extent node covers, up to
/// [`MAX_TREE_BLOCKS`] in total.
fn collect_leaf(node: &[u8], entries: u16, out: &mut Vec<u64>) {
    for index in 0..usize::from(entries) {
        if out.len() >= MAX_TREE_BLOCKS {
            return;
        }
        let at = 12 + index * 12;
        let (Some(len), Some(hi), Some(lo)) = (
            u16_le(node, at + 4),
            u16_le(node, at + 6),
            u32_le(node, at + 8),
        ) else {
            return;
        };
        if len == 0 || len > 32768 {
            continue;
        }
        let start = u64::from(lo) | (u64::from(hi) << 32);
        for block in 0..u64::from(len) {
            let Some(number) = start.checked_add(block) else {
                return;
            };
            if out.len() >= MAX_TREE_BLOCKS {
                return;
            }
            out.push(number);
        }
    }
}

/// Descriptor-block tag targets, in the order their data blocks follow.
fn descriptor_targets(block: &[u8]) -> Vec<u64> {
    let mut targets = Vec::new();
    // Tags start after the 12-byte journal header (v2 tags are 12 bytes plus
    // 16 for the UUID when not flagged same-uuid).
    let mut at = 12_usize;
    while at + 8 <= block.len() {
        let (Some(blocknr), Some(flags)) = (u32_be(block, at), u32_be(block, at + 4)) else {
            break;
        };
        targets.push(u64::from(blocknr));
        at += 8;
        if flags & JBD2_FLAG_SAME_UUID == 0 {
            at += 16;
        }
        if flags & JBD2_FLAG_LAST_TAG != 0 {
            break;
        }
    }
    targets
}

/// A deleted directory entry recovered by carving a directory block.
#[derive(Clone, PartialEq, Eq)]
pub struct NameGhost {
    /// Name from the entry.
    pub name: String,
    /// Inode number the entry pointed at.
    pub inode: u32,
}

impl std::fmt::Debug for NameGhost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameGhost")
            .field("name", &"<redacted>")
            .field("inode", &self.inode)
            .finish()
    }
}

/// Carves `ext4_dir_entry_2` records out of a directory block, including
/// entries hidden in the slack of a preceding entry's `rec_len`.
#[must_use]
pub fn dir_entries(block: &[u8]) -> Vec<NameGhost> {
    let mut out = Vec::new();
    let mut at = 0_usize;
    while at + 8 <= block.len() {
        let (Some(inode), Some(rec_len), Some(&name_len)) =
            (u32_le(block, at), u16_le(block, at + 4), block.get(at + 6))
        else {
            break;
        };
        let rec_len = usize::from(rec_len);
        // A self-consistent entry: 4-byte aligned record holding its name.
        if rec_len < 12 || rec_len % 4 != 0 || at + rec_len > block.len() {
            at += 4;
            continue;
        }
        if inode != 0
            && name_len > 0
            && usize::from(name_len) + 8 <= rec_len
            && let Some(raw) = block.get(at + 8..at + 8 + usize::from(name_len))
            && let Ok(name) = std::str::from_utf8(raw)
            && !name.contains('\0')
        {
            out.push(NameGhost {
                name: name.to_owned(),
                inode,
            });
        }
        at += rec_len;
    }
    out
}

/// Unix seconds to `SystemTime`; zero means unset.
fn unix_time(secs: u32) -> Option<SystemTime> {
    if secs == 0 {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(u64::from(secs)))
}
