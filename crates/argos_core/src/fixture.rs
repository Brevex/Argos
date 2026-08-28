//! In-memory [`BlockSource`] fixture with injectable bad sectors.
//!
//! Only compiled under the `test-util` feature so production builds cannot reach it.
//! Fixture content must be synthetic — never real personal photos.

use std::collections::BTreeSet;

use crate::ports::{BlockSource, DeviceClass, Geometry, ReadError};
use crate::{Lba, SectorSize};

/// An in-memory medium: a byte buffer addressed as sectors, with selected sectors
/// made unreadable to exercise damage handling.
#[derive(Clone, Debug)]
pub struct MemDisk {
    sector_size: SectorSize,
    class: DeviceClass,
    data: Vec<u8>,
    bad: BTreeSet<u64>,
}

impl MemDisk {
    /// A medium of `data` addressed in sectors of `sector_size`.
    ///
    /// # Panics
    ///
    /// Panics if `data.len()` is not a multiple of `sector_size` — a fixture built
    /// that way is a bug in the test, not a medium condition.
    #[must_use]
    pub fn new(sector_size: SectorSize, data: Vec<u8>) -> Self {
        assert!(
            data.len().is_multiple_of(sector_size.get() as usize),
            "fixture length {} is not a multiple of the sector size {}",
            data.len(),
            sector_size
        );
        Self {
            sector_size,
            class: DeviceClass::ImageFile,
            data,
            bad: BTreeSet::new(),
        }
    }

    /// Marks sector `lba` unreadable.
    #[must_use]
    pub fn with_bad_sector(mut self, lba: Lba) -> Self {
        self.bad.insert(lba.get());
        self
    }

    /// The full backing buffer, including bytes hidden behind bad sectors.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn sector_count(&self) -> u64 {
        (self.data.len() / self.sector_size.get() as usize) as u64
    }

    /// The fixture's geometry.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        Geometry::new(self.sector_size, self.sector_count(), self.class)
    }

    /// Reads exactly `buf.len()` bytes starting at sector `lba`; see
    /// [`BlockSource::read_at`] for the full contract.
    ///
    /// # Errors
    ///
    /// Fails on out-of-range requests and scripted bad sectors.
    ///
    /// # Panics
    ///
    /// Panics if `buf.len()` is zero or not a multiple of the sector size.
    pub fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        let sector_bytes = self.sector_size.get() as usize;
        assert!(
            !buf.is_empty() && buf.len().is_multiple_of(sector_bytes),
            "read buffer of {} bytes is not a non-zero multiple of the sector size {}",
            buf.len(),
            self.sector_size
        );
        let sectors = (buf.len() / sector_bytes) as u64;

        if !self.geometry().contains(lba, sectors) {
            return Err(ReadError::out_of_range(lba, sectors, self.sector_count()));
        }
        if let Some(&bad) = self.bad.range(lba.get()..lba.get() + sectors).next() {
            return Err(ReadError::bad_sector(Lba::new(bad), 1));
        }

        let start = usize::try_from(lba.get()).unwrap_or_else(|_| {
            panic!("start sector {lba} fits usize: the range was checked against the data length")
        }) * sector_bytes;
        buf.copy_from_slice(&self.data[start..start + buf.len()]);
        Ok(())
    }
}

impl BlockSource for MemDisk {
    fn geometry(&self) -> Geometry {
        Self::geometry(self)
    }

    fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        Self::read_at(self, lba, buf)
    }
}
