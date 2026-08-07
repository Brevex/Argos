//! Raw image files behind the [`BlockSource`] port.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use argos_core::geometry::{Lba, SectorSize};
use argos_core::source::{BlockSource, DeviceClass, Geometry, ReadError};

use crate::device::DeviceError;

/// A raw (sector-by-sector) image file of a medium, opened read-only.
///
/// Images carry no geometry of their own, so the sector size is chosen at open
/// time; a trailing partial sector is not addressable and is ignored.
#[derive(Debug)]
pub struct ImageSource {
    file: File,
    geometry: Geometry,
    trailing_bytes: u64,
}

impl ImageSource {
    /// Sector size assumed by [`ImageSource::open`] — 512 bytes, the addressing
    /// unit virtually all raw images of consumer media use.
    pub const DEFAULT_SECTOR_SIZE: SectorSize = SectorSize::new(512);

    /// Opens the image at `path` read-only, addressed in 512-byte sectors.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened read-only or its length queried.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DeviceError> {
        Self::with_sector_size(path, Self::DEFAULT_SECTOR_SIZE)
    }

    /// Opens the image at `path` read-only, addressed in sectors of `sector_size`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened read-only or its length queried.
    pub fn with_sector_size(
        path: impl AsRef<Path>,
        sector_size: SectorSize,
    ) -> Result<Self, DeviceError> {
        let path = path.as_ref();
        let file = File::options()
            .read(true)
            .open(path)
            .map_err(|source| DeviceError::open(path, source))?;
        let len = file
            .metadata()
            .map_err(|source| DeviceError::geometry(path, source))?
            .len();
        let sector_count = len / sector_size.as_u64();
        Ok(Self {
            file,
            geometry: Geometry::new(sector_size, sector_count, DeviceClass::ImageFile),
            trailing_bytes: len % sector_size.as_u64(),
        })
    }

    /// Bytes of a trailing partial sector that are not addressable through the
    /// sector geometry. Non-zero for truncated images (e.g. an interrupted
    /// `dd`); reports must disclose it so coverage is never overstated.
    #[must_use]
    pub fn trailing_bytes(&self) -> u64 {
        self.trailing_bytes
    }

    /// The image's geometry, fixed at open time.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Reads exactly `buf.len()` bytes starting at sector `lba`; see
    /// [`BlockSource::read_at`] for the full contract.
    ///
    /// # Errors
    ///
    /// Fails on out-of-range requests and I/O faults.
    ///
    /// # Panics
    ///
    /// Panics if `buf.len()` is zero or not a multiple of the sector size.
    pub fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        let sector_bytes = self.geometry.sector_size.get() as usize;
        assert!(
            !buf.is_empty() && buf.len().is_multiple_of(sector_bytes),
            "read buffer of {} bytes is not a non-zero multiple of the sector size {}",
            buf.len(),
            self.geometry.sector_size
        );
        let sectors = (buf.len() / sector_bytes) as u64;

        if !self.geometry.contains(lba, sectors) {
            return Err(ReadError::out_of_range(
                lba,
                sectors,
                self.geometry.sector_count,
            ));
        }
        let offset = lba
            .to_byte_offset(self.geometry.sector_size)
            .unwrap_or_else(|| {
                panic!(
                    "byte offset of sector {lba} at {} overflowed although the range was checked \
                     against the geometry",
                    self.geometry.sector_size
                )
            })
            .get();

        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(buf))
            .map_err(|source| ReadError::io(lba, sectors, source))
    }
}

impl BlockSource for ImageSource {
    fn geometry(&self) -> Geometry {
        Self::geometry(self)
    }

    fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        Self::read_at(self, lba, buf)
    }
}
