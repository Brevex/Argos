//! Reading the operating system's mount table.
//!
//! Every platform publishes the same facts in a different shape, and the
//! parsing is what decides whether a scan warns the examiner that the medium
//! is changing underneath it. So the parsing lives here, compiled on every
//! target and tested on every target, and the platform modules do nothing but
//! hand it the text or the records their OS produced
//! (`M-MOCKABLE-SYSCALLS`).

use std::path::{Path, PathBuf};

use crate::inventory::MountPoint;

/// Parses Linux' `/proc/self/mountinfo`.
///
/// The format, per `Documentation/filesystems/proc.rst`, is a fixed head of
/// six fields, then optional tags, then a `-` separator, then the filesystem
/// type, the mount source and the superblock options:
///
/// ```text
/// 36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue
/// ```
///
/// `mountinfo` is used rather than `/proc/mounts` because it survives paths
/// containing spaces — which it escapes as `\040` — and because its separator
/// makes the variable-length tag list unambiguous.
#[must_use]
pub fn parse_linux_mountinfo(text: &str) -> Vec<MountPoint> {
    let mut mounts = Vec::new();
    for line in text.lines() {
        let Some((head, tail)) = line.split_once(" - ") else {
            continue;
        };
        let mut head_fields = head.split(' ');
        // Fields 1..=5 are ignored; field 6 is the per-mount option list,
        // which is where `ro` lives.
        let options = head_fields.nth(5);
        let mut tail_fields = tail.split(' ');
        // Filesystem type, then the source.
        let Some(source) = tail_fields.nth(1) else {
            continue;
        };
        let super_options = tail_fields.next();

        // A source that is not a path is a pseudo-filesystem — `proc`,
        // `tmpfs`, `cgroup` — and belongs to no medium.
        if !source.starts_with('/') {
            continue;
        }
        let Some(target) = head.split(' ').nth(4) else {
            continue;
        };
        mounts.push(MountPoint {
            source: PathBuf::from(unescape_octal(source)),
            target: PathBuf::from(unescape_octal(target)),
            read_only: has_ro_option(options) || has_ro_option(super_options),
        });
    }
    mounts
}

/// Whether a comma-separated mount-option list contains `ro`.
///
/// Matched as a whole field, so `rootcontext=…` and `errors=remount-ro` do not
/// count as read-only mounts.
fn has_ro_option(options: Option<&str>) -> bool {
    options.is_some_and(|list| list.split(',').any(|option| option == "ro"))
}

/// Expands the `\040`-style octal escapes Linux writes for the characters it
/// cannot put in a whitespace-separated field.
fn unescape_octal(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_owned();
    }
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        // Exactly three octal digits, or the backslash was literal.
        let digits: String = chars.clone().take(3).collect();
        match u8::from_str_radix(&digits, 8) {
            Ok(byte) if digits.len() == 3 => {
                out.push(char::from(byte));
                for _ in 0..3 {
                    let _ = chars.next();
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// Which of `mounts` belong to the medium at `device`.
///
/// A mount names a partition; a scan targets the disk. This is the join
/// between them: a mount belongs to the device when it *is* the device, or
/// when the device is the whole disk the mount's partition sits on.
///
/// `whole_disk_of` maps a partition path to its disk — the per-OS convention
/// from [`naming`](crate::naming) — so this function stays platform-neutral.
#[must_use]
pub fn mounts_of(
    device: &Path,
    mounts: &[MountPoint],
    whole_disk_of: impl Fn(&str) -> Option<String>,
) -> Vec<MountPoint> {
    let device_text = device.to_string_lossy();
    mounts
        .iter()
        .filter(|mount| {
            let source = mount.source.to_string_lossy();
            source == device_text || whole_disk_of(&source).is_some_and(|disk| disk == device_text)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{mounts_of, parse_linux_mountinfo};
    use crate::inventory::MountPoint;
    use crate::naming;

    /// A `mountinfo` excerpt of the shape a running Linux system produces.
    const MOUNTINFO: &str = "\
21 27 0:20 / /proc rw,nosuid,relatime shared:5 - proc proc rw
24 27 0:6 / /dev rw,nosuid shared:2 - devtmpfs devtmpfs rw,size=4096k
30 27 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw,errors=remount-ro
31 30 8:1 / /boot ro,relatime shared:9 - vfat /dev/sda1 ro,fmask=0022
32 30 259:3 / /home rw,relatime shared:12 - ext4 /dev/nvme0n1p3 rw
33 30 8:17 / /media/My\\040Disk rw,relatime shared:14 - ext4 /dev/sdb1 rw
";

    #[test]
    fn pseudo_filesystems_are_not_mounts_of_any_medium() {
        let mounts = parse_linux_mountinfo(MOUNTINFO);
        assert!(
            mounts.iter().all(|mount| mount.source.starts_with("/dev/")),
            "proc, devtmpfs and friends belong to no medium: {mounts:?}"
        );
        assert_eq!(mounts.len(), 4);
    }

    #[test]
    fn a_read_only_mount_is_recognised_from_either_option_list() {
        let mounts = parse_linux_mountinfo(MOUNTINFO);
        let boot = mounts
            .iter()
            .find(|mount| mount.target == Path::new("/boot"))
            .expect("/boot is in the fixture");
        assert!(boot.read_only);

        // `errors=remount-ro` is not a read-only mount, and matching the
        // option as a substring would say it was.
        let root = mounts
            .iter()
            .find(|mount| mount.target == Path::new("/"))
            .expect("/ is in the fixture");
        assert!(
            !root.read_only,
            "errors=remount-ro is not a read-only mount"
        );
    }

    #[test]
    fn a_mount_path_with_a_space_survives_parsing() {
        let mounts = parse_linux_mountinfo(MOUNTINFO);
        let media = mounts
            .iter()
            .find(|mount| mount.source == Path::new("/dev/sdb1"))
            .expect("/dev/sdb1 is in the fixture");
        assert_eq!(media.target, PathBuf::from("/media/My Disk"));
    }

    #[test]
    fn a_disk_inherits_the_mounts_of_its_partitions() {
        let mounts = parse_linux_mountinfo(MOUNTINFO);

        // The whole disk: both of its partitions count, even though neither
        // mount names the disk itself.
        let sda = mounts_of(Path::new("/dev/sda"), &mounts, |path| {
            naming::linux_whole_disk(path)
        });
        assert_eq!(sda.len(), 2, "sda1 and sda2 both sit on /dev/sda");

        // An NVMe namespace uses a different partition convention, and the
        // join must follow it too.
        let nvme = mounts_of(Path::new("/dev/nvme0n1"), &mounts, |path| {
            naming::linux_whole_disk(path)
        });
        assert_eq!(nvme.len(), 1);

        // A disk with nothing mounted on it comes back empty rather than
        // inheriting some other disk's mounts.
        let idle = mounts_of(Path::new("/dev/sdz"), &mounts, |path| {
            naming::linux_whole_disk(path)
        });
        assert!(idle.is_empty());
    }

    #[test]
    fn a_partition_matches_only_its_own_mount() {
        let mounts = parse_linux_mountinfo(MOUNTINFO);
        let sda1 = mounts_of(Path::new("/dev/sda1"), &mounts, |path| {
            naming::linux_whole_disk(path)
        });
        assert_eq!(sda1.len(), 1);
        assert_eq!(sda1[0].target, PathBuf::from("/boot"));
    }

    #[test]
    fn a_macos_slice_joins_to_its_disk() {
        // The same join, with macOS' convention supplied instead.
        let mounts = vec![
            MountPoint {
                source: PathBuf::from("/dev/disk0s2"),
                target: PathBuf::from("/"),
                read_only: false,
            },
            MountPoint {
                source: PathBuf::from("/dev/disk1s1"),
                target: PathBuf::from("/Volumes/Backup"),
                read_only: true,
            },
        ];
        let disk0 = mounts_of(Path::new("/dev/disk0"), &mounts, |path| {
            naming::macos_whole_disk(path)
        });
        assert_eq!(disk0.len(), 1);
        assert_eq!(disk0[0].target, PathBuf::from("/"));
    }

    #[test]
    fn a_mountinfo_line_without_a_separator_is_skipped_not_guessed_at() {
        // Truncated or unexpected input yields nothing rather than a
        // half-parsed mount that would misreport which disk is busy.
        assert!(parse_linux_mountinfo("garbage").is_empty());
        assert!(parse_linux_mountinfo("").is_empty());
        assert!(parse_linux_mountinfo("21 27 0:20 / /proc rw - proc").is_empty());
    }
}
