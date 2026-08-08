//! Windows native device access: `CreateFileW` on `\\.\PhysicalDriveN`, disk
//! geometry and storage-property ioctls.
//!
//! Two `unsafe` surfaces only, and both are one call wide: opening the handle,
//! and `DeviceIoControl`. Everything after the open goes through a `File`
//! built from the owned handle, so the read path is ordinary safe Rust —
//! exactly as the Linux HAL reads through `read_exact_at`.
//!
//! The handle is opened `GENERIC_READ` with no write access requested. There
//! is no write path here to review (A-READ-ONLY).

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::FileExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use argos_core::geometry::{Lba, SectorSize};
use argos_core::source::{Geometry, ReadError};
use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, DEVICE_TRIM_DESCRIPTOR, GET_LENGTH_INFORMATION,
    IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty, StorageDeviceTrimProperty,
};

use crate::class::{self, TrimState};
use crate::naming;

use super::DeviceError;

use windows_sys::Win32::System::Ioctl::{DISK_GEOMETRY, IOCTL_DISK_GET_DRIVE_GEOMETRY};

/// A physical drive opened `GENERIC_READ`, unbuffered.
///
/// `FILE_FLAG_NO_BUFFERING` is the counterpart of Linux' `O_DIRECT`: a
/// full-surface sweep must not evict the machine's page cache, and must not be
/// served pages some other process happened to have read. It obliges every
/// read to be sector-aligned in both offset and length, which the bounce
/// buffer below provides.
pub(crate) struct Native {
    file: File,
    geometry: Geometry,
    trim: TrimState,
    /// Reused bounce buffer providing the alignment unbuffered I/O requires.
    bounce: Vec<u8>,
}

/// Manual, redacting impl: `bounce` holds raw bytes read off the evidence
/// medium after any read; its content must never render (A-NO-CONTENT-IN-LOGS).
impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Native")
            .field("file", &self.file)
            .field("geometry", &self.geometry)
            .field("trim", &self.trim)
            // Deliberately the length and not the bytes.
            .field("bounce_len", &self.bounce.len())
            .finish()
    }
}

impl Native {
    pub(crate) fn open(path: &Path) -> Result<Self, DeviceError> {
        let text = path.to_string_lossy();
        // A path with no device-namespace convention is not something to open
        // and hope: an image file belongs to `ImageSource`, and a drive letter
        // would scan one partition while reporting a whole-disk recovery.
        //
        // Shadow copies are the other thing worth opening here. They live in
        // their own namespace rather than under `\\.\`, so `windows_node_kind`
        // does not recognise them — and without this they would every one be
        // refused, which would make the snapshots the `devices` listing
        // advertises impossible to actually scan.
        let known = naming::windows_node_kind(&text).is_some()
            || crate::shadow::shadow_copy_index(&text).is_some();
        if !known {
            return Err(DeviceError::not_block_device(path));
        }

        let file = open_read_only(path)?;
        let bytes = length_bytes(&file).map_err(|source| DeviceError::geometry(path, source))?;
        let sector_bytes =
            logical_sector_bytes(&file).map_err(|source| DeviceError::geometry(path, source))?;
        let sector_size = SectorSize::from_u32(sector_bytes).map_err(|_unusable| {
            DeviceError::geometry(
                path,
                io::Error::other(format!(
                    "the storage driver reported an unusable logical sector size: {sector_bytes}"
                )),
            )
        })?;

        // Neither property query is required to succeed: several USB bridges
        // and every virtual disk decline them, and an unanswered query means
        // `Unknown`, never a guess.
        let class = class::from_seek_penalty(seek_penalty(&file));
        let trim = TrimState::from_flag(trim_enabled(&file));

        Ok(Self {
            file,
            geometry: Geometry::new(sector_size, bytes / sector_size.as_u64(), class),
            trim,
            bounce: Vec::new(),
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

        // Unbuffered reads need an aligned buffer; the caller's makes no such
        // promise, so read into an aligned bounce buffer and copy out.
        let aligned = crate::device::aligned_slice(&mut self.bounce, buf.len(), sector_bytes);
        let result =
            read_exact_at(&self.file, aligned, offset).map(|()| buf.copy_from_slice(aligned));

        result.map_err(|source| {
            // `ERROR_CRC` (23) and `ERROR_SECTOR_NOT_FOUND` (27) are what a
            // failing medium reports; everything else is an I/O fault the
            // report keeps separate.
            const ERROR_CRC: i32 = 23;
            const ERROR_SECTOR_NOT_FOUND: i32 = 27;
            match source.raw_os_error() {
                Some(ERROR_CRC | ERROR_SECTOR_NOT_FOUND) => ReadError::bad_sector(lba, sectors),
                _ => ReadError::io(lba, sectors, source),
            }
        })
    }
}

/// Reads exactly `buf.len()` bytes at `offset`.
///
/// Windows has no `read_exact_at`; `seek_read` may come back short at a device
/// boundary or under memory pressure, so the loop is written out. A short read
/// that reaches zero bytes is the medium ending early, which is an error here
/// because the range was already checked against the geometry.
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the device ended inside a range its geometry said it held",
                ));
            }
            Ok(read) => {
                buf = &mut buf[read..];
                offset = offset.saturating_add(read as u64);
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => return Err(source),
        }
    }
    Ok(())
}

/// Opens `path` for reading only, unbuffered.
///
/// `FILE_SHARE_READ | FILE_SHARE_WRITE` is required rather than permissive:
/// Windows keeps a handle on any disk carrying a mounted volume, and refusing
/// to share would make every scan of a live system fail. Sharing *write* does
/// not grant Argos write access — the handle is `GENERIC_READ`.
fn open_read_only(path: &Path) -> Result<File, DeviceError> {
    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `wide` is a null-terminated UTF-16 string that outlives the
    // call, which is `CreateFileW`'s only requirement of the name. Every other
    // argument is a constant or null; the security-attributes and template
    // parameters are null, which the API documents as "defaults, no
    // inheritance". The call returns an owned handle or the sentinel, and
    // nothing is retained by the OS beyond the handle we take ownership of
    // below.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Err(DeviceError::open(path, io::Error::last_os_error()));
    }

    // SAFETY: `raw` is a valid, exclusively-owned handle that `CreateFileW`
    // just returned and that nothing else holds; the sentinel and null were
    // rejected above. Ownership transfers to `OwnedHandle`, which closes it
    // exactly once on drop.
    let owned = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    Ok(File::from(owned))
}

/// Total addressable length of the device behind `file`, in bytes.
fn length_bytes(file: &File) -> io::Result<u64> {
    let mut reply = [0_u8; size_of::<GET_LENGTH_INFORMATION>()];
    control(file, IOCTL_DISK_GET_LENGTH_INFO, &[], &mut reply)?;
    let at = std::mem::offset_of!(GET_LENGTH_INFORMATION, Length);
    let length = read_i64(&reply, at)
        .ok_or_else(|| io::Error::other("the storage driver answered a short length reply"))?;
    u64::try_from(length).map_err(|_negative| {
        io::Error::other("the storage driver reported a negative device length")
    })
}

/// Logical sector size of the device behind `file`, in bytes.
fn logical_sector_bytes(file: &File) -> io::Result<u32> {
    let mut reply = [0_u8; size_of::<DISK_GEOMETRY>()];
    control(file, IOCTL_DISK_GET_DRIVE_GEOMETRY, &[], &mut reply)?;
    let at = std::mem::offset_of!(DISK_GEOMETRY, BytesPerSector);
    read_u32(&reply, at)
        .ok_or_else(|| io::Error::other("the storage driver answered a short geometry reply"))
}

/// Whether the medium incurs a seek penalty, when the driver will say.
fn seek_penalty(file: &File) -> Option<bool> {
    let mut reply = [0_u8; size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()];
    let query = property_query(StorageDeviceSeekPenaltyProperty);
    control(file, IOCTL_STORAGE_QUERY_PROPERTY, &query, &mut reply).ok()?;
    let at = std::mem::offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty);
    read_boolean(&reply, at)
}

/// Whether the medium reports TRIM enabled, when the driver will say.
fn trim_enabled(file: &File) -> Option<bool> {
    let mut reply = [0_u8; size_of::<DEVICE_TRIM_DESCRIPTOR>()];
    let query = property_query(StorageDeviceTrimProperty);
    control(file, IOCTL_STORAGE_QUERY_PROPERTY, &query, &mut reply).ok()?;
    let at = std::mem::offset_of!(DEVICE_TRIM_DESCRIPTOR, TrimEnabled);
    read_boolean(&reply, at)
}

/// A standard-query descriptor request for `property`, as its wire bytes.
fn property_query(property: i32) -> [u8; size_of::<STORAGE_PROPERTY_QUERY>()] {
    let mut query = [0_u8; size_of::<STORAGE_PROPERTY_QUERY>()];
    let property_at = std::mem::offset_of!(STORAGE_PROPERTY_QUERY, PropertyId);
    let query_at = std::mem::offset_of!(STORAGE_PROPERTY_QUERY, QueryType);
    query[property_at..property_at + 4].copy_from_slice(&property.to_le_bytes());
    query[query_at..query_at + 4].copy_from_slice(&PropertyStandardQuery.to_le_bytes());
    query
}

/// Issues `code` on `file`, sending `input` and filling `reply`.
///
/// Both payloads are plain byte buffers, and every field is read back out at
/// its own offset by the callers above. That is deliberate rather than
/// incidental: the obvious shape here is a generic over the reply *type* with
/// `MaybeUninit::assume_init`, and it is unsound. Two of the descriptors this
/// module reads — `DEVICE_SEEK_PENALTY_DESCRIPTOR` and
/// `DEVICE_TRIM_DESCRIPTOR` — carry a Win32 `BOOLEAN`, which is an unsigned
/// char whose contract is *any non-zero byte is true*, and which the bindings
/// project as a Rust `bool`, whose validity invariant is exactly `0` or `1`.
/// A driver answering `0xFF` — legal, and what several storage filters
/// write — would make `assume_init` produce an invalid `bool` and hand
/// undefined behaviour to safe code (`M-UNSOUND`). Bytes have no validity
/// invariant, so there is nothing left to get wrong.
///
/// The reply must arrive whole: a driver that returns fewer bytes than the
/// caller's buffer has answered a different question, and the remainder would
/// be the zeros this buffer started as rather than anything the medium said.
fn control(file: &File, code: u32, input: &[u8], reply: &mut [u8]) -> io::Result<usize> {
    let mut returned: u32 = 0;
    let input_len = u32::try_from(input.len())
        .map_err(|_too_large| io::Error::other("ioctl input does not fit a u32 length"))?;
    let reply_len = u32::try_from(reply.len())
        .map_err(|_too_large| io::Error::other("ioctl reply does not fit a u32 length"))?;

    // SAFETY: the handle is live for the call. The input pointer addresses
    // `input`, which is `input.len()` readable bytes and outlives the call;
    // the output pointer addresses `reply`, which is `reply.len()` writable
    // bytes and likewise outlives it — both lengths come from those very
    // slices, so neither can disagree with its buffer. `returned` is a live
    // `u32` slot. The overlapped pointer is null, which for a synchronous
    // handle means the call does not return until the buffers are no longer
    // in use, so nothing is retained.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle().cast(),
            code,
            input.as_ptr().cast(),
            input_len,
            reply.as_mut_ptr().cast(),
            reply_len,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if returned < reply_len {
        return Err(io::Error::other(format!(
            "the storage driver answered {returned} bytes where the reply needs {reply_len}"
        )));
    }
    Ok(returned as usize)
}

/// A Win32 `BOOLEAN` at `offset`: one byte, any non-zero value meaning true.
fn read_boolean(buffer: &[u8], offset: usize) -> Option<bool> {
    buffer.get(offset).map(|byte| *byte != 0)
}

/// A little-endian `u32` at `offset`, when the buffer holds one there.
fn read_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    let bytes = buffer.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

/// A little-endian `i64` at `offset`, when the buffer holds one there.
fn read_i64(buffer: &[u8], offset: usize) -> Option<i64> {
    let bytes = buffer.get(offset..offset.checked_add(8)?)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}
