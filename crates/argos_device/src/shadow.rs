//! Volume Shadow Copies: the snapshots a live Windows system already holds.
//!
//! Windows keeps periodic read-only snapshots of its volumes. For a recovery
//! they are unusually good evidence: a file deleted last week is *present*, in
//! full, in the snapshot taken before the deletion — no carving, no
//! reassembly, no confidence tier below the top. A scan that ignores them
//! reconstructs from fragments what was sitting there intact.
//!
//! Each snapshot appears in the NT object namespace as
//! `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN`, and opens like any other
//! volume. Argos enumerates them by probing that namespace and keeping the
//! ones that open read-only.
//!
//! **What this deliberately does not do.** The VSS COM API
//! (`IVssBackupComponents`) would additionally report each snapshot's creation
//! time and which volume it belongs to. Argos does not use it: it requires
//! COM initialisation and, for most queries, a backup-operator privilege the
//! tool otherwise never needs. The consequence is stated rather than hidden —
//! a snapshot's provenance records the device path it was read from and
//! nothing more, so an examiner correlating a recovery to a point in time must
//! get the timestamp from Windows itself (A-PROVENANCE). Recovered *files*
//! still carry the timestamps their filesystem metadata holds, which is the
//! evidence that matters most.

use std::path::PathBuf;

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
pub fn list() -> Vec<ShadowCopy> {
    platform_list()
}

#[cfg(windows)]
fn platform_list() -> Vec<ShadowCopy> {
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
fn platform_list() -> Vec<ShadowCopy> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{list, shadow_copy_index, shadow_copy_path};

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
        let _ = list();
    }
}
