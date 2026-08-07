//! Storage positions and sizes: sector-addressed and byte-addressed, never mixed.
//!
//! Mixing a sector number with a byte offset is the classic data-recovery bug; these
//! newtypes make it unrepresentable. All arithmetic that involves a value which could
//! have been influenced by a medium is checked and returns `Option`.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;

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
/// [`SectorSize::MIN_BYTES`]`..=`[`SectorSize::MAX_BYTES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorSize(u32);

impl SectorSize {
    /// Smallest supported logical sector size — 512 bytes, the floor for every
    /// ATA/SCSI/NVMe block device.
    pub const MIN_BYTES: u32 = 512;

    /// Largest supported logical sector size — 4096 bytes (4Kn devices).
    pub const MAX_BYTES: u32 = 4096;

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
