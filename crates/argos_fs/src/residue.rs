//! Residue sweep: volume anchors left behind by earlier formats.
//!
//! A quick re-format overwrites a small metadata region and nothing else, so
//! the anchors of the *previous* filesystem survive elsewhere on the medium.
//! This sweep tests every sector boundary for NTFS boot sectors and `FILE`
//! records, ext superblocks, FAT/exFAT boot sectors and APFS container
//! superblocks, and validates each hit by **internal consistency** — never by
//! position. Overlapping candidates are all kept; later stages decide by
//! yield.

use std::io::{Read, Seek};

use argos_core::geometry::{ByteOffset, ByteRange};

use crate::apfs::Apfs;
use crate::bytes::read_at;
use crate::ext4::{Ext4, SUPERBLOCK_OFFSET};
use crate::fat::Fat;
use crate::ntfs::Ntfs;
use crate::{Anchor, FsError, FsKind, Origin, Volume};

/// Sector granularity of the sweep. Volume anchors are always sector-aligned;
/// stepping by 512 bytes is what makes a whole-surface sweep affordable.
const STEP_BYTES: u64 = 512;

/// Bytes read per sweep window. 1 MiB keeps the sweep in large sequential
/// reads while the window buffer stays small.
const WINDOW_BYTES: usize = 1024 * 1024;

/// Bytes carried between windows so an anchor straddling a window edge is
/// still whole: the largest structure a validator inspects (an APFS block).
const WINDOW_OVERLAP: usize = 4096;

/// Everything one sweep located.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    /// Volumes found, current and residual.
    pub volumes: Vec<Volume>,
    /// Regions holding orphaned NTFS `FILE` records, in ascending order.
    /// Hand these to [`crate::ntfs::orphan_scan`] with the geometry of the
    /// volume they belong to.
    pub ntfs_records: Vec<ByteRange>,
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
    let mut found = Vec::new();
    let mut records: Vec<ByteRange> = Vec::new();
    let mut pos = 0_u64;

    while pos < len {
        let want = usize::try_from((len - pos).min(WINDOW_BYTES as u64)).unwrap_or(WINDOW_BYTES);
        if !read_at(src, pos, want, &mut window)? {
            // A short read at the tail still yields whatever was readable.
            if window.is_empty() {
                break;
            }
        }
        let mut at = 0_usize;
        while at + 512 <= window.len() {
            let absolute = pos + at as u64;
            match anchor_at(&window[at..], absolute, len) {
                Some(Anchor::Volume(_)) => {
                    if let Some(volume) = volume_at(&window[at..], absolute, len) {
                        found.push(volume);
                    }
                }
                Some(Anchor::NtfsRecord) => push_record_region(&mut records, absolute),
                None => {}
            }
            at += usize::try_from(STEP_BYTES).unwrap_or(512);
        }
        if pos + want as u64 >= len {
            break;
        }
        pos += (want - WINDOW_OVERLAP) as u64;
    }

    for volume in &mut found {
        if current
            .iter()
            .any(|range| range.start == volume.range.start)
        {
            volume.origin = Origin::Current;
        }
    }
    found.sort_by_key(|volume| (volume.range.start, volume.range.len));
    found.dedup();
    Ok(Sweep {
        volumes: found,
        ntfs_records: records,
    })
}

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
#[must_use]
pub fn anchor_at(window: &[u8], at: u64, medium_len: u64) -> Option<Anchor> {
    if let Some(volume) = volume_at(window, at, medium_len) {
        return Some(Anchor::Volume(volume.kind));
    }
    // An orphaned MFT record: the primary NTFS residue after a re-format,
    // recognised by its signature plus a verifiable fixup array.
    if window.get(..4) == Some(NTFS_RECORD_MAGIC) && crate::ntfs::is_plausible_record(window) {
        return Some(Anchor::NtfsRecord);
    }
    None
}

/// `FILE` record signature. Source: NTFS file-record layout.
const NTFS_RECORD_MAGIC: &[u8; 4] = b"FILE";

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
        });
    }
    if let Some(fat) = Fat::from_boot_sector(window, ByteOffset::new(at)) {
        return Some(Volume {
            kind: fat.kind,
            // The volume's own total-sector count, capped at the medium; a
            // default of "the rest of the disk" would fabricate its size.
            range: ByteRange::new(ByteOffset::new(at), fat.volume_bytes.min(remaining)),
            origin: Origin::Residual,
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
        });
    }
    if window.len() >= 4096
        && let Some(container) = apfs_container_bytes(window)
    {
        return Some(Volume {
            kind: FsKind::Apfs,
            range: ByteRange::new(ByteOffset::new(at), container.min(remaining)),
            origin: Origin::Residual,
        });
    }
    None
}

/// Container size in bytes when `window` starts with a checksum-valid `NXSB`.
///
/// Validation (magic, Fletcher-64, block-size sanity) lives in the container
/// parser; the sweep reuses it rather than duplicating the checks.
fn apfs_container_bytes(window: &[u8]) -> Option<u64> {
    let mut cursor = std::io::Cursor::new(window);
    let container = Apfs::open(&mut cursor, ByteOffset::new(0)).ok()??;
    container.total_bytes
}
