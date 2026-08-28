//! Every medium this crate opens behind the [`BlockSource`] port, and the
//! byte-addressed view over any of them.
//!
//! [`Device`] is the OS block device, [`ImageSource`] a raw image file, and
//! [`Ctrl`] drives the mocked device that runs the same code paths without
//! hardware (`test-util` only). [`BlockReader`] adapts any of the three from
//! sector addressing to `Read + Seek`, which is what every recovery crate
//! consumes.
//!
//! Every open here is read-only at the lowest layer; none of these types
//! exposes a write path to a source medium.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
#[cfg(feature = "test-util")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "test-util")]
use argos_core::fixture::MemDisk;
use argos_core::ports::{BlockSource, DeviceClass, Geometry, ReadError};
use argos_core::{Lba, SectorSize};

use crate::class::TrimState;

/// A `len`-byte view into `bounce`, aligned to `align` bytes.
///
/// Both unbuffered read paths — Linux' `O_DIRECT` and Windows'
/// `FILE_FLAG_NO_BUFFERING` — require the user buffer to be sector-aligned,
/// and a caller's buffer promises nothing. Growing one owned buffer and
/// taking an aligned window of it turns that requirement into a single place
/// to audit, and reuses the allocation across every read (`M-MEM-REUSE`).
#[cfg(any(target_os = "linux", windows))]
pub fn aligned_slice(bounce: &mut Vec<u8>, len: usize, align: usize) -> &mut [u8] {
    bounce.resize(len + align, 0);
    let start = bounce.as_ptr().align_offset(align);
    &mut bounce[start..start + len]
}

/// A raw block device, opened read-only at the lowest layer.
///
/// Construction is per-OS; on platforms without a HAL yet, [`Device::open`]
/// fails. The mocked variant (behind `test-util`) drives the same code paths
/// without a device.
#[derive(Debug)]
pub struct Device {
    core: Core,
}

#[derive(Debug)]
enum Core {
    #[cfg(target_os = "linux")]
    Native(crate::platform::linux::Native),
    #[cfg(target_os = "macos")]
    Native(crate::platform::macos::Native),
    #[cfg(windows)]
    Native(crate::platform::windows::Native),
    #[cfg(feature = "test-util")]
    Mocked(Ctrl),
    /// A target with no HAL yet. Carries nothing; it exists so the enum has a
    /// variant on such targets and the match arms below stay exhaustive.
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    Unsupported,
}

impl Device {
    /// Opens the block device at `path` read-only.
    ///
    /// # Errors
    ///
    /// Fails when the path cannot be opened, is not a block device, when its
    /// geometry cannot be queried, or on platforms without a native HAL yet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DeviceError> {
        let path = path.as_ref();
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                core: Core::Native(crate::platform::linux::Native::open(path)?),
            })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                core: Core::Native(crate::platform::macos::Native::open(path)?),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                core: Core::Native(crate::platform::windows::Native::open(path)?),
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            Err(DeviceError::unsupported(path))
        }
    }

    /// A mocked device plus the controller that scripts its behaviour.
    ///
    /// The controller is created here and returned — two devices can never share
    /// one controller by accident.
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn new_mocked(disk: argos_core::fixture::MemDisk) -> (Self, Ctrl) {
        let ctrl = Ctrl::new(disk);
        (
            Self {
                core: Core::Mocked(ctrl.clone()),
            },
            ctrl,
        )
    }

    /// The device's geometry, queried once at open time.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        match &self.core {
            #[cfg(any(target_os = "linux", target_os = "macos", windows))]
            Core::Native(native) => native.geometry(),
            #[cfg(feature = "test-util")]
            Core::Mocked(ctrl) => ctrl.geometry(),
            #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
            Core::Unsupported => unreachable!("an unsupported target never opens a device"),
        }
    }

    /// Whether the medium reports TRIM enabled.
    ///
    /// Finer than the device class alone: an SSD with TRIM disabled still
    /// holds its deleted content, and telling a user otherwise would talk
    /// them out of a recovery that was going to work
    /// (see [`class::expects_deleted_content`](crate::class::expects_deleted_content)).
    #[must_use]
    pub fn trim(&self) -> TrimState {
        match &self.core {
            #[cfg(any(target_os = "linux", target_os = "macos", windows))]
            Core::Native(native) => native.trim(),
            #[cfg(feature = "test-util")]
            Core::Mocked(_) => TrimState::Unknown,
            #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
            Core::Unsupported => TrimState::Unknown,
        }
    }

    /// Reads exactly `buf.len()` bytes starting at sector `lba`; see
    /// [`BlockSource::read_at`] for the full contract.
    ///
    /// # Errors
    ///
    /// Fails on out-of-range requests, unreadable sectors and I/O faults.
    pub fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        match &mut self.core {
            #[cfg(any(target_os = "linux", target_os = "macos", windows))]
            Core::Native(native) => native.read_at(lba, buf),
            #[cfg(feature = "test-util")]
            Core::Mocked(ctrl) => ctrl.read_at(lba, buf),
            #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
            Core::Unsupported => unreachable!("an unsupported target never opens a device"),
        }
    }
}

impl BlockSource for Device {
    fn geometry(&self) -> Geometry {
        Self::geometry(self)
    }

    fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        Self::read_at(self, lba, buf)
    }
}

/// Opening or querying a device failed.
#[derive(Debug)]
pub struct DeviceError {
    path: PathBuf,
    kind: DeviceErrorKind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum DeviceErrorKind {
    Open(io::Error),
    NotBlockDevice,
    Geometry(io::Error),
    Unsupported,
}

impl DeviceError {
    pub(crate) fn open(path: &Path, source: io::Error) -> Self {
        Self::with_kind(path, DeviceErrorKind::Open(source))
    }

    pub(crate) fn not_block_device(path: &Path) -> Self {
        Self::with_kind(path, DeviceErrorKind::NotBlockDevice)
    }

    pub(crate) fn geometry(path: &Path, source: io::Error) -> Self {
        Self::with_kind(path, DeviceErrorKind::Geometry(source))
    }

    #[cfg_attr(
        any(target_os = "linux", target_os = "macos", windows),
        expect(dead_code, reason = "constructed only on targets without a native HAL")
    )]
    pub(crate) fn unsupported(path: &Path) -> Self {
        Self::with_kind(path, DeviceErrorKind::Unsupported)
    }

    fn with_kind(path: &Path, kind: DeviceErrorKind) -> Self {
        Self {
            path: path.to_path_buf(),
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// Path of the device the failure concerns.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the path exists but is not a block device.
    #[must_use]
    pub fn is_not_block_device(&self) -> bool {
        matches!(self.kind, DeviceErrorKind::NotBlockDevice)
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "device {}: ", self.path.display())?;
        match &self.kind {
            DeviceErrorKind::Open(source) => write!(f, "cannot open read-only: {source}"),
            DeviceErrorKind::NotBlockDevice => {
                f.write_str("not a block device (raw images go through ImageSource)")
            }
            DeviceErrorKind::Geometry(source) => write!(f, "cannot query geometry: {source}"),
            DeviceErrorKind::Unsupported => {
                f.write_str("no native device HAL for this platform yet")
            }
        }?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for DeviceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            DeviceErrorKind::Open(source) | DeviceErrorKind::Geometry(source) => Some(source),
            DeviceErrorKind::NotBlockDevice | DeviceErrorKind::Unsupported => None,
        }
    }
}

/// Controller for a mocked [`Device`](crate::Device).
#[cfg(feature = "test-util")]
#[derive(Clone, Debug)]
pub struct Ctrl {
    state: Arc<Mutex<State>>,
}

#[cfg(feature = "test-util")]
#[derive(Debug)]
struct State {
    disk: MemDisk,
    reads: u64,
}

#[cfg(feature = "test-util")]
impl Ctrl {
    pub(crate) fn new(disk: MemDisk) -> Self {
        Self {
            state: Arc::new(Mutex::new(State { disk, reads: 0 })),
        }
    }

    /// Number of `read_at` calls the mocked device has served.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.lock().reads
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.lock().disk.geometry()
    }

    pub(crate) fn read_at(&self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
        let mut state = self.lock();
        state.reads += 1;
        state.disk.read_at(lba, buf)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect(
            "mock state lock poisoned: a previous mock operation panicked while \
                 holding it (e.g. the buffer-contract assert in MemDisk::read_at)",
        )
    }
}

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
    pub(crate) const DEFAULT_SECTOR_SIZE: SectorSize = SectorSize::new(512);

    /// Opens the image at `path` read-only, addressed in 512-byte sectors.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened read-only or its length queried.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DeviceError> {
        let sector_size = Self::DEFAULT_SECTOR_SIZE;
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

/// `Read + Seek` view of a [`BlockSource`], addressed in bytes.
///
/// Reads that are sector-aligned go straight into the caller's buffer; only an
/// unaligned head or a sub-sector request is staged through the internal
/// bounce buffer, so the engine's large aligned chunk reads cost no extra copy.
///
/// Like every path to a medium in Argos this is read-only: the adapter exposes
/// no `Write`, and the port beneath it has no write method to expose.
#[derive(Debug)]
pub struct BlockReader<S> {
    source: S,
    geometry: Geometry,
    /// Total addressable bytes: whole sectors only.
    len: u64,
    /// Absolute byte position of the next read.
    pos: u64,
    /// One-sector staging buffer for unaligned reads; reused across calls
    /// (`M-MEM-REUSE`).
    bounce: Vec<u8>,
}

impl<S: BlockSource> BlockReader<S> {
    /// Wraps `source`, positioned at byte zero.
    ///
    /// # Panics
    ///
    /// Panics if the medium's sector count times its sector size overflows
    /// `u64` — a geometry that cannot describe a real device.
    #[must_use]
    pub fn new(source: S) -> Self {
        let geometry = source.geometry();
        let len = geometry.capacity_bytes().unwrap_or_else(|| {
            panic!(
                "medium geometry overflows: {} sectors of {}",
                geometry.sector_count, geometry.sector_size
            )
        });
        Self {
            source,
            geometry,
            len,
            pos: 0,
            bounce: Vec::new(),
        }
    }

    /// Addressable length of the medium in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the medium addresses no bytes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The medium's geometry.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Returns the wrapped source.
    #[must_use]
    pub fn into_inner(self) -> S {
        self.source
    }
}

impl<S: BlockSource> Read for BlockReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.len.saturating_sub(self.pos);
        if remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let sector_bytes = self.geometry.sector_size.get() as usize;
        let want = usize::try_from(remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        let within = usize::try_from(self.pos % self.geometry.sector_size.as_u64())
            .unwrap_or_else(|_| unreachable!("a remainder of a u32 sector size fits usize"));
        let lba = Lba::new(self.pos / self.geometry.sector_size.as_u64());

        // Aligned and at least one whole sector: read straight into `buf`.
        if within == 0 && want >= sector_bytes {
            let take = (want / sector_bytes) * sector_bytes;
            self.source
                .read_at(lba, &mut buf[..take])
                .map_err(io::Error::other)?;
            self.pos += take as u64;
            return Ok(take);
        }

        // Unaligned head or a sub-sector request: stage one sector and copy
        // out the requested slice. The caller's `read_exact` loops for more.
        self.bounce.clear();
        self.bounce.resize(sector_bytes, 0);
        self.source
            .read_at(lba, &mut self.bounce)
            .map_err(io::Error::other)?;
        let take = want.min(sector_bytes - within);
        buf[..take].copy_from_slice(&self.bounce[within..within + take]);
        self.pos += take as u64;
        Ok(take)
    }
}

impl<S: BlockSource> Seek for BlockReader<S> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        // Seeking past the end is legal and reads zero bytes there; seeking to
        // a negative position is not.
        let Some(target) = target else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the medium",
            ));
        };
        self.pos = target;
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.pos)
    }
}
