//! Linux device enumeration, from sysfs.
//!
//! `/sys/block` lists every block device the kernel knows, and each entry
//! carries its size, its partitions and its queue attributes as plain files.
//! Nothing here needs privileges or `unsafe`; the decisions all live in
//! [`naming`](crate::naming), [`class`](crate::class) and
//! [`mount`](super::mount).

use std::path::{Path, PathBuf};

use crate::class::{self, TrimState};
use crate::naming::{self, NodeKind};

use super::{DeviceInfo, mount};

/// Bytes per sector in the units `/sys/block/*/size` counts.
///
/// The kernel reports that file in 512-byte units regardless of the device's
/// real logical sector size — a long-standing quirk of the interface, not a
/// property of the medium (`M-DOCUMENTED-MAGIC`). The true sector size comes
/// from an ioctl at open time.
const SYSFS_SIZE_UNIT: u64 = 512;

pub(super) fn list() -> Vec<DeviceInfo> {
    let mounts = read_mounts();
    let Ok(entries) = std::fs::read_dir("/sys/block") else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let sysfs = entry.path();
        let path = PathBuf::from(format!("/dev/{name}"));
        // Only nodes whose convention this build understands; a name with no
        // convention is skipped rather than presented as a disk.
        let Some(kind) = naming::linux_node_kind(&path.to_string_lossy()) else {
            continue;
        };
        found.push(describe(&path, kind, &sysfs, &mounts));

        // Partitions are subdirectories of their disk, carrying a `partition`
        // file and their own `size`.
        let Ok(children) = std::fs::read_dir(&sysfs) else {
            continue;
        };
        for child in children.flatten() {
            let child_name = child.file_name();
            let Some(child_name) = child_name.to_str() else {
                continue;
            };
            if !child.path().join("partition").exists() {
                continue;
            }
            let child_path = PathBuf::from(format!("/dev/{child_name}"));
            found.push(describe(
                &child_path,
                NodeKind::Partition,
                &child.path(),
                &mounts,
            ));
        }
    }
    found
}

/// Everything sysfs will say about one node without opening it.
fn describe(path: &Path, kind: NodeKind, sysfs: &Path, mounts: &[super::MountPoint]) -> DeviceInfo {
    // A partition's queue attributes live on its parent disk.
    let queue = if kind == NodeKind::Partition {
        sysfs.parent().map(Path::to_path_buf)
    } else {
        Some(sysfs.to_path_buf())
    };
    let attribute = |name: &str| {
        queue
            .as_ref()
            .and_then(|base| std::fs::read_to_string(base.join("queue").join(name)).ok())
    };

    DeviceInfo {
        path: path.to_path_buf(),
        kind,
        capacity_bytes: std::fs::read_to_string(sysfs.join("size"))
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .and_then(|sectors| sectors.checked_mul(SYSFS_SIZE_UNIT)),
        class: class::from_rotational(attribute("rotational").as_deref()),
        trim: TrimState::from_flag(
            attribute("discard_max_bytes")
                .and_then(|text| text.trim().parse::<u64>().ok())
                .map(|bytes| bytes > 0),
        ),
        mounts: mount::mounts_of(path, mounts, naming::linux_whole_disk),
        model: std::fs::read_to_string(sysfs.join("device").join("model"))
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty()),
    }
}

/// The kernel's mount table, or nothing when it cannot be read.
fn read_mounts() -> Vec<super::MountPoint> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|text| mount::parse_linux_mountinfo(&text))
        .unwrap_or_default()
}
