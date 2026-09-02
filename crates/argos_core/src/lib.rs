//! Domain vocabulary of Argos: storage newtypes, port traits and canonical errors.
//!
//! Everything here is sans-IO. The ports are the edges of the hexagon: every other
//! crate either implements one or calls through one.
//!
//! | Port | Direction | Adapters |
//! | --- | --- | --- |
//! | [`ports::BlockSource`] | read the medium | device HAL, image file, [`fixture`] |
//! | [`ports::ArtifactSink`] | deliver results | output directory, test collector |
//! | [`ports::Classifier`] | score recovered images | ML triage, [`ports::AcceptAll`] |
//! | [`ports::ProgressSink`] | report progress | CLI renderer, UI bridge, [`ports::Discard`] |
//!
//! Where `std` already provides the abstraction, the `std` trait *is* the port:
//! parsers and carvers consume `impl Read + Seek`. [`ports::BlockSource`] exists only
//! for what that cannot express — sector addressing, bad sectors and geometry.
//!
//! The root itself holds how an artifact was recovered and what it is — [`Stage`],
//! [`Format`], [`Confidence`], [`Timestamps`] — and where on a medium it sits:
//! [`Lba`], [`ByteOffset`], [`SectorSize`] and the two ranges over them. That
//! vocabulary is shared by every crate that produces, merges or reports a finding,
//! so a recovery stage, an evidence tier and a position mean exactly one thing
//! across the workspace.
//!
//! Sector-addressed and byte-addressed values are never mixed: that is the classic
//! data-recovery bug, and these newtypes make it unrepresentable. All arithmetic
//! over a value a medium could have influenced is checked and returns `Option`.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::time::SystemTime;

pub mod ports;

#[cfg(feature = "test-util")]
pub mod fixture;

/// Timestamps recovered from filesystem metadata.
///
/// An absent field was not stored by the filesystem or did not survive. They
/// are never inferred from anything else and never defaulted to "now"
/// (A-PROVENANCE): for an image-possession question the recorded time is often
/// the most probative field there is, and a fabricated one is worse than none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamps {
    /// Creation time, where the filesystem records one.
    pub created: Option<SystemTime>,
    /// Last content modification time.
    pub modified: Option<SystemTime>,
}

impl Timestamps {
    /// Whether no timestamp survived.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.created.is_none() && self.modified.is_none()
    }
}

/// Pipeline stage that produced a finding.
///
/// The stages run cheapest-and-most-trusted first, so a finding's stage also
/// says how much work stood behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// Partition tables and the prior-filesystem residue sweep.
    Volumes,
    /// Filesystem metadata recovery over located volumes.
    Filesystem,
    /// Full-surface signature carving.
    Carve,
    /// Reassembly of images the medium stored in pieces.
    Reassembly,
    /// Structural validation, hashing and scoring of candidates.
    Validation,
    /// Writing artifacts, and describing each one once it is written:
    /// its preview, its perceptual hash and its triage label all come from the
    /// bytes this stage is already holding.
    Report,
    /// Joining what the later stages learned onto the records already written,
    /// and writing the manifest that describes them.
    ///
    /// Last, and never a finding's stage: nothing is recovered here. It is a
    /// stage because on a whole-disk recovery it is minutes of work between
    /// the final artifact and the run ending, and a stage that says nothing
    /// for minutes cannot be told from one that has stopped.
    Manifest,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Volumes => "volumes",
            Self::Filesystem => "filesystem",
            Self::Carve => "carve",
            Self::Reassembly => "reassembly",
            Self::Validation => "validation",
            Self::Report => "report",
            Self::Manifest => "manifest",
        };
        f.write_str(name)
    }
}

/// Image format of a recovered artifact.
///
/// Exhaustive on purpose: a new format needs a validator, an extension, a
/// signature and a report mapping, and every one of those sites should fail to
/// compile until it is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    /// JFIF/EXIF JPEG (ITU-T T.81).
    Jpeg,
    /// PNG (ISO 15948).
    Png,
}

impl Format {
    /// Conventional file extension for the format.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

impl std::str::FromStr for Format {
    type Err = UnknownFormat;

    /// Parses the name [`Display`](fmt::Display) writes.
    ///
    /// This is how a format survives a round trip through a manifest, which is
    /// what lets a later run pick up where an earlier one left off.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "jpeg" => Ok(Self::Jpeg),
            "png" => Ok(Self::Png),
            _ => Err(UnknownFormat),
        }
    }
}

/// A name that is not one of the formats this tool handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownFormat;

impl fmt::Display for UnknownFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an image format this tool recovers")
    }
}

impl std::error::Error for UnknownFormat {}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
        };
        f.write_str(name)
    }
}

/// Evidence tier of a recovered artifact, ordered weakest to strongest.
///
/// The tier is fixed by how the artifact was obtained and is never raised by
/// post-processing; reporting a tier above the evidence would fabricate certainty.
/// The ladder is deliberately exhaustive — a new tier is a change to the recovery
/// model, not a routine addition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Medium bytes decoded inside a header this tool supplied.
    ///
    /// An entropy-coded fragment with no header of its own, entered at a
    /// restart marker and grafted onto the header of a surviving file from the
    /// same batch. The pixels came off the medium; the container did not, and
    /// the frame size is another file's. It is the floor of the ladder because
    /// it is the one tier whose artifact is **not a file the medium ever
    /// held** — an examiner looking at one is looking at real pixels in an
    /// arrangement this tool built.
    Grafted,
    /// A fragment or embedded thumbnail; the parent image was not recovered.
    PartialOrThumbnail,
    /// Reassembled from non-contiguous fragments by carving.
    Reassembled,
    /// Carved as one contiguous, fully validated run.
    ContiguousCarve,
    /// Extents recovered from stale filesystem journal copies.
    JournalResidue,
    /// Extents taken from live or residual filesystem metadata.
    FsMetadata,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Grafted => "grafted",
            Self::PartialOrThumbnail => "partial-or-thumbnail",
            Self::Reassembled => "reassembled",
            Self::ContiguousCarve => "contiguous-carve",
            Self::JournalResidue => "journal-residue",
            Self::FsMetadata => "fs-metadata",
        };
        f.write_str(name)
    }
}

/// Logical block address: a position on a medium, counted in sectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lba(u64);

impl Lba {
    /// Address of sector `sector`.
    #[must_use]
    pub const fn new(sector: u64) -> Self {
        Self(sector)
    }

    /// The sector number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Address `sectors` past `self`, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, sectors: u64) -> Option<Self> {
        match self.0.checked_add(sectors) {
            Some(sector) => Some(Self(sector)),
            None => None,
        }
    }

    /// Byte position of the first byte of this sector, or `None` on overflow.
    #[must_use]
    pub const fn to_byte_offset(self, sector_size: SectorSize) -> Option<ByteOffset> {
        match self.0.checked_mul(sector_size.as_u64()) {
            Some(bytes) => Some(ByteOffset::new(bytes)),
            None => None,
        }
    }
}

impl fmt::Display for Lba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A position on a medium, counted in bytes from the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(u64);

impl ByteOffset {
    /// Offset of byte `bytes`.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The byte position.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Offset `bytes` past `self`, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }
}

impl fmt::Display for ByteOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Size of a logical sector in bytes; guarded to a power of two in
/// `SectorSize::MIN_BYTES..=SectorSize::MAX_BYTES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorSize(u32);

impl SectorSize {
    /// Smallest supported logical sector size — 512 bytes, the floor for every
    /// ATA/SCSI/NVMe block device.
    pub(crate) const MIN_BYTES: u32 = 512;

    /// Largest supported logical sector size — 4096 bytes (4Kn devices).
    pub(crate) const MAX_BYTES: u32 = 4096;

    /// Validates `bytes` as a sector size.
    ///
    /// # Errors
    ///
    /// Fails unless `bytes` is a power of two in `MIN_BYTES..=MAX_BYTES`.
    pub fn from_u32(bytes: u32) -> Result<Self, GeometryError> {
        if bytes.is_power_of_two() && (Self::MIN_BYTES..=Self::MAX_BYTES).contains(&bytes) {
            Ok(Self(bytes))
        } else {
            Err(GeometryError::new(bytes))
        }
    }

    /// Like [`SectorSize::from_u32`], for values known at compile time.
    ///
    /// # Panics
    ///
    /// Panics if `bytes` is not a power of two in `MIN_BYTES..=MAX_BYTES`; in a
    /// `const` context this is a compile error.
    #[must_use]
    pub const fn new(bytes: u32) -> Self {
        assert!(
            bytes.is_power_of_two() && bytes >= Self::MIN_BYTES && bytes <= Self::MAX_BYTES,
            "sector size must be a power of two in 512..=4096"
        );
        Self(bytes)
    }

    /// The sector size in bytes.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The sector size in bytes, widened for offset arithmetic.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0 as u64
    }
}

impl fmt::Display for SectorSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes", self.0)
    }
}

impl TryFrom<u32> for SectorSize {
    type Error = GeometryError;

    fn try_from(bytes: u32) -> Result<Self, Self::Error> {
        Self::from_u32(bytes)
    }
}

/// A contiguous run of sectors: `start` inclusive, `sectors` long.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorRange {
    /// First sector of the run.
    pub start: Lba,
    /// Number of sectors in the run.
    pub sectors: u64,
}

impl SectorRange {
    /// The run of `sectors` sectors starting at `start`.
    #[must_use]
    pub const fn new(start: Lba, sectors: u64) -> Self {
        Self { start, sectors }
    }

    /// First sector past the end of the run, or `None` on overflow.
    #[must_use]
    pub const fn end(self) -> Option<Lba> {
        self.start.checked_add(self.sectors)
    }
}

impl fmt::Display for SectorRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sectors {}..+{}", self.start, self.sectors)
    }
}

/// A contiguous run of bytes: `start` inclusive, `len` long.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteRange {
    /// First byte of the run.
    pub start: ByteOffset,
    /// Length of the run in bytes.
    pub len: u64,
}

impl ByteRange {
    /// The run of `len` bytes starting at `start`.
    #[must_use]
    pub const fn new(start: ByteOffset, len: u64) -> Self {
        Self { start, len }
    }

    /// First byte past the end of the run, or `None` on overflow.
    #[must_use]
    pub const fn end(self) -> Option<ByteOffset> {
        self.start.checked_add(self.len)
    }

    /// First byte past the end of the run, saturating at [`u64::MAX`].
    ///
    /// A range whose end overflows can only come from a corrupt on-disk
    /// length. Saturating keeps ordering and overlap tests total without
    /// silently shrinking the range, which would understate the damage.
    #[must_use]
    pub const fn end_saturating(self) -> ByteOffset {
        ByteOffset::new(self.start.get().saturating_add(self.len))
    }
}

impl fmt::Display for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bytes {}..+{}", self.start, self.len)
    }
}

/// A value could not be interpreted as valid storage geometry.
#[derive(Debug)]
pub struct GeometryError {
    bytes: u32,
    backtrace: Backtrace,
}

impl GeometryError {
    fn new(bytes: u32) -> Self {
        Self {
            bytes,
            backtrace: Backtrace::capture(),
        }
    }

    /// The rejected value.
    #[must_use]
    pub fn bytes(&self) -> u32 {
        self.bytes
    }

    /// Backtrace captured where the value was rejected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid sector size: {} bytes (must be a power of two in {}..={})",
            self.bytes,
            SectorSize::MIN_BYTES,
            SectorSize::MAX_BYTES
        )?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for GeometryError {}
