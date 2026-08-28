//! btrfs deleted-file recovery through backup roots and stale copy-on-write blocks.
//!
//! On-disk layout implemented (source: the btrfs on-disk format documentation
//! and `include/uapi/linux/btrfs_tree.h`): the superblock (magic `_BHRfS_M` at
//! offset 64) with its `crc32c` seal and its `sys_chunk_array`, the chunk tree
//! that maps logical addresses to physical ones, B-tree headers, leaves and
//! interior nodes, and the `INODE_ITEM`, `INODE_REF`, `EXTENT_DATA`,
//! `ROOT_ITEM` and `CHUNK_ITEM` records.
//!
//! Recovery paths, per the btrfs spec in `argos-recovery-algorithms`: the
//! superblock retains four historical root sets (`super_roots`), so an older
//! generation's filesystem tree still describes files the newest one has
//! dropped — the copy-on-write counterpart of the APFS checkpoint ring, tier
//! `FsMetadata`. Beyond that ring, copy-on-write means a deleted file's leaf is
//! freed rather than overwritten, so a sweep of the volume at node granularity
//! finds checksum-valid leaves no live root reaches, tier `JournalResidue`.
//!
//! Every tree pointer and every extent address here is **logical** and is
//! resolved through the chunk map before a byte is read; treating logical as
//! physical would read the wrong bytes and report them as evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};
use std::time::{Duration, SystemTime};

use argos_core::{ByteOffset, ByteRange, Confidence};

use crate::{DeletedFile, FsError, FsKind, Timestamps};
use crate::{read_at, u16_le, u32_le, u64_le};

/// Superblock magic (`_BHRfS_M`), at offset 64. Source: btrfs on-disk format.
const SUPERBLOCK_MAGIC: u64 = 0x4D5F_5366_5248_425F;

/// Bytes of one superblock copy. Source: `BTRFS_SUPER_INFO_SIZE`.
const SUPERBLOCK_BYTES: usize = 4096;

/// Physical offsets every superblock copy sits at, primary first.
///
/// Source: `BTRFS_SUPER_MIRROR_OFFSET` — 64 KiB, 64 MiB, 256 GiB, 1 EiB. A copy
/// states which of these it is in its own `bytenr` field, which is what lets a
/// mirror fix the volume start when the primary is gone.
const MIRROR_OFFSETS: [u64; 4] = [0x1_0000, 0x400_0000, 0x40_0000_0000, 0x4_0000_0000_0000];

/// Checksum type this parser accepts: `crc32c`. Source: `btrfs_csum_type`.
///
/// A volume sealed with `xxhash64`, `sha256` or `blake2b` fails its superblock
/// parse and yields nothing, rather than something unverified.
const CSUM_TYPE_CRC32C: u16 = 0;

/// Bytes of a B-tree block header. Source: `sizeof(struct btrfs_header)`.
const HEADER_BYTES: usize = 101;

/// Bytes of one leaf item descriptor. Source: `sizeof(struct btrfs_item)`.
const ITEM_BYTES: usize = 25;

/// Bytes of one interior-node pointer. Source: `sizeof(struct btrfs_key_ptr)`.
const KEY_PTR_BYTES: usize = 33;

/// Bytes of one on-disk key. Source: `sizeof(struct btrfs_disk_key)`.
const KEY_BYTES: usize = 17;

/// Bytes of one chunk stripe. Source: `sizeof(struct btrfs_stripe)`.
const STRIPE_BYTES: usize = 32;

/// Byte at which a `btrfs_chunk`'s stripe array begins. Source: the struct.
const CHUNK_STRIPES_AT: usize = 48;

/// Key types read here. Source: btrfs key-type constants.
const KEY_INODE_ITEM: u8 = 1;
const KEY_INODE_REF: u8 = 12;
const KEY_EXTENT_DATA: u8 = 108;
const KEY_ROOT_ITEM: u8 = 132;
const KEY_CHUNK_ITEM: u8 = 228;

/// Filesystem-tree objectid, and the range subvolumes and snapshots occupy.
/// Source: `BTRFS_FS_TREE_OBJECTID`, `BTRFS_FIRST_FREE_OBJECTID`,
/// `BTRFS_LAST_FREE_OBJECTID`. Everything outside them is an internal tree
/// (extent, chunk, device, checksum, quota, uuid) that holds no file records.
const FS_TREE_OBJECTID: u64 = 5;
const FIRST_FREE_OBJECTID: u64 = 256;
const LAST_FREE_OBJECTID: u64 = u64::MAX - 255;

/// File-extent item types. Source: `btrfs_file_extent_type`.
const EXTENT_INLINE: u8 = 0;
const EXTENT_REGULAR: u8 = 1;
const EXTENT_PREALLOC: u8 = 2;

/// Byte at which an inline extent's payload starts within its item.
/// Source: `BTRFS_FILE_EXTENT_INLINE_DATA_START`.
const EXTENT_INLINE_DATA_AT: usize = 21;

/// Block-group profiles that split one stripe across several devices.
///
/// Source: `BTRFS_BLOCK_GROUP_RAID0|RAID10|RAID5|RAID6`. Argos scans one
/// medium, so a chunk carrying any of these is not mapped at all: the bytes for
/// a given logical address are partly on a device this scan cannot see, and a
/// guessed offset would be fabricated evidence.
const STRIPED_PROFILES: u64 = (1 << 3) | (1 << 6) | (1 << 7) | (1 << 8);

/// `S_IFMT` and `S_IFREG`: only regular files carry recoverable content.
const MODE_FORMAT_MASK: u32 = 0xF000;
const MODE_REGULAR: u32 = 0x8000;

/// Maximum B-tree depth walked. Source: `BTRFS_MAX_LEVEL`. Deeper is corrupt,
/// and the bound is what makes a crafted cycle terminate (A-UNTRUSTED-ONDISK).
const MAX_TREE_DEPTH: u32 = 8;

/// Maximum tree blocks read in one recovery, independent of on-disk counts.
const MAX_NODES: usize = 65_536;

/// Maximum children queued during a walk (A-BOUNDED-ALLOC).
const MAX_QUEUED: usize = 8_192;

/// Maximum records of each kind kept from one generation (A-BOUNDED-ALLOC).
const MAX_RECORDS: usize = 1 << 16;

/// Maximum chunk-map entries. A real volume holds tens; the bound stops a
/// crafted chunk tree growing the map without limit (A-BOUNDED-ALLOC).
const MAX_CHUNKS: usize = 8_192;

/// Maximum stripes honoured in one chunk, independent of its `num_stripes`.
const MAX_STRIPES: usize = 128;

/// Maximum extents accumulated for one file (A-BOUNDED-ALLOC).
const MAX_EXTENTS_PER_FILE: usize = 4_096;

/// Maximum name bytes taken from an `INODE_REF`. Source: `BTRFS_NAME_LEN`.
const MAX_NAME_BYTES: usize = 255;

/// Maximum node-sized blocks the orphan sweep reads from one volume.
///
/// At the default 16 KiB node size this is 16 TiB of surface, so a real volume
/// is covered; a volume claiming more is bounded rather than followed.
const MAX_ORPHAN_BLOCKS: u64 = 1 << 30;

/// Largest node size accepted. Source: btrfs caps `nodesize` at 64 KiB.
const MAX_NODE_BYTES: u64 = 65_536;

/// CRC-32C (Castagnoli) lookup table, reflected polynomial `0x82F6_3B78`.
///
/// btrfs seals every object with CRC-32C, which is a different polynomial from
/// the CRC-32/ISO-HDLC `crc32fast` computes for GPT headers; the two are not
/// interchangeable. Written here rather than taken as a dependency, for the
/// same reason `apfs` writes its own Fletcher-64: it is one loop over a table,
/// it runs only after a magic or `fsid` match, and it must also be reachable
/// from the fixture builder that seals synthetic blocks (`A-ONE-IMPLEMENTATION`
/// — there is no other CRC-32C in the workspace to call).
static CRC32C_TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut index = 0_u32;
    while index < 256 {
        let mut value = index;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 0 {
                value >> 1
            } else {
                (value >> 1) ^ 0x82F6_3B78
            };
            bit += 1;
        }
        table[index as usize] = value;
        index += 1;
    }
    table
};

/// The CRC-32C every btrfs object is sealed with.
///
/// Exposed for fixture builders, which must seal synthetic blocks for the
/// validators here to accept them.
#[must_use]
pub fn crc32c(body: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in body {
        let index = usize::from(((crc ^ u32::from(byte)) & 0xFF) as u8);
        crc = CRC32C_TABLE[index] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Whether `raw` carries the superblock magic at offset 64.
///
/// An eight-byte test, so the residue sweep can skip the checksum and the
/// block-sized staging read [`Btrfs::open`] performs. The sweep runs at every
/// sector boundary of the whole surface, which makes this the difference
/// between a scan bounded by the medium and one bounded by memory bandwidth
/// (`M-HOTPATH`). It decides nothing on its own: [`from_superblock`] still
/// verifies everything before a volume is reported.
#[must_use]
pub(crate) fn has_superblock_magic(raw: &[u8]) -> bool {
    u64_le(raw, 64) == Some(SUPERBLOCK_MAGIC)
}

/// A btrfs volume located from one superblock copy.
///
/// `volume_offset` is corrected for which mirror the anchor turned out to be,
/// so a copy found 64 MiB into the medium still names the volume's own start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchored {
    /// Absolute byte offset of the volume start.
    pub volume_offset: ByteOffset,
    /// Bytes in the unit data extents are allocated in.
    pub sector_bytes: u64,
    /// Bytes this device contributes, from `dev_item.total_bytes`.
    pub total_bytes: u64,
}

/// Interprets `raw` as a superblock copy found at `anchor`.
///
/// Validation is the magic, a `bytenr` naming one of the four mirror offsets,
/// a sane geometry and the `crc32c` over everything past the checksum field.
/// Position decides nothing: the anchor's own `bytenr` is what fixes where the
/// volume begins.
#[must_use]
pub fn from_superblock(raw: &[u8], anchor: ByteOffset) -> Option<Anchored> {
    let head = SuperBlock::parse(raw)?;
    let volume_offset = anchor.get().checked_sub(head.bytenr)?;
    Some(Anchored {
        volume_offset: ByteOffset::new(volume_offset),
        sector_bytes: head.sector_bytes,
        total_bytes: head.device_bytes,
    })
}

/// The superblock fields every walk needs.
#[derive(Clone, Copy)]
struct SuperBlock {
    bytenr: u64,
    generation: u64,
    sector_bytes: u64,
    node_bytes: u64,
    device_bytes: u64,
    devid: u64,
    tree_root: u64,
    chunk_root: u64,
    sys_chunk_bytes: usize,
    fsid: [u8; 16],
}

impl SuperBlock {
    /// Offsets within the superblock. Source: `struct btrfs_super_block`.
    const CSUM_AT: usize = 0;
    const FSID_AT: usize = 32;
    const BYTENR_AT: usize = 48;
    const GENERATION_AT: usize = 72;
    const TREE_ROOT_AT: usize = 80;
    const CHUNK_ROOT_AT: usize = 88;
    const SECTORSIZE_AT: usize = 144;
    const NODESIZE_AT: usize = 148;
    const SYS_CHUNK_SIZE_AT: usize = 160;
    const CSUM_TYPE_AT: usize = 196;
    const DEV_ITEM_AT: usize = 201;
    const SYS_CHUNK_ARRAY_AT: usize = 811;
    const SYS_CHUNK_ARRAY_BYTES: usize = 2048;
    const SUPER_ROOTS_AT: usize = 2859;

    fn parse(raw: &[u8]) -> Option<Self> {
        if u64_le(raw, 64)? != SUPERBLOCK_MAGIC {
            return None;
        }
        if u16_le(raw, Self::CSUM_TYPE_AT)? != CSUM_TYPE_CRC32C {
            return None;
        }
        // The seal covers everything after the checksum field itself.
        let body = raw.get(Self::FSID_AT..SUPERBLOCK_BYTES)?;
        if crc32c(body) != u32_le(raw, Self::CSUM_AT)? {
            return None;
        }
        let bytenr = u64_le(raw, Self::BYTENR_AT)?;
        if !MIRROR_OFFSETS.contains(&bytenr) {
            return None;
        }
        let sector_bytes = u64::from(u32_le(raw, Self::SECTORSIZE_AT)?);
        let node_bytes = u64::from(u32_le(raw, Self::NODESIZE_AT)?);
        if !(512..=MAX_NODE_BYTES).contains(&sector_bytes)
            || !sector_bytes.is_power_of_two()
            || !(sector_bytes..=MAX_NODE_BYTES).contains(&node_bytes)
            || !node_bytes.is_power_of_two()
        {
            return None;
        }
        let sys_chunk_bytes = usize::try_from(u32_le(raw, Self::SYS_CHUNK_SIZE_AT)?).ok()?;
        if sys_chunk_bytes > Self::SYS_CHUNK_ARRAY_BYTES {
            return None;
        }
        let mut fsid = [0_u8; 16];
        fsid.copy_from_slice(raw.get(Self::FSID_AT..Self::FSID_AT + 16)?);
        Some(Self {
            bytenr,
            generation: u64_le(raw, Self::GENERATION_AT)?,
            sector_bytes,
            node_bytes,
            // `dev_item.total_bytes` is this device's share; the superblock's
            // own `total_bytes` counts a whole multi-device pool, which would
            // claim past the medium being scanned.
            device_bytes: u64_le(raw, Self::DEV_ITEM_AT + 8)?,
            devid: u64_le(raw, Self::DEV_ITEM_AT)?,
            tree_root: u64_le(raw, Self::TREE_ROOT_AT)?,
            chunk_root: u64_le(raw, Self::CHUNK_ROOT_AT)?,
            sys_chunk_bytes,
            fsid,
        })
    }

    /// The historical root sets in `super_roots`, newest first.
    ///
    /// Source: four `btrfs_root_backup` of 168 bytes; `tree_root` at 0 and its
    /// generation at 8 are the only fields a diff needs, since the root tree
    /// names every subvolume of that generation.
    fn backups(raw: &[u8]) -> Vec<Backup> {
        /// Bytes of one `btrfs_root_backup`. Source: the struct.
        const BACKUP_BYTES: usize = 168;
        /// Historical root sets the superblock retains. Source:
        /// `BTRFS_NUM_BACKUP_ROOTS`.
        const BACKUP_COUNT: usize = 4;

        let mut out = Vec::with_capacity(BACKUP_COUNT);
        for index in 0..BACKUP_COUNT {
            let at = Self::SUPER_ROOTS_AT + index * BACKUP_BYTES;
            let (Some(tree_root), Some(generation), Some(fs_root)) =
                (u64_le(raw, at), u64_le(raw, at + 8), u64_le(raw, at + 48))
            else {
                break;
            };
            if tree_root == 0 {
                continue;
            }
            out.push(Backup {
                generation,
                tree_root,
                fs_root,
            });
        }
        out.sort_by_key(|backup| backup.generation);
        out.dedup_by_key(|backup| backup.generation);
        out
    }
}

/// One historical root set retained in the superblock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backup {
    /// Transaction generation this set belongs to (higher is newer).
    pub generation: u64,
    /// Root-tree address of that generation.
    pub tree_root: u64,
    /// Filesystem-tree address of that generation.
    pub fs_root: u64,
}

/// One mapping from a logical range to physical bytes on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chunk {
    logical: u64,
    length: u64,
    physical: u64,
}

/// A btrfs volume located on the medium, with its chunk map resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Btrfs {
    /// Absolute byte offset of the volume start.
    pub volume_offset: ByteOffset,
    /// Bytes in the unit data extents are allocated in.
    pub sector_bytes: u64,
    /// Bytes per B-tree block.
    pub node_bytes: u64,
    /// Bytes this device contributes to the filesystem.
    pub total_bytes: u64,
    /// Transaction generation of the newest superblock that validated.
    pub generation: u64,
    /// Root-tree address of that generation.
    pub tree_root: u64,
    /// Historical root sets the superblock retains, oldest first.
    pub backups: Vec<Backup>,
    /// Logical-to-physical map, from `sys_chunk_array` and the chunk tree.
    chunks: Vec<Chunk>,
    /// Volume identity, the gate the orphan sweep opens with.
    fsid: [u8; 16],
    /// The device this scan sees; only stripes carrying it are mapped.
    devid: u64,
}

impl Btrfs {
    /// Reads the superblock at `volume_offset` and builds the chunk map.
    ///
    /// Every mirror is read and the newest that validates wins, so a volume
    /// whose primary superblock a later format overwrote is still opened from
    /// the copy 64 MiB in.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn open<R: Read + Seek>(
        src: &mut R,
        volume_offset: ByteOffset,
    ) -> Result<Option<Self>, FsError> {
        let mut buf = Vec::new();
        // The winning copy's bytes are kept, not just its parsed fields: the
        // `sys_chunk_array` that bootstraps the chunk map lives in them, and
        // the loop below goes on to read the remaining mirrors into `buf`.
        let mut best: Option<(SuperBlock, Vec<Backup>, Vec<u8>)> = None;
        for mirror in MIRROR_OFFSETS {
            let Some(at) = volume_offset.checked_add(mirror) else {
                break;
            };
            if !read_at(src, at.get(), SUPERBLOCK_BYTES, &mut buf)? {
                continue;
            }
            let Some(head) = SuperBlock::parse(&buf) else {
                continue;
            };
            if head.bytenr != mirror {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(found, _, _)| head.generation > found.generation)
            {
                best = Some((head, SuperBlock::backups(&buf), buf.clone()));
            }
        }
        let Some((head, backups, raw)) = best else {
            return Ok(None);
        };

        let mut volume = Self {
            volume_offset,
            sector_bytes: head.sector_bytes,
            node_bytes: head.node_bytes,
            total_bytes: head.device_bytes,
            generation: head.generation,
            tree_root: head.tree_root,
            backups,
            chunks: Vec::new(),
            fsid: head.fsid,
            devid: head.devid,
        };
        volume.bootstrap_chunks(&raw, &head);
        volume.load_chunk_tree(src, head.chunk_root)?;
        Ok(Some(volume))
    }

    /// Absolute byte offset of a logical address, through the chunk map.
    ///
    /// `None` when no mapped chunk covers it — an unmapped striped profile, or
    /// an address a corrupt tree invented.
    fn physical(&self, logical: u64) -> Option<u64> {
        let chunk = self.chunks.iter().find(|chunk| {
            logical
                .checked_sub(chunk.logical)
                .is_some_and(|delta| delta < chunk.length)
        })?;
        let delta = logical.checked_sub(chunk.logical)?;
        chunk
            .physical
            .checked_add(delta)
            .and_then(|at| self.volume_offset.get().checked_add(at))
    }

    /// Seeds the chunk map from the superblock's `sys_chunk_array`.
    ///
    /// The bootstrap exists because the chunk tree's own address is logical:
    /// without these entries there is no way to read the tree that would map
    /// it.
    fn bootstrap_chunks(&mut self, raw: &[u8], head: &SuperBlock) {
        let from = SuperBlock::SYS_CHUNK_ARRAY_AT;
        let Some(array) = raw.get(from..from.saturating_add(head.sys_chunk_bytes)) else {
            return;
        };
        let mut at = 0_usize;
        while let Some(next) = self.absorb_chunk(array, at, head.devid) {
            at = next;
        }
    }

    /// Reads one (key, chunk) pair of `sys_chunk_array`, returning where the
    /// next pair begins.
    fn absorb_chunk(&mut self, array: &[u8], at: usize, devid: u64) -> Option<usize> {
        let logical = u64_le(array, at.checked_add(9)?)?;
        let chunk_at = at.checked_add(KEY_BYTES)?;
        let end = self.absorb_chunk_item(array, chunk_at, logical, devid)?;
        (end > at).then_some(end)
    }

    /// Adds one `btrfs_chunk` at `at` covering `logical`, returning its end.
    ///
    /// A striped profile is skipped rather than mapped: this scan sees one
    /// device, and part of every such stripe is on another.
    fn absorb_chunk_item(
        &mut self,
        raw: &[u8],
        at: usize,
        logical: u64,
        devid: u64,
    ) -> Option<usize> {
        let length = u64_le(raw, at)?;
        let profile = u64_le(raw, at.checked_add(24)?)?;
        let num_stripes = usize::from(u16_le(raw, at.checked_add(44)?)?).min(MAX_STRIPES);
        if num_stripes == 0 {
            return None;
        }
        let end = at
            .checked_add(CHUNK_STRIPES_AT)?
            .checked_add(num_stripes.checked_mul(STRIPE_BYTES)?)?;
        if raw.len() < end || length == 0 || self.chunks.len() >= MAX_CHUNKS {
            return None;
        }
        if profile & STRIPED_PROFILES != 0 {
            return Some(end);
        }
        // Each remaining profile stores whole copies, so the first stripe on
        // this device is the whole chunk.
        for index in 0..num_stripes {
            let stripe_at = at.checked_add(CHUNK_STRIPES_AT)? + index * STRIPE_BYTES;
            let (Some(stripe_dev), Some(physical)) =
                (u64_le(raw, stripe_at), u64_le(raw, stripe_at + 8))
            else {
                break;
            };
            if stripe_dev == devid {
                self.chunks.push(Chunk {
                    logical,
                    length,
                    physical,
                });
                break;
            }
        }
        Some(end)
    }

    /// Completes the chunk map by walking the chunk tree.
    fn load_chunk_tree<R: Read + Seek>(
        &mut self,
        src: &mut R,
        chunk_root: u64,
    ) -> Result<(), FsError> {
        let devid = self.devid;
        // Staged rather than applied in the walk, because adding a chunk needs
        // `&mut self` while the walk holds `&self`. Bounded by `MAX_CHUNKS`,
        // and a chunk item is tens of bytes (A-BOUNDED-ALLOC).
        let mut found: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut budget = MAX_NODES;
        self.walk_tree(src, chunk_root, &mut budget, &mut |_, raw| {
            for index in 0..leaf_count(raw) {
                let Some(item) = leaf_item(raw, index) else {
                    break;
                };
                if item.key_type == KEY_CHUNK_ITEM && found.len() < MAX_CHUNKS {
                    found.push((item.key_offset, item.data.to_vec()));
                }
            }
        })?;
        for (logical, data) in found {
            self.absorb_chunk_item(&data, 0, logical, devid);
        }
        Ok(())
    }
}

impl Btrfs {
    /// Whether `raw` is a tree block of this volume sitting at `logical`.
    ///
    /// Three things must agree: the volume's own `fsid`, the `crc32c` seal, and
    /// the block's own record of where it lives. The last is what makes a block
    /// found by sweeping the surface self-locating rather than assumed.
    fn node_is_valid(&self, raw: &[u8], logical: u64) -> bool {
        /// Offsets within a `btrfs_header`. Source: the struct.
        const FSID_AT: usize = 32;
        const BYTENR_AT: usize = 48;

        raw.len() == usize::try_from(self.node_bytes).unwrap_or(0)
            && raw.get(FSID_AT..FSID_AT + 16) == Some(&self.fsid[..])
            && u64_le(raw, BYTENR_AT) == Some(logical)
            && u32_le(raw, 0) == Some(crc32c(&raw[FSID_AT..]))
    }

    /// Reads the tree block at `logical` into `buf`, validating it.
    fn read_node<R: Read + Seek>(
        &self,
        src: &mut R,
        logical: u64,
        buf: &mut Vec<u8>,
    ) -> Result<bool, FsError> {
        let Some(at) = self.physical(logical) else {
            return Ok(false);
        };
        let Ok(len) = usize::try_from(self.node_bytes) else {
            return Ok(false);
        };
        if !read_at(src, at, len, buf)? {
            return Ok(false);
        }
        Ok(self.node_is_valid(buf, logical))
    }

    /// Visits every leaf of the tree rooted at `root`, with the leaf's absolute
    /// offset on the medium.
    ///
    /// The walk is bounded in depth by [`MAX_TREE_DEPTH`], in queue size by
    /// [`MAX_QUEUED`] and in blocks read by `budget`, which the caller shares
    /// across every tree of one recovery — so a crafted forest costs no more
    /// than a crafted tree (A-UNTRUSTED-ONDISK).
    fn walk_tree<R: Read + Seek, F: FnMut(u64, &[u8])>(
        &self,
        src: &mut R,
        root: u64,
        budget: &mut usize,
        visit: &mut F,
    ) -> Result<(), FsError> {
        let mut queue = vec![(root, 0_u32)];
        let mut buf = Vec::new();
        while let Some((logical, depth)) = queue.pop() {
            if depth > MAX_TREE_DEPTH || *budget == 0 {
                break;
            }
            *budget -= 1;
            if !self.read_node(src, logical, &mut buf)? {
                continue;
            }
            let level = buf.get(100).copied().unwrap_or(u8::MAX);
            if level == 0 {
                let Some(at) = self.physical(logical) else {
                    continue;
                };
                visit(at, &buf);
                continue;
            }
            for index in 0..node_count(&buf) {
                if queue.len() >= MAX_QUEUED {
                    break;
                }
                let Some(child) = key_ptr(&buf, index) else {
                    break;
                };
                queue.push((child, depth + 1));
            }
        }
        Ok(())
    }

    /// Every filesystem record of one generation's root tree.
    ///
    /// The root tree names each subvolume and snapshot, so a file deleted from
    /// any of them is reachable — not only from `FS_TREE`.
    fn records_of<R: Read + Seek>(
        &self,
        src: &mut R,
        tree_root: u64,
        budget: &mut usize,
    ) -> Result<Records, FsError> {
        let mut subvolumes = Vec::new();
        self.walk_tree(src, tree_root, budget, &mut |_, raw| {
            for index in 0..leaf_count(raw) {
                let Some(item) = leaf_item(raw, index) else {
                    break;
                };
                if item.key_type != KEY_ROOT_ITEM || !is_file_tree(item.objectid) {
                    continue;
                }
                // `btrfs_root_item.bytenr`. Source: the struct.
                if let Some(bytenr) = u64_le(item.data, 176) {
                    subvolumes.push((item.objectid, bytenr));
                }
            }
        })?;
        subvolumes.sort_unstable();
        subvolumes.dedup();

        let mut records = Records::default();
        for (root, bytenr) in subvolumes {
            self.absorb_tree(src, root, bytenr, budget, &mut records)?;
        }
        Ok(records)
    }

    /// Absorbs one filesystem tree's inode, name and extent records.
    fn absorb_tree<R: Read + Seek>(
        &self,
        src: &mut R,
        root: u64,
        bytenr: u64,
        budget: &mut usize,
        records: &mut Records,
    ) -> Result<(), FsError> {
        self.walk_tree(src, bytenr, budget, &mut |at, raw| {
            records.absorb_leaf(self, root, at, raw);
        })
    }
}

/// The files a volume's current trees still describe.
///
/// Both recovery paths are a diff against this, so it is computed once and
/// handed to each: walking the live trees is the most expensive thing either
/// does, and doing it twice would double the stage that already costs the most.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    ids: BTreeSet<(u64, u64)>,
}

impl Btrfs {
    /// Reads what the volume's current trees describe.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn live<R: Read + Seek>(&self, src: &mut R) -> Result<Live, FsError> {
        let mut budget = MAX_NODES;
        let records = self.records_of(src, self.tree_root, &mut budget)?;
        Ok(Live {
            ids: records.inodes.into_keys().collect(),
        })
    }

    /// Reports files an older generation's trees describe and the newest does
    /// not, with their extents.
    ///
    /// The superblock retains four historical root sets; each is a complete,
    /// checksummed tree the filesystem itself kept for recovery, so what one of
    /// them still names and the current generation has dropped is a deleted
    /// file — tier `FsMetadata`.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; corrupt trees yield fewer records, never an
    /// error.
    pub fn recover_deleted<R: Read + Seek>(
        &self,
        src: &mut R,
        live: &Live,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let mut budget = MAX_NODES;
        let mut found = Vec::new();
        for backup in self.backups.iter().rev() {
            if backup.tree_root == self.tree_root || budget == 0 {
                continue;
            }
            let past = self.records_of(src, backup.tree_root, &mut budget)?;
            past.emit_missing(&live.ids, Confidence::FsMetadata, &mut found);
        }
        Ok(found)
    }

    /// Sweeps `range` for tree leaves no live root reaches, and reports the
    /// files they still describe.
    ///
    /// Copy-on-write frees the leaf that described a deleted file rather than
    /// overwriting it, so leaves of older generations survive on the surface
    /// long after the backup-root ring has rotated past them. A block is
    /// accepted only when the volume's `fsid`, the `crc32c` seal and the
    /// block's own record of its address all agree, which is what keeps a sweep
    /// of a whole volume from reporting coincidences.
    ///
    /// The extent such a leaf names may since have been reallocated, so the
    /// tier is `JournalResidue` rather than `FsMetadata`.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn orphan_scan<R: Read + Seek>(
        &self,
        src: &mut R,
        live: &Live,
        range: ByteRange,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let Ok(len) = usize::try_from(self.node_bytes) else {
            return Ok(Vec::new());
        };
        let mut stale = Records::default();
        let mut buf = Vec::new();
        let mut at = range.start.get();
        let end = range.end_saturating().get();
        let mut steps = 0_u64;
        while at < end && steps < MAX_ORPHAN_BLOCKS {
            steps += 1;
            let next = at.saturating_add(self.node_bytes);
            if !read_at(src, at, len, &mut buf)? {
                at = next;
                continue;
            }
            // The `fsid` is the cheap gate; the seal and the self-address are
            // what actually decide.
            if buf.get(32..48) == Some(&self.fsid[..])
                && let Some(logical) = u64_le(&buf, 48)
                && self.physical(logical) == Some(at)
                && self.node_is_valid(&buf, logical)
                && buf.get(100) == Some(&0)
            {
                // A stale leaf carries the objectid of the tree it belonged to.
                let owner = u64_le(&buf, 88).unwrap_or(FS_TREE_OBJECTID);
                if is_file_tree(owner) {
                    stale.absorb_leaf(self, owner, at, &buf);
                }
            }
            at = next;
        }

        let mut found = Vec::new();
        stale.emit_missing(&live.ids, Confidence::JournalResidue, &mut found);
        Ok(found)
    }
}

/// Whether `objectid` names a tree that holds file records.
///
/// `FS_TREE` plus the subvolume and snapshot range; everything else is an
/// internal tree (extent, chunk, device, checksum, quota, uuid) whose items are
/// not files.
fn is_file_tree(objectid: u64) -> bool {
    objectid == FS_TREE_OBJECTID || (FIRST_FREE_OBJECTID..=LAST_FREE_OBJECTID).contains(&objectid)
}

/// One file as its tree describes it.
struct FileRecord {
    size: u64,
    timestamps: Timestamps,
    regular: bool,
    /// Extents in file order, keyed by their offset within the file.
    parts: BTreeMap<u64, ByteRange>,
    /// False once an extent could not be resolved or is compressed: the file's
    /// bytes are then not what its extents point at, so it is not reported.
    usable: bool,
}

impl Default for FileRecord {
    /// A file is usable until one of its extents says otherwise.
    fn default() -> Self {
        Self {
            size: 0,
            timestamps: Timestamps::default(),
            regular: false,
            parts: BTreeMap::new(),
            usable: true,
        }
    }
}

/// Everything one generation's trees describe, keyed by (tree, inode).
///
/// An inode number is unique only within its own subvolume, so the tree it came
/// from is part of its identity; keying on the number alone would make one
/// subvolume's live file hide another's deleted one.
#[derive(Default)]
struct Records {
    inodes: BTreeMap<(u64, u64), FileRecord>,
    names: BTreeMap<(u64, u64), String>,
}

impl Records {
    /// Reads one leaf's inode, name and extent records.
    fn absorb_leaf(&mut self, fs: &Btrfs, tree: u64, at: u64, raw: &[u8]) {
        for index in 0..leaf_count(raw) {
            let Some(item) = leaf_item(raw, index) else {
                break;
            };
            let key = (tree, item.objectid);
            match item.key_type {
                KEY_INODE_ITEM if self.inodes.len() < MAX_RECORDS => {
                    self.absorb_inode(key, item.data);
                }
                KEY_INODE_REF if self.names.len() < MAX_RECORDS => {
                    if let Some(name) = inode_ref_name(item.data) {
                        self.names.entry(key).or_insert(name);
                    }
                }
                KEY_EXTENT_DATA if self.inodes.len() < MAX_RECORDS => {
                    let record = self.inodes.entry(key).or_default();
                    absorb_extent(record, fs, at, &item);
                }
                _ => {}
            }
        }
    }

    /// Reads a `btrfs_inode_item`. Source: the struct's field offsets.
    fn absorb_inode(&mut self, key: (u64, u64), data: &[u8]) {
        let (Some(size), Some(mode), Some(otime), Some(mtime)) = (
            u64_le(data, 16),
            u32_le(data, 52),
            u64_le(data, 148),
            u64_le(data, 136),
        ) else {
            return;
        };
        let record = self.inodes.entry(key).or_default();
        record.size = size;
        record.regular = mode & MODE_FORMAT_MASK == MODE_REGULAR;
        record.timestamps = Timestamps {
            created: unix_time(otime),
            modified: unix_time(mtime),
        };
    }

    /// Appends every file this set describes that `live` does not.
    fn emit_missing(
        self,
        live: &BTreeSet<(u64, u64)>,
        confidence: Confidence,
        out: &mut Vec<DeletedFile>,
    ) {
        for (key, record) in self.inodes {
            if live.contains(&key) || !record.regular || !record.usable || record.size == 0 {
                continue;
            }
            let extents: Vec<ByteRange> = record.parts.into_values().collect();
            if extents.is_empty() {
                continue;
            }
            out.push(DeletedFile {
                name: self.names.get(&key).cloned(),
                timestamps: record.timestamps,
                size: record.size,
                extents,
                fs: FsKind::Btrfs,
                confidence,
                source_object: Some(key.1),
            });
        }
    }
}

/// Adds one `btrfs_file_extent_item` to `record`.
///
/// A compressed or encrypted extent holds bytes that are not the file's, and an
/// extent whose chunk does not map cannot be placed at all; either makes the
/// whole file unusable, because a missing piece would splice unrelated bytes
/// into the middle of a photograph (`A-CONFIDENCE-HONEST`).
fn absorb_extent(record: &mut FileRecord, fs: &Btrfs, leaf_at: u64, item: &Item<'_>) {
    /// Field offsets within `btrfs_file_extent_item`. Source: the struct.
    const RAM_BYTES_AT: usize = 8;
    const COMPRESSION_AT: usize = 16;
    const ENCRYPTION_AT: usize = 17;
    const TYPE_AT: usize = 20;
    const DISK_BYTENR_AT: usize = 21;
    const OFFSET_AT: usize = 37;
    const NUM_BYTES_AT: usize = 45;

    let (data, file_offset) = (item.data, item.key_offset);
    let (Some(&compression), Some(&encryption), Some(&kind)) = (
        data.get(COMPRESSION_AT),
        data.get(ENCRYPTION_AT),
        data.get(TYPE_AT),
    ) else {
        record.usable = false;
        return;
    };
    if compression != 0 || encryption != 0 || record.parts.len() >= MAX_EXTENTS_PER_FILE {
        record.usable = false;
        return;
    }

    if kind == EXTENT_INLINE {
        // The payload is inside this leaf, so its bytes are already located —
        // the same shape as a resident NTFS `$DATA`.
        let Some(len) = u64_le(data, RAM_BYTES_AT) else {
            record.usable = false;
            return;
        };
        let Some(payload) = data.get(EXTENT_INLINE_DATA_AT..) else {
            record.usable = false;
            return;
        };
        let Some(start) = item
            .data_at
            .checked_add(EXTENT_INLINE_DATA_AT)
            .and_then(|delta| u64::try_from(delta).ok())
            .and_then(|delta| leaf_at.checked_add(delta))
        else {
            record.usable = false;
            return;
        };
        let held = u64::try_from(payload.len()).unwrap_or(0);
        record.parts.insert(
            file_offset,
            ByteRange::new(ByteOffset::new(start), len.min(held)),
        );
        return;
    }
    if kind != EXTENT_REGULAR && kind != EXTENT_PREALLOC {
        record.usable = false;
        return;
    }

    let (Some(disk_bytenr), Some(offset), Some(num_bytes)) = (
        u64_le(data, DISK_BYTENR_AT),
        u64_le(data, OFFSET_AT),
        u64_le(data, NUM_BYTES_AT),
    ) else {
        record.usable = false;
        return;
    };
    // A hole: `disk_bytenr` zero means the range reads as zeros and nothing on
    // the medium holds it.
    if disk_bytenr == 0 || num_bytes == 0 {
        record.usable = false;
        return;
    }
    let (Some(logical), Some(_)) = (
        disk_bytenr.checked_add(offset),
        num_bytes.checked_add(offset),
    ) else {
        record.usable = false;
        return;
    };
    let Some(start) = fs.physical(logical) else {
        record.usable = false;
        return;
    };
    record.parts.insert(
        file_offset,
        ByteRange::new(ByteOffset::new(start), num_bytes),
    );
}

/// Items in a leaf, capped by what its size can actually hold.
fn leaf_count(raw: &[u8]) -> usize {
    let declared = u32_le(raw, 96).map_or(0, |count| usize::try_from(count).unwrap_or(0));
    let capacity = raw.len().saturating_sub(HEADER_BYTES) / ITEM_BYTES;
    declared.min(capacity)
}

/// Pointers in an interior node, capped by what its size can actually hold.
fn node_count(raw: &[u8]) -> usize {
    let declared = u32_le(raw, 96).map_or(0, |count| usize::try_from(count).unwrap_or(0));
    let capacity = raw.len().saturating_sub(HEADER_BYTES) / KEY_PTR_BYTES;
    declared.min(capacity)
}

/// One leaf item: its key's objectid, type and offset, the byte at which its
/// payload begins within the block, and the payload.
fn leaf_item(raw: &[u8], index: usize) -> Option<Item<'_>> {
    let at = HEADER_BYTES.checked_add(index.checked_mul(ITEM_BYTES)?)?;
    let objectid = u64_le(raw, at)?;
    let key_type = *raw.get(at.checked_add(8)?)?;
    let key_offset = u64_le(raw, at.checked_add(9)?)?;
    let data_at = HEADER_BYTES
        .checked_add(usize::try_from(u32_le(raw, at.checked_add(KEY_BYTES)?)?).ok()?)?;
    let data_len = usize::try_from(u32_le(raw, at.checked_add(KEY_BYTES + 4)?)?).ok()?;
    let data = raw.get(data_at..data_at.checked_add(data_len)?)?;
    Some(Item {
        objectid,
        key_type,
        key_offset,
        data_at,
        data,
    })
}

/// One item of a leaf, as read off the block.
struct Item<'a> {
    objectid: u64,
    key_type: u8,
    key_offset: u64,
    /// Byte at which `data` begins within the block, so an inline extent can
    /// state where its bytes are on the medium without pointer arithmetic.
    data_at: usize,
    data: &'a [u8],
}

/// The child address of one interior-node pointer.
fn key_ptr(raw: &[u8], index: usize) -> Option<u64> {
    let at = HEADER_BYTES.checked_add(index.checked_mul(KEY_PTR_BYTES)?)?;
    u64_le(raw, at.checked_add(KEY_BYTES)?)
}

/// The name in a `btrfs_inode_ref`, capped at [`MAX_NAME_BYTES`].
fn inode_ref_name(data: &[u8]) -> Option<String> {
    let len = usize::from(u16_le(data, 8)?).min(MAX_NAME_BYTES);
    let raw = data.get(10..10usize.checked_add(len)?)?;
    let name = std::str::from_utf8(raw).ok()?;
    (!name.is_empty() && !name.contains('\0')).then(|| name.to_owned())
}

/// Unix seconds to `SystemTime`; zero means unset.
fn unix_time(secs: u64) -> Option<SystemTime> {
    if secs == 0 {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}
