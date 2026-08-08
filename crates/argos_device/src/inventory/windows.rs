//! Windows device enumeration, from the device namespace and the volume list.
//!
//! Two walks that meet in the middle. `\\.\PhysicalDriveN` is probed for the
//! disks; `FindFirstVolumeW` walks the volumes, and each volume's disk extents
//! say which physical drive it sits on. Joining them is what lets a scan of
//! `\\.\PhysicalDrive0` know that `C:` is mounted on it and warn accordingly.
//!
//! Every handle here is opened with **no access rights at all** — `0` for
//! `dwDesiredAccess`, which Windows documents as a query-only handle. That is
//! what lets an unprivileged user list their disks: asking for `GENERIC_READ`
//! on a physical drive requires Administrator, and a listing that needed
//! elevation would be a listing nobody could get before deciding to elevate.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, MAX_PATH};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW,
    FindVolumeClose, GetVolumeInformationW, GetVolumePathNamesForVolumeNameW,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{DISK_EXTENT, VOLUME_DISK_EXTENTS};
use windows_sys::Win32::System::SystemServices::FILE_READ_ONLY_VOLUME;

use crate::class::TrimState;
use crate::naming::{self, NodeKind};

use super::{DeviceInfo, MountPoint};

/// Highest physical drive index probed.
///
/// Windows numbers drives from zero with no gaps in practice, but a machine
/// with many enclosures can reach into the dozens. The probe stops at this
/// index rather than at the first miss, so one absent number does not hide the
/// drives above it; each probe is a handle open that costs microseconds
/// (`M-DOCUMENTED-MAGIC`).
const MAX_PHYSICAL_DRIVE: u32 = 64;

/// Bytes of the `VOLUME_DISK_EXTENTS` head, before its extent array.
const EXTENTS_HEAD_BYTES: usize = size_of::<VOLUME_DISK_EXTENTS>();

/// Bytes of one `DISK_EXTENT` record.
const EXTENT_BYTES: usize = size_of::<DISK_EXTENT>();

/// Disk extents one volume may report before the rest are ignored.
///
/// A volume spanning more disks than this is a storage-space or dynamic-disk
/// arrangement Argos does not recover from; reading the first few still
/// attributes it to a disk for the mount warning (A-BOUNDED-ALLOC).
const MAX_VOLUME_EXTENTS: usize = 16;

/// UTF-16 code units in the volume-name buffer.
///
/// `cchBufferLength` counts code units, not bytes, which is why the buffer is
/// `[u16; _]` and this is passed unscaled. `MAX_PATH` is 260, so the `u32` the
/// calls take cannot truncate it.
const VOLUME_NAME_UNITS: u32 = MAX_PATH;

pub(super) fn list() -> Vec<DeviceInfo> {
    let volumes = read_volumes();
    let mut found = Vec::new();
    for index in 0..MAX_PHYSICAL_DRIVE {
        let path = PathBuf::from(naming::windows_physical_drive(index));
        let Some(handle) = query_handle(&path) else {
            continue;
        };
        drop(handle);
        found.push(DeviceInfo {
            mounts: volumes
                .iter()
                .filter(|volume| volume.disks.contains(&index))
                .flat_map(|volume| {
                    volume.mount_points.iter().map(|target| MountPoint {
                        source: PathBuf::from(volume.name.clone()),
                        target: PathBuf::from(target),
                        read_only: volume.read_only,
                    })
                })
                .collect(),
            path,
            kind: NodeKind::WholeDisk,
            // Both need a `GENERIC_READ` handle, which needs Administrator.
            // Enumeration must work without it.
            capacity_bytes: None,
            class: argos_core::source::DeviceClass::Unknown,
            trim: TrimState::Unknown,
            model: None,
        });
    }
    found
}

/// One volume the OS knows, and where it lives.
struct Volume {
    /// The `\\?\Volume{GUID}\` name.
    name: String,
    /// Physical drive indices the volume occupies.
    disks: Vec<u32>,
    /// Drive letters and mounted folders it is reachable through.
    mount_points: Vec<String>,
    /// Whether the volume is mounted read-only.
    read_only: bool,
}

/// Opens `path` for querying only: no read, no write, no access at all.
fn query_handle(path: &std::path::Path) -> Option<OwnedHandle> {
    let wide = wide(&path.to_string_lossy());
    // SAFETY: `wide` is a null-terminated UTF-16 string outliving the call.
    // `dwDesiredAccess` is zero — the documented query-only handle, which
    // grants neither read nor write. The security-attributes and template
    // parameters are null, which the API documents as defaults. The returned
    // handle is taken into an `OwnedHandle` below, which closes it once.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a valid handle `CreateFileW` just returned and nothing
    // else holds; the sentinel and null were rejected above.
    Some(unsafe { OwnedHandle::from_raw_handle(raw.cast()) })
}

/// Every volume the OS knows, with its disks and mount points.
fn read_volumes() -> Vec<Volume> {
    let mut volumes = Vec::new();
    let mut name = [0_u16; VOLUME_NAME_UNITS as usize];

    // SAFETY: `name` is a live buffer of exactly the length passed. The call
    // writes a null-terminated name into it and returns a search handle or the
    // sentinel.
    let search = unsafe { FindFirstVolumeW(name.as_mut_ptr(), VOLUME_NAME_UNITS) };
    if search == INVALID_HANDLE_VALUE || search.is_null() {
        return volumes;
    }

    loop {
        if let Some(volume) = describe_volume(&name) {
            volumes.push(volume);
        }
        // SAFETY: `search` is the live handle from `FindFirstVolumeW`, not yet
        // closed; `name` is a live buffer of exactly the length passed.
        let more = unsafe { FindNextVolumeW(search, name.as_mut_ptr(), VOLUME_NAME_UNITS) };
        if more == 0 {
            break;
        }
    }
    // SAFETY: `search` is the live handle from `FindFirstVolumeW` and is not
    // used again after this call.
    unsafe { FindVolumeClose(search) };
    volumes
}

/// What one volume name resolves to.
fn describe_volume(name: &[u16]) -> Option<Volume> {
    let text = from_wide(name);
    if text.is_empty() {
        return None;
    }

    // `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` needs the name without its
    // trailing backslash; `GetVolumePathNamesForVolumeNameW` needs it with.
    let device_name = text.trim_end_matches('\\').to_owned();
    let disks = volume_disks(&device_name);
    Some(Volume {
        mount_points: volume_mount_points(&text),
        read_only: volume_is_read_only(&text),
        name: text,
        disks,
        // `read_only` and `mount_points` are set above; the struct order is
        // for readability, not initialisation order.
    })
}

/// Physical drive indices the volume at `device_name` occupies.
fn volume_disks(device_name: &str) -> Vec<u32> {
    let Some(handle) = query_handle(std::path::Path::new(device_name)) else {
        return Vec::new();
    };

    // `VOLUME_DISK_EXTENTS` is variable-length: a count followed by that many
    // `DISK_EXTENT`s. The buffer is sized for the cap above, and every field
    // is read out explicitly at its own offset rather than transmuted — the
    // same discipline every on-disk structure in this workspace follows.
    let mut buffer = vec![0_u8; EXTENTS_HEAD_BYTES + MAX_VOLUME_EXTENTS * EXTENT_BYTES];
    let mut returned: u32 = 0;

    // SAFETY: the handle is live for the call. The input pointer is null with
    // a zero length, which this request takes. The output pointer addresses
    // `buffer`, which is exactly the number of writable bytes passed and
    // outlives the call; `returned` is a live `u32` slot. The overlapped
    // pointer is null, so the call is synchronous and retains nothing.
    let ok = unsafe {
        DeviceIoControl(
            handle.as_raw_handle().cast(),
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            std::ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            u32::try_from(buffer.len()).unwrap_or(u32::MAX),
            &raw mut returned,
            std::ptr::null_mut(),
        )
    };
    // A driver that answered less than the head has answered a different
    // question; the rest of the buffer is the zeros it started as, and reading
    // an extent count out of those would attribute the volume to disk zero.
    if ok == 0 || (returned as usize) < EXTENTS_HEAD_BYTES {
        return Vec::new();
    }

    let count_at = std::mem::offset_of!(VOLUME_DISK_EXTENTS, NumberOfDiskExtents);
    let extents_at = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
    let disk_at = std::mem::offset_of!(DISK_EXTENT, DiskNumber);

    let Some(count) = read_u32(&buffer, count_at) else {
        return Vec::new();
    };
    let count = (count as usize).min(MAX_VOLUME_EXTENTS);
    (0..count)
        .filter_map(|index| read_u32(&buffer, extents_at + index * EXTENT_BYTES + disk_at))
        .collect()
}

/// Drive letters and mounted folders the volume is reachable through.
fn volume_mount_points(volume: &str) -> Vec<String> {
    let wide_name = wide(volume);
    let mut buffer = vec![0_u16; 1024];
    let mut needed: u32 = 0;

    // SAFETY: both pointers address live buffers; the length passed is exactly
    // `buffer`'s length and `needed` is a live `u32` slot. The name is a
    // null-terminated UTF-16 string outliving the call.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            wide_name.as_ptr(),
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).unwrap_or(u32::MAX),
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Vec::new();
    }
    // The reply is a sequence of null-terminated strings ending in an empty
    // one. A volume with no mount point — an unlettered recovery partition —
    // yields nothing, which is the correct answer.
    buffer
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// Whether the volume is mounted read-only.
fn volume_is_read_only(volume: &str) -> bool {
    let wide_name = wide(volume);
    let mut flags: u32 = 0;
    // SAFETY: the name is a null-terminated UTF-16 string outliving the call.
    // Every buffer pointer is null with a zero length, which the API takes for
    // the fields the caller does not want; `flags` is a live `u32` slot.
    let ok = unsafe {
        GetVolumeInformationW(
            wide_name.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    ok != 0 && flags & FILE_READ_ONLY_VOLUME != 0
}

/// A little-endian `u32` at `offset`, when the buffer holds one there.
fn read_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    let bytes = buffer.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

/// A null-terminated UTF-16 encoding of `text`.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The string in a null-terminated UTF-16 buffer.
fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
