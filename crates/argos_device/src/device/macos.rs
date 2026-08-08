//! macOS native device access: the raw `/dev/rdiskN` node and its `DKIOC`
//! geometry ioctls.
//!
//! The `unsafe` here is three ioctl wrappers, each one call wide. Devices are
//! opened `O_RDONLY`; there is no write path to encapsulate (A-READ-ONLY).
//!
//! macOS exposes every medium twice. `/dev/diskN` is a buffered block device;
//! `/dev/rdiskN` is the raw character device, which is both faster for a
//! full-surface sweep and free of the buffer cache's second-hand pages. Argos
//! always reads the raw node, rewriting the path if the caller named the
//! buffered one.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

use argos_core::geometry::{Lba, SectorSize};
use argos_core::source::{Geometry, ReadError};

use crate::class::{self, TrimState};
use crate::naming;

use super::DeviceError;

/// `DKIOCGETBLOCKSIZE`: writes the logical block size into a `u32`.
///
/// Darwin encodes ioctls as `_IOR(group, number, type)`, which is
/// `0x4000_0000 | ((size & 0x1fff) << 16) | (group << 8) | number`. For
/// `('d', 24, u32)` that is `0x4000_0000 | 0x0004_0000 | 0x6400 | 0x18`.
const DKIOCGETBLOCKSIZE: libc::c_ulong = 0x4004_6418;

/// `DKIOCGETBLOCKCOUNT`: writes the block count into a `u64`.
/// `_IOR('d', 25, u64)` by the encoding above.
const DKIOCGETBLOCKCOUNT: libc::c_ulong = 0x4008_6419;

/// `DKIOCISSOLIDSTATE`: writes a boolean into a `u32`.
/// `_IOR('d', 79, u32)` by the encoding above.
///
/// Unlike the two geometry requests, this one is advisory: a driver that does
/// not implement it answers `ENOTTY`, which leaves the class `Unknown` rather
/// than wrong.
const DKIOCISSOLIDSTATE: libc::c_ulong = 0x4004_644F;

/// `DKIOCGETFEATURES`: writes the medium's feature flags into a `u32`.
/// `_IOR('d', 76, u32)` by the encoding above.
const DKIOCGETFEATURES: libc::c_ulong = 0x4004_644C;

/// `DK_FEATURE_UNMAP`: the feature bit reporting UNMAP — the SCSI/NVMe name
/// for what ATA calls TRIM.
const DK_FEATURE_UNMAP: u32 = 0x0000_0010;

/// A raw disk device opened `O_RDONLY`.
pub(crate) struct Native {
    file: File,
    geometry: Geometry,
    trim: TrimState,
}

/// Manual impl for symmetry with the other HALs: nothing here holds medium
/// content, but the type must not grow a derived `Debug` that later would
/// (A-NO-CONTENT-IN-LOGS).
impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Native")
            .field("file", &self.file)
            .field("geometry", &self.geometry)
            .field("trim", &self.trim)
            .finish()
    }
}

impl Native {
    pub(crate) fn open(path: &Path) -> Result<Self, DeviceError> {
        let text = path.to_string_lossy();
        // Anything without a disk-node convention belongs to `ImageSource`.
        if naming::macos_node_kind(&text).is_none() {
            return Err(DeviceError::not_block_device(path));
        }
        // Read the raw character node whatever the caller named.
        let raw = naming::macos_raw_path(&text);
        let raw = Path::new(&raw);

        let file = File::options()
            .read(true)
            .custom_flags(libc::O_RDONLY)
            .open(raw)
            .map_err(|source| DeviceError::open(raw, source))?;

        let block_bytes = u32_ioctl(&file, DKIOCGETBLOCKSIZE)
            .map_err(|source| DeviceError::geometry(raw, source))?;
        let block_count = u64_ioctl(&file, DKIOCGETBLOCKCOUNT)
            .map_err(|source| DeviceError::geometry(raw, source))?;
        let sector_size = SectorSize::from_u32(block_bytes).map_err(|_unusable| {
            DeviceError::geometry(
                raw,
                io::Error::other(format!(
                    "the disk driver reported an unusable logical block size: {block_bytes}"
                )),
            )
        })?;

        // Both remaining queries are advisory: a driver that declines leaves
        // the answer unknown rather than wrong.
        let class =
            class::from_solid_state(u32_ioctl(&file, DKIOCISSOLIDSTATE).ok().map(|v| v != 0));
        let trim = TrimState::from_flag(
            u32_ioctl(&file, DKIOCGETFEATURES)
                .ok()
                .map(|features| features & DK_FEATURE_UNMAP != 0),
        );

        Ok(Self {
            file,
            geometry: Geometry::new(sector_size, block_count, class),
            trim,
        })
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.geometry
    }

    pub(crate) fn trim(&self) -> TrimState {
        self.trim
    }

    pub(crate) fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError> {
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

        // The raw node reads unbuffered but, unlike `O_DIRECT`, asks nothing
        // of the buffer's alignment — only that offset and length are whole
        // blocks, which the assertion above and the geometry check establish.
        self.file
            .read_exact_at(buf, offset)
            .map_err(|source| match source.raw_os_error() {
                Some(libc::EIO) => ReadError::bad_sector(lba, sectors),
                _ => ReadError::io(lba, sectors, source),
            })
    }
}

/// Issues a `DKIOC` request that writes exactly one `u32`.
///
/// The request codes are hardcoded above, so the payload width can never
/// disagree with the local slot.
fn u32_ioctl(file: &File, request: libc::c_ulong) -> io::Result<u32> {
    let mut value: u32 = 0;
    // SAFETY: `file` owns a descriptor that stays open for the duration of the
    // call. `request` is one of the hardcoded `_IOR(…, u32)` codes above,
    // whose handler writes exactly one u32 through the pointer — matching
    // `value`, a live stack slot that outlives the call. The kernel neither
    // retains the pointer nor reads from it.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), request, &raw mut value) };
    if rc == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Issues a `DKIOC` request that writes exactly one `u64`.
fn u64_ioctl(file: &File, request: libc::c_ulong) -> io::Result<u64> {
    let mut value: u64 = 0;
    // SAFETY: as `u32_ioctl`, for the hardcoded `_IOR(…, u64)` code above.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), request, &raw mut value) };
    if rc == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}
