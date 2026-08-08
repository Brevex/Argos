//! The OS block device behind the [`BlockSource`] port.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use argos_core::geometry::Lba;
use argos_core::source::{BlockSource, Geometry, ReadError};

use crate::class::TrimState;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// A `len`-byte view into `bounce`, aligned to `align` bytes.
///
/// Both unbuffered read paths — Linux' `O_DIRECT` and Windows'
/// `FILE_FLAG_NO_BUFFERING` — require the user buffer to be sector-aligned,
/// and a caller's buffer promises nothing. Growing one owned buffer and
/// taking an aligned window of it turns that requirement into a single place
/// to audit, and reuses the allocation across every read (`M-MEM-REUSE`).
#[cfg(any(target_os = "linux", windows))]
fn aligned_slice(bounce: &mut Vec<u8>, len: usize, align: usize) -> &mut [u8] {
    bounce.resize(len + align, 0);
    let start = bounce.as_ptr().align_offset(align);
    &mut bounce[start..start + len]
}

/// A raw block device, opened read-only at the lowest layer.
///
/// Construction is per-OS; on platforms without a HAL yet, [`Device::open`] fails
/// with an error whose [`DeviceError::is_unsupported`] returns `true`. The mocked
/// variant (behind `test-util`) drives the same code paths without a device.
#[derive(Debug)]
pub struct Device {
    core: Core,
}

#[derive(Debug)]
enum Core {
    #[cfg(target_os = "linux")]
    Native(linux::Native),
    #[cfg(target_os = "macos")]
    Native(macos::Native),
    #[cfg(windows)]
    Native(windows::Native),
    #[cfg(feature = "test-util")]
    Mocked(crate::mock::Ctrl),
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
                core: Core::Native(linux::Native::open(path)?),
            })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                core: Core::Native(macos::Native::open(path)?),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                core: Core::Native(windows::Native::open(path)?),
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
    pub fn new_mocked(disk: argos_core::fixture::MemDisk) -> (Self, crate::mock::Ctrl) {
        let ctrl = crate::mock::Ctrl::new(disk);
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

    /// Whether this platform has no native device HAL yet.
    #[must_use]
    pub fn is_unsupported(&self) -> bool {
        matches!(self.kind, DeviceErrorKind::Unsupported)
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
