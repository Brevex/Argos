//! What each platform's answer about a medium means for recovery.
//!
//! Every OS reports the same underlying fact — is this a spinning platter or
//! solid state — through a different query, and each query can decline to
//! answer. What matters here is what a *missing* answer means: it means
//! [`DeviceClass::Unknown`], never a guess. Reporting `Ssd` on a hunch would
//! tell an examiner their data is probably gone when it may not be; reporting
//! `Hdd` on a hunch does the reverse.
//!
//! Like [`naming`](crate::naming), this compiles on every target so the
//! decisions are tested everywhere rather than only on the OS that produced
//! them.

use argos_core::ports::DeviceClass;

/// Class of a medium from Linux's sysfs `queue/rotational` flag.
///
/// `1` is a rotational disk, `0` is not; anything else — including sysfs not
/// answering at all — is unknown.
#[must_use]
pub fn from_rotational(flag: Option<&str>) -> DeviceClass {
    match flag.map(str::trim) {
        Some("1") => DeviceClass::Hdd,
        Some("0") => DeviceClass::Ssd,
        _ => DeviceClass::Unknown,
    }
}

/// Class of a medium from Windows' `IncursSeekPenalty`.
///
/// A seek penalty is what a rotational disk has and solid state does not, so
/// this is the same question `rotational` asks. `None` is the storage driver
/// declining to answer, which several USB bridges and virtual disks do.
#[must_use]
pub fn from_seek_penalty(incurs_penalty: Option<bool>) -> DeviceClass {
    match incurs_penalty {
        Some(true) => DeviceClass::Hdd,
        Some(false) => DeviceClass::Ssd,
        None => DeviceClass::Unknown,
    }
}

/// Class of a medium from macOS' `DKIOCISSOLIDSTATE`.
///
/// `None` is the driver declining, which is the normal answer for disk images
/// and many external enclosures.
#[must_use]
pub fn from_solid_state(is_solid_state: Option<bool>) -> DeviceClass {
    match is_solid_state {
        Some(true) => DeviceClass::Ssd,
        Some(false) => DeviceClass::Hdd,
        None => DeviceClass::Unknown,
    }
}

/// Whether a medium of this class and TRIM state is still expected to hold
/// deleted content on its host-visible surface.
///
/// This is finer than the class alone, and the difference matters to an
/// examiner. After TRIM, a solid-state controller returns zeros for the
/// blocks a delete released, and the data is gone before Argos ever sees the
/// medium — but an SSD with TRIM *disabled*, which is the usual state for
/// external USB enclosures and for volumes an OS never mounted, keeps its
/// deleted content exactly as a spinning disk would. Reporting "probably
/// erased" for those would talk an examiner out of a recovery that was going
/// to work.
#[must_use]
pub fn expects_deleted_content(class: DeviceClass, trim: TrimState) -> bool {
    match class {
        // Not solid state: nothing erases behind the deletion, whatever the
        // discard flag says. TRIM is a controller behaviour, and a platter
        // has no controller that rewrites it — a rotational device reporting
        // discard support is a virtual or mapped device, not an erasing one.
        DeviceClass::Hdd | DeviceClass::ImageFile => true,
        // Solid state with TRIM confirmed off keeps its deleted content;
        // solid state that would not say is assumed to have TRIM, because
        // TRIM is the default nearly everywhere and silence is not evidence
        // it is off.
        DeviceClass::Ssd => trim == TrimState::Disabled,
        // No class, or a class this build has never heard of — `DeviceClass`
        // is `#[non_exhaustive]`. Discard support is then the only evidence
        // there is, and a medium that accepts discard is one whose controller
        // can erase behind a delete.
        DeviceClass::Unknown | _ => trim != TrimState::Enabled,
    }
}

/// Whether the medium reports the TRIM/UNMAP command as enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrimState {
    /// The medium reports TRIM enabled: deleted blocks are likely already
    /// zeroed by the controller.
    Enabled,
    /// The medium reports TRIM unavailable or disabled.
    Disabled,
    /// The medium did not say, or the platform has no way to ask.
    #[default]
    Unknown,
}

impl TrimState {
    /// TRIM state from a reported flag; `None` is a driver that declined.
    #[must_use]
    pub fn from_flag(enabled: Option<bool>) -> Self {
        match enabled {
            Some(true) => Self::Enabled,
            Some(false) => Self::Disabled,
            None => Self::Unknown,
        }
    }
}

impl std::fmt::Display for TrimState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Unknown => "unknown",
        })
    }
}

#[cfg(test)]
mod tests {
    use argos_core::ports::DeviceClass;

    use super::{
        TrimState, expects_deleted_content, from_rotational, from_seek_penalty, from_solid_state,
    };

    #[test]
    fn every_platform_reads_the_same_fact_the_same_way() {
        // The three queries are the same question in three dialects, so they
        // must agree — a disk that is rotational on Linux is one that incurs
        // a seek penalty on Windows and is not solid state on macOS.
        assert_eq!(from_rotational(Some("1")), DeviceClass::Hdd);
        assert_eq!(from_seek_penalty(Some(true)), DeviceClass::Hdd);
        assert_eq!(from_solid_state(Some(false)), DeviceClass::Hdd);

        assert_eq!(from_rotational(Some("0")), DeviceClass::Ssd);
        assert_eq!(from_seek_penalty(Some(false)), DeviceClass::Ssd);
        assert_eq!(from_solid_state(Some(true)), DeviceClass::Ssd);
    }

    #[test]
    fn a_driver_that_declines_leaves_the_class_unknown() {
        // The failure that matters: never invent a class. Several USB bridges
        // and every virtual disk answer nothing at all.
        assert_eq!(from_rotational(None), DeviceClass::Unknown);
        assert_eq!(from_rotational(Some("")), DeviceClass::Unknown);
        assert_eq!(from_rotational(Some("maybe")), DeviceClass::Unknown);
        assert_eq!(from_seek_penalty(None), DeviceClass::Unknown);
        assert_eq!(from_solid_state(None), DeviceClass::Unknown);
    }

    #[test]
    fn sysfs_whitespace_does_not_change_the_answer() {
        // sysfs files end in a newline.
        assert_eq!(from_rotational(Some("1\n")), DeviceClass::Hdd);
        assert_eq!(from_rotational(Some(" 0 \n")), DeviceClass::Ssd);
    }

    #[test]
    fn an_ssd_with_trim_off_still_expects_its_deleted_content() {
        // The case a class-only warning gets wrong: external enclosures and
        // never-mounted volumes routinely have TRIM disabled, and their
        // deleted content survives exactly as a platter's would.
        assert!(expects_deleted_content(
            DeviceClass::Ssd,
            TrimState::Disabled
        ));
        assert!(!expects_deleted_content(
            DeviceClass::Ssd,
            TrimState::Enabled
        ));
        // Unknown TRIM on an SSD is the pessimistic case: TRIM is the default
        // nearly everywhere, so silence is not evidence it is off.
        assert!(!expects_deleted_content(
            DeviceClass::Ssd,
            TrimState::Unknown
        ));
    }

    #[test]
    fn a_rotational_disk_always_expects_its_deleted_content() {
        for trim in [TrimState::Enabled, TrimState::Disabled, TrimState::Unknown] {
            assert!(expects_deleted_content(DeviceClass::Hdd, trim), "{trim:?}");
            assert!(
                expects_deleted_content(DeviceClass::ImageFile, trim),
                "{trim:?}"
            );
        }
    }

    #[test]
    fn an_unknown_class_only_warns_on_evidence() {
        // Nothing known either way: proceed without discouraging the user.
        assert!(expects_deleted_content(
            DeviceClass::Unknown,
            TrimState::Unknown
        ));
        assert!(expects_deleted_content(
            DeviceClass::Unknown,
            TrimState::Disabled
        ));
        // TRIM confirmed on is evidence, whatever the class turned out to be.
        assert!(!expects_deleted_content(
            DeviceClass::Unknown,
            TrimState::Enabled
        ));
    }
}
