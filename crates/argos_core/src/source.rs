//! The read-only port to a medium under analysis.
//!
//! [`BlockSource`] is the only way any Argos crate reaches bytes on a device or image.
//! It deliberately has no write, discard or passthrough method: a write path to the
//! source medium must not exist anywhere in the workspace.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::io;

use crate::geometry::{Lba, SectorSize};

/// Sector-addressed, read-only access to a medium under analysis.
///
/// Implementations report unreadable regions as [`ReadError`]s — corruption and bad
/// sectors are the expected operating condition, never a reason to panic — and must
/// never fabricate data for sectors they could not read.
pub trait BlockSource: Send {
    /// The medium's geometry, queried once at open time.
    fn geometry(&self) -> Geometry;

    /// Reads exactly `buf.len()` bytes starting at the first byte of sector `lba`.
    ///
    /// # Errors
    ///
    /// Fails when the range lies outside the medium, when the underlying medium
    /// reports unreadable sectors, or on an I/O fault. On error the contents of
    /// `buf` are unspecified and must not be used.
    ///
    /// # Panics
    ///
    /// Implementations panic if `buf.len()` is zero or not a multiple of the sector
    /// size — that is a caller bug, not a property of the medium.
    fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError>;
}

/// Geometry of a medium: sector size, extent and device class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Logical sector size used for all addressing on this medium.
    pub sector_size: SectorSize,
    /// Total number of addressable sectors.
    pub sector_count: u64,
    /// What kind of medium this is.
    pub class: DeviceClass,
}

impl Geometry {
    /// Geometry of a `class` medium with `sector_count` sectors of `sector_size`.
    #[must_use]
    pub const fn new(sector_size: SectorSize, sector_count: u64, class: DeviceClass) -> Self {
        Self {
            sector_size,
            sector_count,
            class,
        }
    }

    /// Total capacity in bytes, or `None` on overflow.
    #[must_use]
    pub const fn capacity_bytes(&self) -> Option<u64> {
        self.sector_count.checked_mul(self.sector_size.as_u64())
    }

    /// Whether `range` (starting at `lba`, `sectors` long) lies within the medium.
    #[must_use]
    pub const fn contains(&self, lba: Lba, sectors: u64) -> bool {
        match lba.get().checked_add(sectors) {
            Some(end) => end <= self.sector_count,
            None => false,
        }
    }
}

/// Kind of medium behind a [`BlockSource`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceClass {
    /// Rotational disk: sequential access dominates throughput.
    Hdd,
    /// Solid-state device: TRIM may have erased deleted data.
    Ssd,
    /// A file holding a raw image of a medium.
    ImageFile,
    /// The class could not be determined.
    Unknown,
}

impl fmt::Display for DeviceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Hdd => "hdd",
            Self::Ssd => "ssd",
            Self::ImageFile => "image file",
            Self::Unknown => "unknown",
        };
        f.write_str(name)
    }
}

/// A read from a [`BlockSource`] failed.
///
/// Carries the failed range so reports can map damage precisely; accessors expose
/// what a caller can act on without making internal failure modes public API.
#[derive(Debug)]
pub struct ReadError {
    lba: Lba,
    sectors: u64,
    kind: ReadErrorKind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum ReadErrorKind {
    BadSector,
    OutOfRange { sector_count: u64 },
    Io(io::Error),
}

impl ReadError {
    /// The medium reported the range starting at `lba` unreadable.
    #[must_use]
    pub fn bad_sector(lba: Lba, sectors: u64) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::BadSector)
    }

    /// The requested range does not fit a medium of `sector_count` sectors.
    #[must_use]
    pub fn out_of_range(lba: Lba, sectors: u64, sector_count: u64) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::OutOfRange { sector_count })
    }

    /// An I/O fault occurred while reading the range.
    #[must_use]
    pub fn io(lba: Lba, sectors: u64, source: io::Error) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::Io(source))
    }

    fn with_kind(lba: Lba, sectors: u64, kind: ReadErrorKind) -> Self {
        Self {
            lba,
            sectors,
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// First sector of the failed range.
    #[must_use]
    pub fn lba(&self) -> Lba {
        self.lba
    }

    /// Length of the failed range in sectors.
    #[must_use]
    pub fn sectors(&self) -> u64 {
        self.sectors
    }

    /// Whether the medium itself reported the sectors unreadable.
    #[must_use]
    pub fn is_bad_sector(&self) -> bool {
        matches!(self.kind, ReadErrorKind::BadSector)
    }

    /// Whether the request lay outside the medium.
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        matches!(self.kind, ReadErrorKind::OutOfRange { .. })
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "read of sectors {}..+{} failed: ",
            self.lba, self.sectors
        )?;
        match &self.kind {
            ReadErrorKind::BadSector => f.write_str("medium reports bad sector"),
            ReadErrorKind::OutOfRange { sector_count } => {
                write!(f, "range exceeds medium of {sector_count} sectors")
            }
            ReadErrorKind::Io(source) => write!(f, "i/o fault: {source}"),
        }?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for ReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ReadErrorKind::Io(source) => Some(source),
            ReadErrorKind::BadSector | ReadErrorKind::OutOfRange { .. } => None,
        }
    }
}
