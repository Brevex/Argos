//! What media this machine offers, and whether any of them is in use.
//!
//! Three questions an examiner needs answered before a scan starts, and all
//! three are evidence-handling questions rather than conveniences.
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
//! **What snapshots the platform already holds.** Windows keeps periodic
//! read-only Volume Shadow Copies of its volumes. For a recovery they are
//! unusually good evidence: a file deleted last week is *present*, in full, in
//! the snapshot taken before the deletion — no carving, no reassembly, no
//! confidence tier below the top. A scan that ignores them reconstructs from
//! fragments what was sitting there intact. Each snapshot appears in the NT
//! object namespace as `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN` and
//! opens like any other volume, so enumeration is a probe of that namespace
//! keeping the ones that open read-only.
//!
//! **What this deliberately does not do.** The VSS COM API
//! (`IVssBackupComponents`) would additionally report each snapshot's creation
//! time and which volume it belongs to. Argos does not use it: it requires COM
//! initialisation and, for most queries, a backup-operator privilege the tool
//! otherwise never needs. The consequence is stated rather than hidden — a
//! snapshot's provenance records the device path it was read from and nothing
//! more, so an examiner correlating a recovery to a point in time must get the
//! timestamp from Windows itself (A-PROVENANCE). Recovered *files* still carry
//! the timestamps their filesystem metadata holds, which is the evidence that
//! matters most.
//!
//! The mount-table parsing lives in [`mount`](crate::mount), compiled everywhere and
//! tested everywhere; only the syscalls and the directory walks are behind
//! `cfg`.

use std::fmt;
use std::path::PathBuf;

use argos_core::ports::DeviceClass;

use crate::class::TrimState;
use crate::naming::NodeKind;

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
    crate::platform::linux::list()
}

#[cfg(target_os = "macos")]
fn platform_list() -> Vec<DeviceInfo> {
    crate::platform::macos::list()
}

#[cfg(windows)]
fn platform_list() -> Vec<DeviceInfo> {
    crate::platform::windows::list()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_list() -> Vec<DeviceInfo> {
    Vec::new()
}

/// One shadow copy that can be opened and read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowCopy {
    /// Device path to open it by.
    pub path: PathBuf,
    /// The snapshot's index in the object namespace.
    pub index: u32,
}

/// Prefix of a shadow copy's device path in the NT object namespace.
pub(crate) const SHADOW_COPY_PREFIX: &str = r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy";

/// The device path of shadow copy `index`.
#[must_use]
pub fn shadow_copy_path(index: u32) -> String {
    format!("{SHADOW_COPY_PREFIX}{index}")
}

/// Whether a path names a shadow copy, and which one.
///
/// `None` for anything else. Used to tell a snapshot source apart from a live
/// volume when a recovery is attributed.
#[must_use]
pub fn shadow_copy_index(path: &str) -> Option<u32> {
    // The namespace is case-insensitive, as the rest of the Win32 device
    // namespace is.
    let lower = path.to_ascii_lowercase();
    let prefix = SHADOW_COPY_PREFIX.to_ascii_lowercase();
    lower.strip_prefix(&prefix)?.parse().ok()
}

/// Highest shadow-copy index probed.
///
/// Windows numbers snapshots monotonically from 1 and does not reuse indices,
/// so a machine that has taken and released many snapshots over its life can
/// have live ones well above its snapshot count. Probing to here covers a
/// system with years of restore points; each probe is one handle open
/// (`M-DOCUMENTED-MAGIC`, A-BOUNDED-ALLOC).
pub const MAX_SHADOW_COPY_INDEX: u32 = 1024;

/// Every shadow copy this machine currently holds.
///
/// Empty on platforms without shadow copies, and on Windows when none exist
/// or none can be opened. Enumeration never fails: a machine whose snapshots
/// cannot be listed is one whose live volumes can still be scanned.
#[must_use]
pub fn shadow_copies() -> Vec<ShadowCopy> {
    platform_shadow_copies()
}

#[cfg(windows)]
fn platform_shadow_copies() -> Vec<ShadowCopy> {
    (1..=MAX_SHADOW_COPY_INDEX)
        .filter_map(|index| {
            let path = shadow_copy_path(index);
            // A snapshot that opens is a snapshot that exists. Opening
            // read-only is also the only access Argos ever wants of one.
            std::fs::File::open(&path).ok().map(|_| ShadowCopy {
                path: PathBuf::from(path),
                index,
            })
        })
        .collect()
}

#[cfg(not(windows))]
fn platform_shadow_copies() -> Vec<ShadowCopy> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use argos_core::ports::DeviceClass;

    use super::{DeviceInfo, MountPoint, list, shadow_copies, shadow_copy_index, shadow_copy_path};
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

    #[test]
    fn a_shadow_copy_path_round_trips_through_its_index() {
        for index in [1_u32, 2, 17, 999, 1024] {
            let path = shadow_copy_path(index);
            assert_eq!(shadow_copy_index(&path), Some(index), "{path}");
        }
    }

    #[test]
    fn a_live_volume_is_not_mistaken_for_a_snapshot() {
        // Attribution depends on this: a recovery from a snapshot and one
        // from the live volume are different evidence, and the manifest must
        // not confuse them.
        assert_eq!(shadow_copy_index(r"\\.\PhysicalDrive0"), None);
        assert_eq!(shadow_copy_index(r"\\.\HarddiskVolume2"), None);
        assert_eq!(shadow_copy_index(r"C:\images\disk.img"), None);
        assert_eq!(shadow_copy_index("/dev/sda"), None);
        // The prefix without an index is not a snapshot either.
        assert_eq!(
            shadow_copy_index(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy"),
            None
        );
    }

    #[test]
    fn the_namespace_is_matched_case_insensitively() {
        // The Win32 device namespace is case-insensitive, and a path that
        // came back from an API may not match the case written here.
        assert_eq!(
            shadow_copy_index(r"\\?\globalroot\device\harddiskvolumeshadowcopy7"),
            Some(7)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "probes the real device namespace on Windows")]
    fn enumeration_never_fails_on_any_platform() {
        // On everything but Windows this is empty; nowhere does it panic or
        // stop a scan.
        let _ = shadow_copies();
    }
}
