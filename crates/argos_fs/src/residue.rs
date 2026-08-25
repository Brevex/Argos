//! Residue sweep: volume anchors left behind by earlier formats.
//!
//! A quick re-format overwrites a small metadata region and nothing else, so
//! the anchors of the *previous* filesystem survive elsewhere on the medium.
//! This sweep tests every sector boundary for NTFS boot sectors and `FILE`
//! records, ext superblocks, FAT/exFAT boot sectors, APFS container
//! superblocks and btrfs superblocks, and validates each hit by **internal
//! consistency** — never by position. Overlapping candidates are all kept; later stages decide by
//! yield.

use std::io::{Read, Seek};

use argos_core::geometry::{ByteOffset, ByteRange};

use crate::apfs::Apfs;
use crate::btrfs;
use crate::ext4::{Ext4, SUPERBLOCK_OFFSET};
use crate::fat::Fat;
use crate::ntfs::Ntfs;
use crate::read_at;
use crate::{Anchor, FsError, FsKind, Origin, Volume};

/// Sector granularity of the sweep. Volume anchors are always sector-aligned;
/// stepping by 512 bytes is what makes a whole-surface sweep affordable.
const STEP_BYTES: u64 = 512;

/// Bytes read per sweep window. 1 MiB keeps the sweep in large sequential
/// reads while the window buffer stays small.
const WINDOW_BYTES: usize = 1024 * 1024;

/// Overlap a caller must carry between consecutive [`scan_window`] buffers.
///
/// It is the largest structure a validator inspects (an APFS block), so an
/// anchor straddling a buffer boundary is still whole in the next one. Feeding
/// disjoint buffers silently loses the anchors on those boundaries.
pub const WINDOW_OVERLAP_BYTES: usize = 4096;

/// Cap on volume anchors one sweep reports.
///
/// A medium can hold a crafted superblock at every sector; without a ceiling
/// the sweep's own result set would exhaust memory before anything could be
/// reported (A-BOUNDED-ALLOC). Real media hold a handful of volumes, so a
/// sweep that reaches this has found a pattern, not a disk.
pub const MAX_VOLUMES: usize = 4096;

/// Cap on orphaned `FILE`-record regions one sweep reports. Adjacent records
/// coalesce into runs, so this bounds genuinely scattered residue.
pub const MAX_RECORD_REGIONS: usize = 65_536;

/// Everything one sweep located.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    /// Volumes found, current and residual.
    pub volumes: Vec<Volume>,
    /// Regions holding orphaned NTFS `FILE` records, in ascending order.
    /// Hand these to [`crate::ntfs::orphan_scan`] with the geometry of the
    /// volume they belong to.
    pub ntfs_records: Vec<ByteRange>,
    /// Regions holding NTFS `INDX` directory index buffers, whose slack names
    /// files a directory no longer lists.
    pub ntfs_indexes: Vec<ByteRange>,
}

impl Sweep {
    /// Appends everything `other` found; call [`Sweep::normalize`] afterwards.
    ///
    /// This is how the results of windows scanned out of order — by parallel
    /// workers, say — become one sweep.
    pub fn merge(&mut self, other: Self) {
        self.volumes.extend(other.volumes);
        self.ntfs_records.extend(other.ntfs_records);
        self.ntfs_indexes.extend(other.ntfs_indexes);
    }

    /// Orders both result lists, drops the duplicates that window overlap
    /// produces, and coalesces adjacent `FILE`-record regions into runs.
    pub fn normalize(&mut self) {
        self.volumes
            .sort_by_key(|volume| (volume.range.start, volume.range.len));
        self.volumes.dedup();

        self.ntfs_records
            .sort_by_key(|region| (region.start, region.len));
        self.ntfs_records.dedup();
        let merged = self.ntfs_records.drain(..).fold(
            Vec::<ByteRange>::new(),
            |mut runs: Vec<ByteRange>, region| {
                match runs.last_mut() {
                    // Adjacent or overlapping: extend the run to the furthest end.
                    Some(last)
                        if last.end().map(ByteOffset::get) >= Some(region.start.get())
                            && last.start <= region.start =>
                    {
                        let end = last
                            .end_saturating()
                            .get()
                            .max(region.end_saturating().get());
                        last.len = end.saturating_sub(last.start.get());
                    }
                    _ => runs.push(region),
                }
                runs
            },
        );
        self.ntfs_records = merged;
    }

    /// Re-labels as [`Origin::Current`] every volume whose start matches one
    /// of the ranges the current partition table lists.
    pub fn mark_current(&mut self, current: &[ByteRange]) {
        for volume in &mut self.volumes {
            if current
                .iter()
                .any(|range| range.start == volume.range.start)
            {
                volume.origin = Origin::Current;
            }
        }
    }
}

/// Sweeps `src` (of `len` bytes) for filesystem anchors.
///
/// Anchors matching a volume already listed in `current` are reported as
/// [`Origin::Current`]; everything else is [`Origin::Residual`].
///
/// # Errors
///
/// Fails only on I/O faults.
pub fn sweep<R: Read + Seek>(
    src: &mut R,
    len: u64,
    current: &[ByteRange],
) -> Result<Sweep, FsError> {
    let mut window = Vec::new();
    let mut found = Sweep::default();
    let mut pos = 0_u64;

    while pos < len {
        let want = usize::try_from((len - pos).min(WINDOW_BYTES as u64)).unwrap_or(WINDOW_BYTES);
        if !read_at(src, pos, want, &mut window)? {
            // A short read at the tail still yields whatever was readable.
            if window.is_empty() {
                break;
            }
        }
        if !scan_window(&window, ByteOffset::new(pos), len, &mut found) {
            break;
        }
        if pos + want as u64 >= len {
            break;
        }
        pos += (want - WINDOW_OVERLAP_BYTES) as u64;
    }

    found.mark_current(current);
    found.normalize();
    Ok(found)
}

/// Tests every sector boundary in `window` for anchors, appending to `out`.
///
/// `start` is the absolute position of `window[0]` and `medium_len` bounds
/// what an anchor may claim. Consecutive windows must overlap by
/// [`WINDOW_OVERLAP_BYTES`]; the resulting duplicates are removed by
/// [`Sweep::normalize`], which the caller runs once at the end.
///
/// Volumes are appended with [`Origin::Residual`]; [`Sweep::mark_current`]
/// re-labels the ones the current partition table also lists.
///
/// Returns `false` once [`MAX_VOLUMES`] or [`MAX_RECORD_REGIONS`] is reached,
/// meaning the sweep stopped early and its result is incomplete.
pub fn scan_window(window: &[u8], start: ByteOffset, medium_len: u64, out: &mut Sweep) -> bool {
    let step = usize::try_from(STEP_BYTES).unwrap_or(512);
    let mut at = 0_usize;
    while at + 512 <= window.len() {
        if out.volumes.len() >= MAX_VOLUMES
            || out.ntfs_records.len() >= MAX_RECORD_REGIONS
            || out.ntfs_indexes.len() >= MAX_RECORD_REGIONS
        {
            return false;
        }
        let absolute = start.get().saturating_add(at as u64);
        match anchor_at(&window[at..], absolute, medium_len) {
            Some(Anchor::Volume(_)) => {
                if let Some(volume) = volume_at(&window[at..], absolute, medium_len) {
                    out.volumes.push(volume);
                }
            }
            Some(Anchor::NtfsRecord) => push_record_region(&mut out.ntfs_records, absolute),
            Some(Anchor::NtfsIndex) => out.ntfs_indexes.push(ByteRange::new(
                ByteOffset::new(absolute),
                INDEX_BUFFER_BYTES,
            )),
            None => {}
        }
        at += step;
    }
    true
}

/// Bytes of one `INDX` buffer read for its slack.
///
/// The default index-allocation unit; a larger one simply yields the first
/// part of it, which is where the entries are.
const INDEX_BUFFER_BYTES: u64 = 4096;

/// Extends the last record region when adjacent, so a run of orphaned
/// records becomes one range instead of thousands.
fn push_record_region(regions: &mut Vec<ByteRange>, at: u64) {
    let record = u64::from(crate::ntfs::DEFAULT_RECORD_SIZE);
    if let Some(last) = regions.last_mut()
        && last.end().map(ByteOffset::get) == Some(at)
    {
        last.len += record;
    } else {
        regions.push(ByteRange::new(ByteOffset::new(at), record));
    }
}

/// Tests one sector-aligned position for any known anchor.
///
/// `window` starts at absolute offset `at`; `medium_len` bounds what an
/// anchor may claim.
fn anchor_at(window: &[u8], at: u64, medium_len: u64) -> Option<Anchor> {
    if let Some(volume) = volume_at(window, at, medium_len) {
        return Some(Anchor::Volume(volume.kind));
    }
    // An orphaned MFT record: the primary NTFS residue after a re-format,
    // recognised by its signature plus a verifiable fixup array.
    if window.get(..4) == Some(NTFS_RECORD_MAGIC) && crate::ntfs::is_plausible_record(window) {
        return Some(Anchor::NtfsRecord);
    }
    // A directory index buffer. Its slack keeps the names of entries the
    // directory has removed, which is how a deleted file can still be named
    // when nothing else about it survived.
    if window.get(..4) == Some(NTFS_INDEX_MAGIC) {
        return Some(Anchor::NtfsIndex);
    }
    None
}

/// `FILE` record signature. Source: NTFS file-record layout.
const NTFS_RECORD_MAGIC: &[u8; 4] = b"FILE";

/// `INDX` buffer signature. Source: NTFS index-buffer layout.
const NTFS_INDEX_MAGIC: &[u8; 4] = b"INDX";

/// Tests one sector-aligned position for a volume anchor.
///
/// `medium_len` caps reported volume lengths so a corrupt size field cannot
/// claim past the medium.
#[must_use]
pub fn volume_at(window: &[u8], at: u64, medium_len: u64) -> Option<Volume> {
    let remaining = medium_len.saturating_sub(at);

    if let Some(ntfs) = Ntfs::from_boot_sector(window, ByteOffset::new(at)) {
        return Some(Volume {
            kind: FsKind::Ntfs,
            range: ByteRange::new(ByteOffset::new(at), ntfs.volume_bytes.min(remaining)),
            origin: Origin::Residual,
            allocation_bytes: ntfs.cluster_bytes,
        });
    }
    if let Some(fat) = Fat::from_boot_sector(window, ByteOffset::new(at)) {
        return Some(Volume {
            kind: fat.kind,
            // The volume's own total-sector count, capped at the medium; a
            // default of "the rest of the disk" would fabricate its size.
            range: ByteRange::new(ByteOffset::new(at), fat.volume_bytes.min(remaining)),
            origin: Origin::Residual,
            allocation_bytes: fat.cluster_bytes,
        });
    }
    // An ext superblock sits 1024 bytes into its volume, so a hit here means
    // the volume starts one kibibyte earlier.
    if let Some(volume_start) = at.checked_sub(SUPERBLOCK_OFFSET)
        && let Some(ext) = Ext4::from_superblock(window, ByteOffset::new(volume_start))
    {
        // A block count that overflows its own byte size is corrupt: report
        // the anchor with no claimed length rather than inventing one.
        let bytes = ext.block_count.checked_mul(ext.block_bytes).unwrap_or(0);
        return Some(Volume {
            kind: FsKind::Ext4,
            range: ByteRange::new(
                ByteOffset::new(volume_start),
                bytes.min(medium_len.saturating_sub(volume_start)),
            ),
            origin: Origin::Residual,
            allocation_bytes: ext.block_bytes,
        });
    }
    // The magic test comes first: the full container parse stages a block and
    // verifies a checksum, and this runs at every sector of the medium.
    if window.len() >= 4096
        && crate::apfs::has_container_magic(window)
        && let Some(container) = apfs_container_bytes(window)
    {
        return Some(Volume {
            kind: FsKind::Apfs,
            range: ByteRange::new(ByteOffset::new(at), container.min(remaining)),
            origin: Origin::Residual,
            allocation_bytes: apfs_block_bytes(window).unwrap_or(0),
        });
    }
    // A btrfs superblock records which of its four fixed copies it is, so an
    // anchor fixes the volume's start rather than merely suggesting it: a
    // mirror 64 MiB in still names the volume it belongs to, even when the
    // primary was what a later format overwrote. The magic test guards the
    // checksum for the same reason it guards the APFS parse.
    if window.len() >= 4096
        && btrfs::has_superblock_magic(window)
        && let Some(found) = btrfs::from_superblock(window, ByteOffset::new(at))
    {
        let start = found.volume_offset.get();
        return Some(Volume {
            kind: FsKind::Btrfs,
            range: ByteRange::new(
                found.volume_offset,
                found.total_bytes.min(medium_len.saturating_sub(start)),
            ),
            origin: Origin::Residual,
            allocation_bytes: found.sector_bytes,
        });
    }
    None
}

/// Container size in bytes when `window` starts with a checksum-valid `NXSB`.
///
/// Validation (magic, Fletcher-64, block-size sanity) lives in the container
/// parser; the sweep reuses it rather than duplicating the checks.
fn apfs_block_bytes(window: &[u8]) -> Option<u64> {
    let mut cursor = std::io::Cursor::new(window);
    let container = Apfs::open(&mut cursor, ByteOffset::new(0)).ok()??;
    Some(container.block_bytes)
}

fn apfs_container_bytes(window: &[u8]) -> Option<u64> {
    let mut cursor = std::io::Cursor::new(window);
    let container = Apfs::open(&mut cursor, ByteOffset::new(0)).ok()??;
    container.total_bytes
}
