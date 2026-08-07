//! Partition tables, filesystem metadata recovery and prior-filesystem residue.
//!
//! Everything here parses **untrusted on-disk bytes**: every length, offset and
//! count is bounds-checked, every self-referencing structure walk is bounded,
//! and a value that fails validation fails that object's parse — the scan
//! continues past it. The crate is sans-IO over `impl Read + Seek` and contains
//! no `unsafe`.
//!
//! Modules follow the recovery pipeline: [`part`] reads the current partition
//! tables, [`residue`] sweeps the surface for anchors of *previous* filesystems
//! (what survives re-formatting), and the per-filesystem modules ([`ntfs`],
//! [`ext4`], [`fat`], [`apfs`]) recover deleted files — names, timestamps and
//! extents — from current or residual volumes.
//!
//! What this cannot do: content overwritten by later writes is gone, `TRIM`med
//! SSD blocks read as zeros, and a file whose metadata and content are both
//! destroyed is only reachable by carving, not from here.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::io;

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};

pub mod apfs;
pub mod ext4;
pub mod fat;
pub mod ntfs;
pub mod part;
pub mod residue;

/// Recovered-metadata timestamps are shared domain vocabulary.
pub use argos_core::Timestamps;

mod bytes;

#[cfg(feature = "test-util")]
pub mod fixture;

/// Filesystem family a volume or finding belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FsKind {
    /// NTFS.
    Ntfs,
    /// ext2/ext3/ext4.
    Ext4,
    /// FAT32.
    Fat32,
    /// exFAT.
    ExFat,
    /// APFS container.
    Apfs,
}

impl fmt::Display for FsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Ntfs => "ntfs",
            Self::Ext4 => "ext4",
            Self::Fat32 => "fat32",
            Self::ExFat => "exfat",
            Self::Apfs => "apfs",
        };
        f.write_str(name)
    }
}

/// What a residue-sweep anchor turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Anchor {
    /// A volume anchor: a boot sector or superblock.
    Volume(FsKind),
    /// An orphaned NTFS `FILE` record — metadata of a filesystem whose
    /// `$MFT` no longer describes it, the primary residue after a re-format.
    NtfsRecord,
}

/// How a volume was located.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Listed in the current partition table.
    Current,
    /// Found by the residue sweep: a filesystem an earlier format left behind.
    Residual,
}

/// A located (current or residual) filesystem volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Filesystem family detected at the anchor.
    pub kind: FsKind,
    /// Byte range of the volume within the medium.
    pub range: ByteRange,
    /// Whether the volume is current or residue of an earlier format.
    pub origin: Origin,
}

/// A deleted file recovered from filesystem metadata.
///
/// `Debug` redacts the recovered name: names read off a medium are identifying
/// content and must never leak into logs or panics (A-NO-CONTENT-IN-LOGS);
/// they are reported only through deliberate report output.
#[derive(Clone, PartialEq, Eq)]
pub struct DeletedFile {
    /// File name recovered from metadata, when one survived.
    pub name: Option<String>,
    /// Timestamps recovered from metadata.
    pub timestamps: Timestamps,
    /// File size in bytes claimed by the metadata.
    pub size: u64,
    /// Content extents, absolute in the medium, in file order. Empty when
    /// only the name survived (a directory-entry ghost).
    pub extents: Vec<ByteRange>,
    /// Filesystem the metadata came from.
    pub fs: FsKind,
    /// Evidence tier of this recovery.
    pub confidence: Confidence,
    /// Identity of the filesystem object the metadata came from — MFT record
    /// number, inode number or first cluster — so a finding can be correlated
    /// back to its source and merged across stages (A-PROVENANCE).
    pub source_object: Option<u64>,
}

impl fmt::Debug for DeletedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeletedFile")
            .field("name", &self.name.as_ref().map(|_| "<redacted>"))
            .field("timestamps", &self.timestamps)
            .field("size", &self.size)
            .field("extents", &self.extents)
            .field("fs", &self.fs)
            .field("confidence", &self.confidence)
            .field("source_object", &self.source_object)
            .finish()
    }
}

/// Reading the medium failed while recovering filesystem metadata.
///
/// Corrupt structures are never an error — they fail their own object's parse
/// and the scan continues. This type is I/O only.
#[derive(Debug)]
pub struct FsError {
    offset: ByteOffset,
    source: io::Error,
    backtrace: Backtrace,
}

impl FsError {
    pub(crate) fn io(offset: ByteOffset, source: io::Error) -> Self {
        Self {
            offset,
            source,
            backtrace: Backtrace::capture(),
        }
    }

    /// Byte position the failed read concerned.
    #[must_use]
    pub fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot read medium at byte {}: {}",
            self.offset, self.source
        )?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for FsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}
