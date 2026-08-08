//! What disks this machine has, and whether any of them is in use.
//!
//! Two questions an examiner needs answered before a scan starts, and both are
//! evidence-handling questions rather than conveniences.
//!
//! **Which node is the whole disk.** Scanning a partition sees only what lies
//! inside it — not the partition table, not the gaps between partitions, and
//! not the residue of filesystems a re-format left behind. A tool that lets
//! someone scan `/dev/sda1` while believing they scanned the disk reports less
//! than the medium holds and never says so.
//!
//! **Whether the medium is mounted.** A mounted filesystem is being written to
//! by the operating system while Argos reads it: journal commits, atime
//! updates, defragmentation. Argos itself never writes (A-READ-ONLY), but the
//! bytes underneath it can still change mid-scan, which makes a manifest
//! describe a medium that no longer exists. This is a warning, not a refusal —
//! reading a live system is sometimes the only option — but it has to be
//! stated.
//!
//! The parsing lives in [`mount`], compiled everywhere and tested everywhere;
//! only the syscalls and the directory walks are behind `cfg`.

use std::fmt;
use std::path::PathBuf;

use argos_core::source::DeviceClass;

use crate::class::TrimState;
use crate::naming::NodeKind;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod mount;
#[cfg(windows)]
mod windows;

/// One medium this machine can be asked to scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Path to open it by.
    pub path: PathBuf,
    /// Whether this addresses the whole disk or one partition of it.
    pub kind: NodeKind,
    /// Capacity in bytes, when the platform reported one without opening the
    /// device.
    pub capacity_bytes: Option<u64>,
    /// What kind of medium the platform says it is.
    pub class: DeviceClass,
    /// Whether the medium reports TRIM enabled.
    pub trim: TrimState,
    /// Where the operating system currently has it mounted, if anywhere.
    pub mounts: Vec<MountPoint>,
    /// Model or product string, when the platform offers one without
    /// privileged access. Recorded for the manifest, never parsed.
    pub model: Option<String>,
}

impl DeviceInfo {
    /// Whether the OS currently has this medium, or something on it, mounted.
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        !self.mounts.is_empty()
    }

    /// Whether any mount of this medium is writable.
    ///
    /// A read-only mount still changes nothing underneath a scan; a writable
    /// one can, and that is the case worth warning about.
    #[must_use]
    pub fn has_writable_mount(&self) -> bool {
        self.mounts.iter().any(|mount| !mount.read_only)
    }
}

/// One place the operating system has a medium mounted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountPoint {
    /// The device node the mount names — a partition, usually, not the disk.
    pub source: PathBuf,
    /// Where it is mounted.
    pub target: PathBuf,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

impl fmt::Display for MountPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.source.display(), self.target.display())?;
        if self.read_only {
            f.write_str(" (read-only)")?;
        }
        Ok(())
    }
}

/// Every medium this machine exposes, whole disks first.
///
/// Enumeration is best-effort and never fails: a machine where the platform's
/// listing is unavailable — no `/sys`, no privileges, an OS with no HAL here —
/// yields an empty list, and the user can still name a device path directly.
/// Returning an error would suggest the scan cannot proceed, which is not
/// true.
#[must_use]
pub fn list() -> Vec<DeviceInfo> {
    let mut found = platform_list();
    // Whole disks first, then by path, so the listing is stable across runs
    // and the target a recovery scan wants is at the top.
    found.sort_by(|left, right| {
        (left.kind == NodeKind::Partition, &left.path)
            .cmp(&(right.kind == NodeKind::Partition, &right.path))
    });
    found
}

#[cfg(target_os = "linux")]
fn platform_list() -> Vec<DeviceInfo> {
    linux::list()
}

#[cfg(target_os = "macos")]
fn platform_list() -> Vec<DeviceInfo> {
    macos::list()
}

#[cfg(windows)]
fn platform_list() -> Vec<DeviceInfo> {
    windows::list()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_list() -> Vec<DeviceInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use argos_core::source::DeviceClass;

    use super::{DeviceInfo, MountPoint, list};
    use crate::class::TrimState;
    use crate::naming::NodeKind;

    fn info(path: &str, kind: NodeKind, mounts: Vec<MountPoint>) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from(path),
            kind,
            capacity_bytes: None,
            class: DeviceClass::Unknown,
            trim: TrimState::Unknown,
            mounts,
            model: None,
        }
    }

    fn mount(source: &str, target: &str, read_only: bool) -> MountPoint {
        MountPoint {
            source: PathBuf::from(source),
            target: PathBuf::from(target),
            read_only,
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "walks the real /sys and /dev")]
    fn enumeration_never_fails_however_little_the_platform_offers() {
        // A machine that cannot list its disks is not a machine that cannot
        // scan one: the user can always name a path. This must not panic on
        // any platform, including those with no HAL at all.
        let _ = list();
    }

    #[test]
    fn a_writable_mount_is_told_from_a_read_only_one() {
        let read_only = info(
            "/dev/sda",
            NodeKind::WholeDisk,
            vec![mount("/dev/sda1", "/mnt/evidence", true)],
        );
        assert!(read_only.is_mounted());
        assert!(
            !read_only.has_writable_mount(),
            "a read-only mount changes nothing underneath a scan"
        );

        let writable = info(
            "/dev/sda",
            NodeKind::WholeDisk,
            vec![
                mount("/dev/sda1", "/boot", true),
                mount("/dev/sda2", "/", false),
            ],
        );
        assert!(writable.has_writable_mount());
    }

    #[test]
    fn an_unmounted_device_reports_neither() {
        let idle = info("/dev/sdb", NodeKind::WholeDisk, Vec::new());
        assert!(!idle.is_mounted());
        assert!(!idle.has_writable_mount());
    }
}
