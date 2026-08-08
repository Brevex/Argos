//! macOS device enumeration, from `/dev` and `getfsstat`.
//!
//! macOS has no sysfs. The disks are the `/dev/diskN` nodes themselves, and
//! the mount table comes from `getfsstat(2)`, which is the same list `mount`
//! prints. Capacity and class need the device open, so they are left for
//! [`Device::open`](crate::Device::open) rather than guessed at here —
//! enumeration must not require the privileges a scan does, or listing the
//! disks would fail for an unprivileged user who only wanted to see them.

use std::path::PathBuf;

use crate::class::TrimState;
use crate::naming;

use super::{DeviceInfo, MountPoint, mount};

pub(super) fn list() -> Vec<DeviceInfo> {
    let mounts = read_mounts();
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // The raw and buffered nodes are the same medium listed twice; keep
        // the buffered name, which is what users and `diskutil` speak, and
        // let the HAL rewrite it to the raw node when it opens.
        if name.starts_with('r') {
            continue;
        }
        let path = PathBuf::from(format!("/dev/{name}"));
        let Some(kind) = naming::macos_node_kind(&path.to_string_lossy()) else {
            continue;
        };
        found.push(DeviceInfo {
            mounts: mount::mounts_of(&path, &mounts, naming::macos_whole_disk),
            path,
            kind,
            // Both need the device open, which needs privileges enumeration
            // must not demand.
            capacity_bytes: None,
            class: argos_core::source::DeviceClass::Unknown,
            trim: TrimState::Unknown,
            model: None,
        });
    }
    found
}

/// The mount table, via `getfsstat(2)`.
///
/// Called twice on purpose: once with a null buffer to learn the count, then
/// once to fill. The count can grow between the two calls — someone may mount
/// a volume — so the second call's return value bounds what is read, never the
/// first's.
fn read_mounts() -> Vec<MountPoint> {
    // SAFETY: a null buffer with a zero size is the documented way to ask
    // `getfsstat` for the number of mounted filesystems; it writes nothing.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    if count == 0 {
        return Vec::new();
    }

    let mut buffer: Vec<libc::statfs> = Vec::with_capacity(count);
    let Ok(bytes) = libc::c_int::try_from(count * size_of::<libc::statfs>()) else {
        return Vec::new();
    };
    // SAFETY: `buffer` has capacity for `count` `statfs` records and the size
    // passed is exactly that capacity in bytes, so the kernel cannot write
    // past it. The pointer is valid for the call and the allocation outlives
    // it. The return value is the number of records actually written, which
    // bounds the `set_len` below.
    let written = unsafe { libc::getfsstat(buffer.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    let Ok(written) = usize::try_from(written) else {
        return Vec::new();
    };
    let written = written.min(count);
    // SAFETY: `getfsstat` initialised exactly `written` records, and `written`
    // is clamped to the capacity allocated above.
    unsafe { buffer.set_len(written) };

    buffer
        .iter()
        .map(|entry| MountPoint {
            source: PathBuf::from(c_field_to_string(&entry.f_mntfromname)),
            target: PathBuf::from(c_field_to_string(&entry.f_mntonname)),
            read_only: entry.f_flags & u32::try_from(libc::MNT_RDONLY).unwrap_or(0) != 0,
        })
        // A source that is not a path is a pseudo-filesystem and belongs to no
        // medium, exactly as on Linux.
        .filter(|point| point.source.starts_with("/dev/"))
        .collect()
}

/// A fixed-size, null-padded C string field as a `String`.
fn c_field_to_string(field: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| byte.cast_unsigned())
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
