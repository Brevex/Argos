//! Linux native device access: `O_DIRECT` reads and geometry ioctls.
//!
//! The only `unsafe` in the workspace lives here, confined to two ioctl wrappers.
//! Devices are opened `O_RDONLY`; there is no write path to encapsulate.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, FileTypeExt, OpenOptionsExt};
use std::path::Path;

use argos_core::geometry::{Lba, SectorSize};
use argos_core::source::{DeviceClass, Geometry, ReadError};

use super::DeviceError;

/// `BLKGETSIZE64` ioctl: writes the total device size in bytes into a `u64`.
/// Encoding `_IOR(0x12, 114, size_t)`; the encoded size field follows the
/// target's `size_t` width, hence the per-width constants.
#[cfg(target_pointer_width = "64")]
const BLKGETSIZE64: libc::c_ulong = 0x8008_1272;
/// See the 64-bit `BLKGETSIZE64` doc; 4-byte `size_t` encoding.
#[cfg(target_pointer_width = "32")]
const BLKGETSIZE64: libc::c_ulong = 0x8004_1272;

/// `BLKSSZGET` ioctl: writes the logical sector size in bytes into a `c_int`.
/// Plain (non-`_IOR`) historical encoding `0x1268`.
const BLKSSZGET: libc::c_ulong = 0x1268;

/// A block device opened `O_RDONLY`, with `O_DIRECT` when the kernel allows it.
pub(crate) struct Native {
    file: File,
    geometry: Geometry,
    /// Whether reads bypass the page cache and therefore need aligned buffers.
    direct: bool,
    /// Reused bounce buffer providing the alignment `O_DIRECT` requires.
    bounce: Vec<u8>,
}

/// Manual, redacting impl: `bounce` holds raw bytes read off the evidence
/// medium after any read; its content must never render (A-NO-CONTENT-IN-LOGS).
impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Native")
            .field("file", &self.file)
            .field("geometry", &self.geometry)
            .field("direct", &self.direct)
            .field("bounce_len", &self.bounce.len())
            .finish()
    }
}

impl Native {
    pub(crate) fn open(path: &Path) -> Result<Self, DeviceError> {
        let (file, direct) = open_read_only(path)?;
        let metadata = file
            .metadata()
            .map_err(|source| DeviceError::open(path, source))?;
        if !metadata.file_type().is_block_device() {
            return Err(DeviceError::not_block_device(path));
        }

        let bytes =
            device_size_bytes(&file).map_err(|source| DeviceError::geometry(path, source))?;
        let sector_bytes =
            logical_sector_bytes(&file).map_err(|source| DeviceError::geometry(path, source))?;
        let sector_size = u32::try_from(sector_bytes)
            .ok()
            .and_then(|bytes| SectorSize::from_u32(bytes).ok())
            .ok_or_else(|| {
                DeviceError::geometry(
                    path,
                    io::Error::other(format!(
                        "kernel reported an unusable logical sector size: {sector_bytes}"
                    )),
                )
            })?;

        let geometry = Geometry::new(
            sector_size,
            bytes / sector_size.as_u64(),
            device_class(path),
        );
        Ok(Self {
            file,
            geometry,
            direct,
            bounce: Vec::new(),
        })
    }

    pub(crate) fn geometry(&self) -> Geometry {
        self.geometry
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

        let result = if self.direct {
            // O_DIRECT requires the user buffer to be aligned to the logical
            // sector size; the caller's buffer makes no such promise, so read
            // into an internally aligned bounce buffer and copy out.
            let aligned = aligned_slice(&mut self.bounce, buf.len(), sector_bytes);
            self.file
                .read_exact_at(aligned, offset)
                .map(|()| buf.copy_from_slice(aligned))
        } else {
            self.file.read_exact_at(buf, offset)
        };

        result.map_err(|source| {
            if source.raw_os_error() == Some(libc::EIO) {
                ReadError::bad_sector(lba, sectors)
            } else {
                ReadError::io(lba, sectors, source)
            }
        })
    }
}

/// Opens `path` read-only, preferring `O_DIRECT`; falls back to a cached open
/// where the filesystem rejects direct I/O (`EINVAL`).
fn open_read_only(path: &Path) -> Result<(File, bool), DeviceError> {
    let direct = File::options()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path);
    match direct {
        Ok(file) => Ok((file, true)),
        Err(source) if source.raw_os_error() == Some(libc::EINVAL) => File::options()
            .read(true)
            .open(path)
            .map(|file| (file, false))
            .map_err(|fallback| DeviceError::open(path, fallback)),
        Err(source) => Err(DeviceError::open(path, source)),
    }
}

/// Device class from the sysfs rotational flag; `Unknown` when sysfs is silent.
fn device_class(path: &Path) -> DeviceClass {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return DeviceClass::Unknown;
    };
    let rotational = std::fs::read_to_string(format!("/sys/class/block/{name}/queue/rotational"));
    match rotational.as_deref().map(str::trim) {
        Ok("1") => DeviceClass::Hdd,
        Ok("0") => DeviceClass::Ssd,
        _ => DeviceClass::Unknown,
    }
}

/// A `len`-byte view into `bounce`, aligned to `align` bytes.
fn aligned_slice(bounce: &mut Vec<u8>, len: usize, align: usize) -> &mut [u8] {
    bounce.resize(len + align, 0);
    let start = bounce.as_ptr().align_offset(align);
    &mut bounce[start..start + len]
}

/// Total size of the block device behind `file`, in bytes.
///
/// The request is hardcoded so the payload width can never disagree with the
/// local slot: `BLKGETSIZE64` is defined to write exactly one `u64`.
fn device_size_bytes(file: &File) -> io::Result<u64> {
    let mut value: u64 = 0;
    // SAFETY: `file` owns a descriptor that stays open for the duration of the
    // call. The request is the hardcoded BLKGETSIZE64, whose kernel handler
    // writes exactly one u64 through the pointer — matching `value`, a live
    // stack slot that outlives the call. The kernel neither retains the
    // pointer nor reads from it.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), BLKGETSIZE64, &raw mut value) };
    if rc == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Logical sector size of the block device behind `file`, in bytes.
///
/// The request is hardcoded so the payload width can never disagree with the
/// local slot: `BLKSSZGET` is defined to write exactly one `c_int`.
fn logical_sector_bytes(file: &File) -> io::Result<libc::c_int> {
    let mut value: libc::c_int = 0;
    // SAFETY: `file` owns a descriptor that stays open for the duration of the
    // call. The request is the hardcoded BLKSSZGET, whose kernel handler
    // writes exactly one c_int through the pointer — matching `value`, a live
    // stack slot that outlives the call. The kernel neither retains the
    // pointer nor reads from it.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), BLKSSZGET, &raw mut value) };
    if rc == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use argos_core::geometry::SectorSize;
    use argos_core::source::{DeviceClass, Geometry};

    use super::Native;

    #[test]
    #[cfg_attr(miri, ignore = "opens a real file handle")]
    fn debug_never_renders_bounce_buffer_content() {
        let native = Native {
            file: std::fs::File::open("/dev/null").expect("open /dev/null read-only"),
            geometry: Geometry::new(SectorSize::new(512), 8, DeviceClass::Unknown),
            direct: true,
            bounce: vec![0xAB; 64],
        };
        let rendered = format!("{native:?}");
        assert!(
            !rendered.contains("171") && !rendered.to_lowercase().contains("0xab"),
            "Debug output leaked bounce bytes: {rendered}"
        );
        assert!(rendered.contains("bounce_len"));
    }
}
