//! What an operating system's device paths mean.
//!
//! Every platform names its disks differently, and each naming scheme carries
//! two facts Argos needs before it opens anything: whether a path addresses a
//! **whole disk** or a single partition, and which disk a partition belongs
//! to. Scanning a partition when the user meant the disk silently skips
//! everything outside it — including the residue of previous filesystems,
//! which is the evidence a re-formatted disk still holds.
//!
//! This module is the one place those conventions are written down, and it is
//! compiled on every target rather than behind `cfg`. That is deliberate:
//! these are decisions, not syscalls, and a decision that only compiles on
//! Windows is a decision only Windows CI can test. The platform modules call
//! in here and keep the `unsafe` to themselves (`M-MOCKABLE-SYSCALLS`).

/// What kind of node a device path addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// The whole disk, including its partition table and everything between
    /// and after its partitions. The only target a full recovery scan should
    /// use.
    WholeDisk,
    /// One partition of a disk. Scanning this cannot see anything outside it.
    Partition,
}

/// Prefix of a Windows physical-drive path.
///
/// `\\.\PhysicalDriveN` is the Win32 device namespace name for the whole
/// disk; partitions are `\\.\HarddiskVolumeN` or a drive letter.
pub const WINDOWS_PHYSICAL_DRIVE: &str = r"\\.\PhysicalDrive";

/// The Windows path addressing physical drive `index` as a whole.
#[must_use]
pub fn windows_physical_drive(index: u32) -> String {
    format!("{WINDOWS_PHYSICAL_DRIVE}{index}")
}

/// What a Windows device path addresses, when it is one Argos understands.
///
/// `None` for anything outside the device namespace — a file path, a UNC
/// share, or a name this module has no convention for. Argos never guesses at
/// an unrecognised path.
#[must_use]
pub fn windows_node_kind(path: &str) -> Option<NodeKind> {
    // Windows accepts `\\.\` and `\\?\` interchangeably for these names, and
    // is case-insensitive about them.
    let rest = path
        .strip_prefix(r"\\.\")
        .or_else(|| path.strip_prefix(r"\\?\"))?;
    let lower = rest.to_ascii_lowercase();

    if let Some(index) = lower.strip_prefix("physicaldrive") {
        return index.parse::<u32>().is_ok().then_some(NodeKind::WholeDisk);
    }
    if let Some(index) = lower.strip_prefix("harddiskvolume") {
        return index.parse::<u32>().is_ok().then_some(NodeKind::Partition);
    }
    // A bare drive letter: `\\.\C:`.
    let bytes = lower.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(NodeKind::Partition);
    }
    None
}

/// What a Linux device path addresses.
///
/// `None` for a path outside `/dev`, or a name with no convention here.
///
/// The conventions: `sdX`/`vdX`/`hdX` are whole disks and `sdX<N>` partitions;
/// `nvmeXnY` is a whole disk and `nvmeXnYpZ` a partition; `mmcblkX` a whole
/// disk and `mmcblkXpY` a partition; loop and device-mapper nodes are whole
/// devices as far as a scan is concerned.
#[must_use]
pub fn linux_node_kind(path: &str) -> Option<NodeKind> {
    let name = path.strip_prefix("/dev/")?;
    if name.is_empty() || name.contains('/') {
        // Nested nodes like `/dev/mapper/x` are whole devices; anything else
        // with a slash is not a disk node this module knows.
        return name
            .strip_prefix("mapper/")
            .filter(|rest| !rest.is_empty())
            .map(|_| NodeKind::WholeDisk);
    }

    for family in ["nvme", "mmcblk"] {
        if let Some(rest) = name.strip_prefix(family) {
            // `nvme0n1` / `mmcblk0`: a partition adds a `p<N>` suffix.
            return Some(match rest.rsplit_once('p') {
                Some((head, tail)) if !head.is_empty() && tail.parse::<u32>().is_ok() => {
                    NodeKind::Partition
                }
                _ => NodeKind::WholeDisk,
            });
        }
    }
    for family in ["sd", "vd", "hd", "loop"] {
        if let Some(rest) = name.strip_prefix(family) {
            if rest.is_empty() {
                return None;
            }
            // `sda` is whole, `sda1` a partition; `loop0` is whole, `loop0p1`
            // a partition.
            let trailing_digits = rest.chars().rev().take_while(char::is_ascii_digit).count();
            let has_letters = rest.chars().any(|c| c.is_ascii_alphabetic());
            return Some(if family == "loop" {
                if rest.contains('p') {
                    NodeKind::Partition
                } else {
                    NodeKind::WholeDisk
                }
            } else if has_letters && trailing_digits > 0 {
                NodeKind::Partition
            } else if has_letters {
                NodeKind::WholeDisk
            } else {
                return None;
            });
        }
    }
    None
}

/// What a macOS device path addresses.
///
/// `diskN` is a whole disk; `diskNsM` is a partition (and `diskNsMsK` a
/// sub-slice, still a partition as far as a scan is concerned). The `r`
/// prefix — `rdiskN` — selects the same medium through the raw, unbuffered
/// character device.
#[must_use]
pub fn macos_node_kind(path: &str) -> Option<NodeKind> {
    let name = path.strip_prefix("/dev/")?;
    let name = name.strip_prefix('r').unwrap_or(name);
    let rest = name.strip_prefix("disk")?;
    let (index, slices) = rest.split_once('s').map_or((rest, false), |(head, tail)| {
        (
            head,
            !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == 's'),
        )
    });
    if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(if slices {
        NodeKind::Partition
    } else {
        NodeKind::WholeDisk
    })
}

/// The raw character-device path for a macOS disk path.
///
/// macOS exposes each medium twice: `/dev/diskN` goes through the buffer
/// cache, `/dev/rdiskN` does not. A forensic read wants the raw node — the
/// cache would both slow a full-surface sweep down and hand back pages some
/// other process happened to have read. Returns the input unchanged when it
/// is already raw or is not a disk path.
#[must_use]
pub fn macos_raw_path(path: &str) -> String {
    match path.strip_prefix("/dev/") {
        Some(name) if name.starts_with("disk") => format!("/dev/r{name}"),
        _ => path.to_owned(),
    }
}

/// The whole-disk path a macOS partition path belongs to.
///
/// `None` when the path is already a whole disk or is not a disk path.
#[must_use]
pub fn macos_whole_disk(path: &str) -> Option<String> {
    if macos_node_kind(path)? != NodeKind::Partition {
        return None;
    }
    let name = path.strip_prefix("/dev/")?;
    let raw = name.starts_with('r');
    let bare = name.strip_prefix('r').unwrap_or(name);
    let index = bare.strip_prefix("disk")?.split('s').next()?;
    Some(if raw {
        format!("/dev/rdisk{index}")
    } else {
        format!("/dev/disk{index}")
    })
}

/// One spelling of a device path, so two spellings of the same medium
/// compare equal.
///
/// Each platform accepts more than one name for one device: macOS has the
/// buffered `/dev/diskN` and the raw `/dev/rdiskN`, and Windows takes both
/// `\\.\` and `\\?\` and is case-insensitive about what follows. Comparing
/// the strings as given therefore misses matches that matter — the mount
/// warning is looked up by path, and a scan of `/dev/rdisk0` would silently
/// find no entry for `/dev/disk0` and warn about nothing, on exactly the raw
/// node the macOS HAL tells callers to prefer.
///
/// Paths with no convention here come back unchanged.
#[must_use]
pub fn canonical(path: &str) -> String {
    // Windows: one prefix, one case.
    if let Some(rest) = path
        .strip_prefix(r"\\.\")
        .or_else(|| path.strip_prefix(r"\\?\"))
        && windows_node_kind(path).is_some()
    {
        return format!(r"\\.\{}", rest.to_ascii_lowercase());
    }
    // macOS: the buffered name, which is what enumeration lists.
    if macos_node_kind(path).is_some()
        && let Some(name) = path.strip_prefix("/dev/")
        && let Some(bare) = name.strip_prefix('r')
    {
        return format!("/dev/{bare}");
    }
    path.to_owned()
}

/// The whole-disk path a Linux partition path belongs to.
///
/// `None` when the path is already a whole disk or is not a disk path.
#[must_use]
pub fn linux_whole_disk(path: &str) -> Option<String> {
    if linux_node_kind(path)? != NodeKind::Partition {
        return None;
    }
    let name = path.strip_prefix("/dev/")?;
    // `nvme0n1p3` and `mmcblk0p1` drop everything from the final `p`; `sda1`
    // drops its trailing digits.
    for family in ["nvme", "mmcblk"] {
        if name.starts_with(family) {
            let (head, _) = name.rsplit_once('p')?;
            return Some(format!("/dev/{head}"));
        }
    }
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    (!trimmed.is_empty() && trimmed != name).then(|| format!("/dev/{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::{
        NodeKind, canonical, linux_node_kind, linux_whole_disk, macos_node_kind, macos_raw_path,
        macos_whole_disk, windows_node_kind, windows_physical_drive,
    };

    #[test]
    fn windows_physical_drives_are_whole_disks_and_volumes_are_not() {
        assert_eq!(
            windows_node_kind(r"\\.\PhysicalDrive0"),
            Some(NodeKind::WholeDisk)
        );
        assert_eq!(
            windows_node_kind(r"\\?\physicaldrive12"),
            Some(NodeKind::WholeDisk)
        );
        assert_eq!(
            windows_node_kind(r"\\.\HarddiskVolume3"),
            Some(NodeKind::Partition)
        );
        assert_eq!(windows_node_kind(r"\\.\C:"), Some(NodeKind::Partition));
        // Anything without a convention here is refused rather than guessed.
        assert_eq!(windows_node_kind(r"C:\images\disk.img"), None);
        assert_eq!(windows_node_kind(r"\\.\PhysicalDriveX"), None);
        assert_eq!(windows_node_kind(r"\\server\share"), None);
    }

    #[test]
    fn windows_drive_paths_round_trip() {
        for index in [0_u32, 1, 9, 10, 255] {
            let path = windows_physical_drive(index);
            assert_eq!(windows_node_kind(&path), Some(NodeKind::WholeDisk));
        }
    }

    #[test]
    fn linux_partitions_are_told_from_their_disks() {
        for (path, kind) in [
            ("/dev/sda", NodeKind::WholeDisk),
            ("/dev/sdb", NodeKind::WholeDisk),
            ("/dev/sda1", NodeKind::Partition),
            ("/dev/sdaa", NodeKind::WholeDisk),
            ("/dev/sdaa12", NodeKind::Partition),
            ("/dev/vda", NodeKind::WholeDisk),
            ("/dev/vda2", NodeKind::Partition),
            ("/dev/nvme0n1", NodeKind::WholeDisk),
            ("/dev/nvme0n1p3", NodeKind::Partition),
            ("/dev/mmcblk0", NodeKind::WholeDisk),
            ("/dev/mmcblk0p1", NodeKind::Partition),
            ("/dev/loop0", NodeKind::WholeDisk),
            ("/dev/mapper/vg-root", NodeKind::WholeDisk),
        ] {
            assert_eq!(linux_node_kind(path), Some(kind), "{path}");
        }
        assert_eq!(linux_node_kind("/tmp/disk.img"), None);
        assert_eq!(linux_node_kind("/dev/null"), None);
        assert_eq!(linux_node_kind("/dev/sd"), None);
    }

    #[test]
    fn a_linux_partition_names_the_disk_it_belongs_to() {
        assert_eq!(linux_whole_disk("/dev/sda1").as_deref(), Some("/dev/sda"));
        assert_eq!(
            linux_whole_disk("/dev/sdaa12").as_deref(),
            Some("/dev/sdaa")
        );
        assert_eq!(
            linux_whole_disk("/dev/nvme0n1p3").as_deref(),
            Some("/dev/nvme0n1")
        );
        assert_eq!(
            linux_whole_disk("/dev/mmcblk0p1").as_deref(),
            Some("/dev/mmcblk0")
        );
        // A whole disk has no disk above it.
        assert_eq!(linux_whole_disk("/dev/sda"), None);
        assert_eq!(linux_whole_disk("/dev/nvme0n1"), None);
    }

    #[test]
    fn macos_slices_are_partitions_and_bare_disks_are_not() {
        for (path, kind) in [
            ("/dev/disk0", NodeKind::WholeDisk),
            ("/dev/rdisk0", NodeKind::WholeDisk),
            ("/dev/disk2", NodeKind::WholeDisk),
            ("/dev/disk0s1", NodeKind::Partition),
            ("/dev/rdisk0s1", NodeKind::Partition),
            ("/dev/disk1s2s1", NodeKind::Partition),
        ] {
            assert_eq!(macos_node_kind(path), Some(kind), "{path}");
        }
        assert_eq!(macos_node_kind("/dev/null"), None);
        assert_eq!(macos_node_kind("/Users/me/disk.img"), None);
        assert_eq!(macos_node_kind("/dev/disk"), None);
    }

    #[test]
    fn the_raw_node_is_preferred_and_asking_twice_changes_nothing() {
        assert_eq!(macos_raw_path("/dev/disk0"), "/dev/rdisk0");
        assert_eq!(macos_raw_path("/dev/disk0s1"), "/dev/rdisk0s1");
        // Already raw: unchanged, so a caller cannot build `/dev/rrdisk0`.
        assert_eq!(macos_raw_path("/dev/rdisk0"), "/dev/rdisk0");
        // Not a disk path: untouched.
        assert_eq!(macos_raw_path("/tmp/disk.img"), "/tmp/disk.img");
    }

    #[test]
    fn two_spellings_of_one_medium_compare_equal() {
        // The mount warning is looked up by path, so a spelling that does not
        // match the enumerated one warns about nothing.
        assert_eq!(canonical("/dev/rdisk0"), canonical("/dev/disk0"));
        assert_eq!(canonical("/dev/rdisk2s1"), canonical("/dev/disk2s1"));
        assert_eq!(
            canonical(r"\\?\PhysicalDrive0"),
            canonical(r"\\.\PhysicalDrive0")
        );
        assert_eq!(
            canonical(r"\\.\physicaldrive3"),
            canonical(r"\\.\PhysicalDrive3")
        );
    }

    #[test]
    fn canonicalising_leaves_different_media_apart() {
        assert_ne!(canonical("/dev/disk0"), canonical("/dev/disk1"));
        assert_ne!(canonical("/dev/disk0"), canonical("/dev/disk0s1"));
        assert_ne!(
            canonical(r"\\.\PhysicalDrive0"),
            canonical(r"\\.\PhysicalDrive1")
        );
        // Linux has one spelling, and a path with no convention is untouched.
        assert_eq!(canonical("/dev/sda1"), "/dev/sda1");
        assert_eq!(canonical("/tmp/disk.img"), "/tmp/disk.img");
        // `/dev/random` starts with `r` but is not a disk node: it must not be
        // rewritten to `/dev/andom`.
        assert_eq!(canonical("/dev/random"), "/dev/random");
    }

    #[test]
    fn a_macos_slice_names_the_disk_it_belongs_to() {
        assert_eq!(
            macos_whole_disk("/dev/disk0s1").as_deref(),
            Some("/dev/disk0")
        );
        assert_eq!(
            macos_whole_disk("/dev/rdisk3s2s1").as_deref(),
            Some("/dev/rdisk3")
        );
        assert_eq!(macos_whole_disk("/dev/disk0"), None);
    }
}
