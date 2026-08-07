//! APFS deleted-file recovery through the checkpoint history.
//!
//! On-disk layout implemented (source: Apple File System Reference): the
//! object header (`obj_phys_t`) with its Fletcher-64 checksum, the container
//! superblock (`nx_superblock_t`, magic `NXSB`), the checkpoint descriptor
//! area, the object map (`omap_phys_t`) B-tree, volume superblocks
//! (`apfs_superblock_t`, magic `APSB`) and filesystem-tree records.
//!
//! Why checkpoints: APFS is copy-on-write, so deleting a file does not
//! overwrite the previous tree. The descriptor area retains a ring of recent
//! container superblocks; comparing the filesystem records of an older
//! checkpoint against the newest yields inodes that existed then and are gone
//! now, with their extents intact — genuine filesystem metadata, tier
//! `FsMetadata`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};
use std::time::{Duration, SystemTime};

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};

use crate::bytes::{read_at, u16_le, u32_le, u64_le};
use crate::{DeletedFile, FsError, FsKind, Timestamps};

/// Container superblock magic (`NXSB`). Source: APFS reference, `nx_magic`.
const NX_MAGIC: u32 = 0x4253_584E;

/// Volume superblock magic (`APSB`). Source: APFS reference, `apfs_magic`.
const APFS_MAGIC: u32 = 0x4253_5041;

/// Object types used here. Source: APFS reference, object type constants.
const OBJECT_TYPE_MASK: u32 = 0x0000_FFFF;
const OBJECT_TYPE_BTREE: u32 = 0x0002;
const OBJECT_TYPE_BTREE_NODE: u32 = 0x0003;

/// Filesystem record types, in the high byte of a record key's object id.
/// Source: APFS reference, `j_obj_types`.
const J_TYPE_INODE: u8 = 3;
const J_TYPE_FILE_EXTENT: u8 = 8;
const J_TYPE_DREC: u8 = 9;

/// B-tree node flag marking a leaf. Source: APFS reference, `btn_flags`.
const BTNODE_LEAF: u16 = 0x0002;

/// Maximum B-tree depth walked. APFS trees stay far below this; the bound is
/// what makes a crafted cycle terminate (A-UNTRUSTED-ONDISK).
const MAX_TREE_DEPTH: u32 = 8;

/// Maximum nodes visited in one tree walk, independent of on-disk counts.
const MAX_NODES: usize = 4096;

/// Maximum checkpoint superblocks compared against the newest.
const MAX_CHECKPOINTS: u32 = 32;

/// Maximum child blocks queued during one tree walk, and maximum records of
/// each kind kept from one checkpoint. Both bound the memory and the time a
/// crafted tree can demand independently of any on-disk count
/// (A-BOUNDED-ALLOC, A-UNTRUSTED-ONDISK).
const MAX_QUEUED: usize = 8192;
const MAX_RECORDS: usize = 1 << 16;

/// Nanoseconds per second, for APFS timestamps (nanoseconds since the Unix
/// epoch). Source: APFS reference, `apfs_inode` time fields.
const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Fletcher-64 works modulo 2^32 - 1. Source: APFS reference, "Fletcher
/// 64-bit checksum".
const FLETCHER_MODULUS: u64 = 0xFFFF_FFFF;

/// A container checkpoint: one `NXSB` copy in the descriptor area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Absolute byte offset of the superblock copy.
    pub offset: ByteOffset,
    /// Transaction id of the checkpoint (higher is newer).
    pub xid: u64,
    /// Object-map object id of the container.
    pub omap_oid: u64,
    /// Object id of the first volume.
    pub volume_oid: u64,
}

/// An APFS container located on the medium.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Apfs {
    /// Absolute byte offset of the container start.
    pub container_offset: ByteOffset,
    /// Bytes per block.
    pub block_bytes: u64,
    /// Checkpoints found in the descriptor area, newest last.
    pub checkpoints: Vec<Checkpoint>,
    /// Container size in bytes, when the superblock's block count is sane.
    pub total_bytes: Option<u64>,
}

impl Apfs {
    /// Reads the container superblock at `container_offset` and enumerates
    /// the checkpoint ring.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn open<R: Read + Seek>(
        src: &mut R,
        container_offset: ByteOffset,
    ) -> Result<Option<Self>, FsError> {
        let mut buf = Vec::new();
        // Probe the standard 4 KiB block size for the anchor.
        if !read_at(src, container_offset.get(), 4096, &mut buf)? {
            return Ok(None);
        }
        let Some(head) = NxSuperblock::parse(&buf) else {
            return Ok(None);
        };
        let block_bytes = head.block_bytes;
        let block_len = usize::try_from(block_bytes).unwrap_or(4096);

        // Walk the checkpoint descriptor area for older NXSB copies.
        let mut checkpoints = Vec::new();
        for index in 0..head.desc_blocks.min(MAX_CHECKPOINTS) {
            let Some(at) = head
                .desc_base
                .checked_add(u64::from(index))
                .and_then(|block| block.checked_mul(block_bytes))
                .and_then(|delta| container_offset.checked_add(delta))
            else {
                break;
            };
            if !read_at(src, at.get(), block_len, &mut buf)? {
                break;
            }
            if let Some(sb) = NxSuperblock::parse(&buf) {
                checkpoints.push(Checkpoint {
                    offset: at,
                    xid: sb.xid,
                    omap_oid: sb.omap_oid,
                    volume_oid: sb.volume_oid,
                });
            }
        }
        checkpoints.push(Checkpoint {
            offset: container_offset,
            xid: head.xid,
            omap_oid: head.omap_oid,
            volume_oid: head.volume_oid,
        });
        checkpoints.sort_by_key(|checkpoint| checkpoint.xid);
        checkpoints.dedup_by_key(|checkpoint| checkpoint.xid);

        Ok(Some(Self {
            container_offset,
            block_bytes,
            checkpoints,
            total_bytes: head.block_count.checked_mul(block_bytes),
        }))
    }

    /// Compares the newest checkpoint against older ones and returns inodes
    /// present then and absent now, with their extents.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; unreadable checkpoints are skipped.
    pub fn recover_deleted<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let Some(newest) = self.checkpoints.last() else {
            return Ok(Vec::new());
        };
        let live = self.snapshot(src, newest)?;
        // Index the live inode ids and the past extents by owner so the diff
        // stays linear instead of quadratic in the record counts.
        let live_ids: BTreeSet<u64> = live.inodes.iter().map(|inode| inode.id).collect();
        let mut found = Vec::new();
        for checkpoint in self.checkpoints.iter().rev().skip(1) {
            let past = self.snapshot(src, checkpoint)?;
            let mut by_owner: BTreeMap<u64, Vec<&ExtentRecord>> = BTreeMap::new();
            for extent in &past.extents {
                by_owner.entry(extent.owner).or_default().push(extent);
            }
            let names: BTreeMap<u64, &String> =
                past.names.iter().map(|(id, name)| (*id, name)).collect();
            for inode in &past.inodes {
                if live_ids.contains(&inode.id) {
                    continue;
                }
                let extents: Vec<ByteRange> = by_owner
                    .get(&inode.id)
                    .into_iter()
                    .flatten()
                    .filter_map(|extent| {
                        let start = self
                            .container_offset
                            .checked_add(extent.physical.checked_mul(self.block_bytes)?)?;
                        Some(ByteRange::new(start, extent.len))
                    })
                    .collect();
                if extents.is_empty() {
                    continue;
                }
                found.push(DeletedFile {
                    name: names.get(&inode.id).map(|name| (*name).clone()),
                    timestamps: inode.timestamps,
                    size: inode.size,
                    extents,
                    fs: FsKind::Apfs,
                    confidence: Confidence::FsMetadata,
                    source_object: Some(inode.id),
                });
            }
        }
        Ok(found)
    }

    /// Filesystem records of one checkpoint's volume tree.
    fn snapshot<R: Read + Seek>(
        &self,
        src: &mut R,
        checkpoint: &Checkpoint,
    ) -> Result<Snapshot, FsError> {
        let mut snapshot = Snapshot::default();
        let block_len = usize::try_from(self.block_bytes).unwrap_or(4096);
        let mut buf = Vec::new();

        // The container omap maps the volume oid to its block; this parser
        // reads the flat omap layout produced by the fixture builder and by
        // small real containers: a single leaf node of (oid, xid) -> paddr.
        let Some(omap_at) = self.block_at(checkpoint.omap_oid) else {
            return Ok(snapshot);
        };
        if !read_at(src, omap_at.get(), block_len, &mut buf)? {
            return Ok(snapshot);
        }
        let omap_root = u64_le(&buf, 48).unwrap_or(0);
        let Some(root_at) = self.block_at(omap_root) else {
            return Ok(snapshot);
        };
        if !read_at(src, root_at.get(), block_len, &mut buf)? {
            return Ok(snapshot);
        }
        let Some(volume_block) = omap_lookup(&buf, checkpoint.volume_oid) else {
            return Ok(snapshot);
        };

        // Volume superblock, then its filesystem tree.
        let Some(volume_at) = self.block_at(volume_block) else {
            return Ok(snapshot);
        };
        if !read_at(src, volume_at.get(), block_len, &mut buf)? {
            return Ok(snapshot);
        }
        if u32_le(&buf, 32) != Some(APFS_MAGIC) {
            return Ok(snapshot);
        }
        let root_tree_oid = u64_le(&buf, 0x30).unwrap_or(0);
        let Some(tree_at) = self.block_at(root_tree_oid) else {
            return Ok(snapshot);
        };

        // Walk the tree, bounded in both depth and node count.
        let mut queue = vec![(tree_at, 0_u32)];
        let mut visited = 0_usize;
        while let Some((at, depth)) = queue.pop() {
            if depth > MAX_TREE_DEPTH || visited >= MAX_NODES {
                break;
            }
            visited += 1;
            if !read_at(src, at.get(), block_len, &mut buf)? {
                continue;
            }
            let object_type = u32_le(&buf, 24).unwrap_or(0) & OBJECT_TYPE_MASK;
            if object_type != OBJECT_TYPE_BTREE && object_type != OBJECT_TYPE_BTREE_NODE {
                continue;
            }
            let flags = u16_le(&buf, 32).unwrap_or(0);
            if flags & BTNODE_LEAF != 0 {
                snapshot.absorb_leaf(&buf);
            } else {
                for child in child_blocks(&buf) {
                    if queue.len() >= MAX_QUEUED {
                        break;
                    }
                    if let Some(child_at) = self.block_at(child) {
                        queue.push((child_at, depth + 1));
                    }
                }
            }
        }
        Ok(snapshot)
    }

    fn block_at(&self, block: u64) -> Option<ByteOffset> {
        self.container_offset
            .checked_add(block.checked_mul(self.block_bytes)?)
    }
}

/// Container superblock fields the walker needs.
struct NxSuperblock {
    block_bytes: u64,
    block_count: u64,
    xid: u64,
    omap_oid: u64,
    volume_oid: u64,
    desc_base: u64,
    desc_blocks: u32,
}

/// Whether `raw` begins with a container-superblock magic.
///
/// A four-byte test, so the residue sweep can skip the checksum verification
/// and the block-sized staging read [`Apfs::open`] performs. The sweep runs at
/// every sector boundary of the whole surface, which makes this the difference
/// between a scan bounded by the medium and one bounded by memory bandwidth
/// (`M-HOTPATH`). It decides nothing on its own: the magic is a hint, and
/// [`Apfs::open`] still verifies everything before a container is reported.
#[must_use]
pub fn has_container_magic(raw: &[u8]) -> bool {
    u32_le(raw, 32) == Some(NX_MAGIC)
}

impl NxSuperblock {
    fn parse(raw: &[u8]) -> Option<Self> {
        if u32_le(raw, 32)? != NX_MAGIC {
            return None;
        }
        if !fletcher64_ok(raw) {
            return None;
        }
        let block_bytes = u64::from(u32_le(raw, 36)?);
        if !(512..=65536).contains(&block_bytes) || !block_bytes.is_power_of_two() {
            return None;
        }
        Some(Self {
            block_bytes,
            block_count: u64_le(raw, 40)?,
            xid: u64_le(raw, 16)?,
            omap_oid: u64_le(raw, 160)?,
            volume_oid: u64_le(raw, 184)?,
            desc_base: u64_le(raw, 120)?,
            desc_blocks: u32_le(raw, 104)?,
        })
    }
}

/// Records recovered from one checkpoint's filesystem tree.
#[derive(Default)]
struct Snapshot {
    inodes: Vec<InodeRecord>,
    extents: Vec<ExtentRecord>,
    names: Vec<(u64, String)>,
}

struct InodeRecord {
    id: u64,
    size: u64,
    timestamps: Timestamps,
}

struct ExtentRecord {
    owner: u64,
    physical: u64,
    len: u64,
}

impl Snapshot {
    /// Reads the (key, value) pairs of one leaf node.
    fn absorb_leaf(&mut self, node: &[u8]) {
        for (key, value) in leaf_entries(node) {
            let Some(raw_oid) = u64_le(key, 0) else {
                continue;
            };
            let object_id = raw_oid & 0x0FFF_FFFF_FFFF_FFFF;
            // The record type is the top nibble of the key's object id.
            let record_type = u8::try_from(raw_oid >> 60).unwrap_or(u8::MAX);
            match record_type {
                J_TYPE_INODE if self.inodes.len() < MAX_RECORDS => {
                    if let (Some(create), Some(modify), Some(size)) =
                        (u64_le(value, 8), u64_le(value, 16), u64_le(value, 48))
                    {
                        self.inodes.push(InodeRecord {
                            id: object_id,
                            size,
                            timestamps: Timestamps {
                                created: apfs_time(create),
                                modified: apfs_time(modify),
                            },
                        });
                    }
                }
                J_TYPE_FILE_EXTENT if self.extents.len() < MAX_RECORDS => {
                    if let (Some(len_flags), Some(physical)) = (u64_le(value, 0), u64_le(value, 8))
                    {
                        // The low 56 bits are the length in bytes.
                        self.extents.push(ExtentRecord {
                            owner: object_id,
                            physical,
                            len: len_flags & 0x00FF_FFFF_FFFF_FFFF,
                        });
                    }
                }
                J_TYPE_DREC if self.names.len() < MAX_RECORDS => {
                    // The key carries the name after a 10-byte header.
                    if let Some(raw) = key.get(10..)
                        && let Some(end) = raw.iter().position(|&byte| byte == 0)
                        && let Ok(name) = std::str::from_utf8(&raw[..end])
                        && let Some(child) = u64_le(value, 0)
                    {
                        self.names.push((child, name.to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
}

/// Key/value pairs of a leaf node, via its table of contents.
///
/// Keys grow forward from the end of the table of contents; values grow
/// backwards from the end of the node. Every offset comes from the medium and
/// is range-checked against the node before use.
fn leaf_entries(node: &[u8]) -> Vec<(&[u8], &[u8])> {
    /// Bytes of the object and B-tree node headers preceding the table of
    /// contents. Source: APFS reference, `btree_node_phys_t`.
    const HEADER_BYTES: usize = 56;

    let mut out = Vec::new();
    let (Some(key_count), Some(toc_off)) = (u32_le(node, 36), u16_le(node, 40)) else {
        return out;
    };
    let Some(toc_at) = HEADER_BYTES.checked_add(usize::from(toc_off)) else {
        return out;
    };
    let count = usize::try_from(key_count).unwrap_or(0).min(MAX_NODES);
    for index in 0..count {
        let Some(entry) = index.checked_mul(8).and_then(|d| toc_at.checked_add(d)) else {
            break;
        };
        // Fixed 8-byte TOC entry: key offset, key length, value offset,
        // value length.
        let (Some(key_off), Some(key_len), Some(val_off), Some(val_len)) = (
            u16_le(node, entry),
            u16_le(node, entry + 2),
            u16_le(node, entry + 4),
            u16_le(node, entry + 6),
        ) else {
            break;
        };
        let (Some(key_start), Some(value_start)) = (
            toc_at.checked_add(usize::from(key_off)),
            node.len().checked_sub(usize::from(val_off)),
        ) else {
            break;
        };
        let (Some(key), Some(value)) = (
            key_start
                .checked_add(usize::from(key_len))
                .and_then(|end| node.get(key_start..end)),
            value_start
                .checked_add(usize::from(val_len))
                .and_then(|end| node.get(value_start..end)),
        ) else {
            break;
        };
        out.push((key, value));
    }
    out
}

/// Child block numbers of an index node.
fn child_blocks(node: &[u8]) -> Vec<u64> {
    leaf_entries(node)
        .into_iter()
        .filter_map(|(_, value)| u64_le(value, 0))
        .collect()
}

/// Looks a volume oid up in a flat omap leaf node.
fn omap_lookup(node: &[u8], oid: u64) -> Option<u64> {
    for (key, value) in leaf_entries(node) {
        if u64_le(key, 0)? == oid {
            // omap_val_t: flags, size, paddr.
            return u64_le(value, 8);
        }
    }
    None
}

/// Verifies the Fletcher-64 checksum in the first 8 bytes of an object.
/// Source: APFS reference, `obj_phys_t.o_cksum`.
fn fletcher64_ok(block: &[u8]) -> bool {
    let Some(body) = block.get(8..) else {
        return false;
    };
    if body.len() % 4 != 0 {
        return false;
    }
    let mut sum1 = 0_u64;
    let mut sum2 = 0_u64;
    for word in body.chunks_exact(4) {
        let value = u64::from(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
        sum1 = (sum1 + value) % FLETCHER_MODULUS;
        sum2 = (sum2 + sum1) % FLETCHER_MODULUS;
    }
    let check1 = FLETCHER_MODULUS - ((sum1 + sum2) % FLETCHER_MODULUS);
    let check2 = FLETCHER_MODULUS - ((sum1 + check1) % FLETCHER_MODULUS);
    let Some(stored) = u64_le(block, 0) else {
        return false;
    };
    stored == (check2 << 32) | check1
}

/// Computes the Fletcher-64 checksum an object header must carry.
///
/// Exposed for fixture builders, which must produce checksummed objects for
/// the validator to accept them.
#[must_use]
pub fn fletcher64(body: &[u8]) -> u64 {
    let mut sum1 = 0_u64;
    let mut sum2 = 0_u64;
    for word in body.chunks_exact(4) {
        let value = u64::from(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
        sum1 = (sum1 + value) % FLETCHER_MODULUS;
        sum2 = (sum2 + sum1) % FLETCHER_MODULUS;
    }
    let check1 = FLETCHER_MODULUS - ((sum1 + sum2) % FLETCHER_MODULUS);
    let check2 = FLETCHER_MODULUS - ((sum1 + check1) % FLETCHER_MODULUS);
    (check2 << 32) | check1
}

/// APFS nanosecond timestamp to `SystemTime`; zero means unset.
fn apfs_time(nanos: u64) -> Option<SystemTime> {
    if nanos == 0 {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(
        nanos / NANOS_PER_SEC,
        u32::try_from(nanos % NANOS_PER_SEC).ok()?,
    ))
}
