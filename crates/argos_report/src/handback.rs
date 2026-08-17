//! Giving the recovered files to the person who asked for them.
//!
//! A scan of a raw device runs with administrator privileges, so every file it
//! writes is created by that account. Without this the person who chose the
//! destination folder could not open, move or delete what was recovered for
//! them — the recovery would have succeeded and still be useless.
//!
//! This is not always possible, and where it is not, it is said rather than
//! attempted twice: a destination on exFAT or on a mounted Windows volume has
//! no Unix ownership to change, and the files are perfectly usable there for a
//! different reason.

use std::path::Path;

/// The account that recovered files belong to once written.
///
/// Constructed from the identity a privileged process was started on behalf
/// of, which the process running the scan resolves; this crate is given the
/// answer rather than looking for it, so what it does is visible in its
/// signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    uid: u32,
    gid: Option<u32>,
}

impl Owner {
    /// The account with this user id, and optionally this group.
    ///
    /// The group is optional because not every elevation path reports one:
    /// `pkexec` publishes the caller's user id alone, and leaving the group
    /// untouched is better than guessing at one.
    #[must_use]
    pub fn new(uid: u32, gid: Option<u32>) -> Self {
        Self { uid, gid }
    }

    /// Gives `path` to this owner.
    ///
    /// # Errors
    ///
    /// Fails when the filesystem cannot represent the change — no ownership at
    /// all on FAT and exFAT — or when this process is not allowed to make it.
    #[cfg(unix)]
    pub fn give(self, path: &Path) -> Result<(), std::io::Error> {
        std::os::unix::fs::chown(path, Some(self.uid), self.gid)
    }

    /// Gives `path` to this owner.
    ///
    /// Windows has nothing to do here: a file created by an elevated process
    /// inherits the destination folder's access rules, so the person who chose
    /// the folder keeps the access they already had to it.
    ///
    /// # Errors
    ///
    /// Never on this platform. The signature is the Unix one so that the caller
    /// stays free of `cfg`.
    #[cfg(not(unix))]
    pub fn give(self, path: &Path) -> Result<(), std::io::Error> {
        let _ = path;
        Ok(())
    }
}

/// Whether an output directory could be handed to its [`Owner`].
///
/// Reported rather than hidden: files left belonging to the administrator are
/// something the person reading the result has to know, and it is the kind of
/// thing that is discovered hours later, at the end of a long scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handback {
    /// Nothing to do: no owner was named, or the platform has no notion of one.
    NotNeeded,
    /// The output directory now belongs to the owner, and its files will too.
    Done,
    /// The output directory could not be handed over, and neither will its
    /// files be. Carries what to tell the user.
    Refused(String),
}

impl Handback {
    /// Attempts to give `dir` to `owner`, and says what happened.
    pub(crate) fn attempt(dir: &Path, owner: Option<Owner>) -> Self {
        let Some(owner) = owner else {
            return Self::NotNeeded;
        };
        match owner.give(dir) {
            Ok(()) => Self::Done,
            // The message names no recovered content and no filename, only the
            // directory the user themselves chose (A-NO-CONTENT-IN-LOGS).
            Err(err) => Self::Refused(format!(
                "recovered files will belong to the administrator account rather than to you, \
                 because the destination does not allow changing ownership ({err}); copy them \
                 elsewhere or take ownership of {} afterwards",
                dir.display()
            )),
        }
    }

    /// The owner to apply to each file written, if any.
    pub(crate) fn owner(&self, owner: Option<Owner>) -> Option<Owner> {
        matches!(self, Self::Done).then_some(owner).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::Handback;
    #[cfg(unix)]
    use super::Owner;

    #[test]
    fn no_owner_means_nothing_to_do() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert_eq!(Handback::attempt(dir.path(), None), Handback::NotNeeded);
    }

    #[cfg(unix)]
    #[test]
    fn handing_a_directory_to_its_current_owner_succeeds() {
        // Every account may give a file it owns to itself, so this exercises
        // the real syscall without needing privileges.
        let dir = tempfile::tempdir().expect("temporary directory");
        let owner = current();
        assert_eq!(Handback::attempt(dir.path(), Some(owner)), Handback::Done);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_handback_stops_the_per_file_attempts() {
        // Once the directory could not be handed over, its files are on the
        // same filesystem and will not be either; retrying per artifact would
        // fail once per recovered image.
        let refused = Handback::Refused("no".to_owned());
        assert_eq!(refused.owner(Some(current())), None);
        assert_eq!(Handback::Done.owner(Some(current())), Some(current()));
        assert_eq!(Handback::Done.owner(None), None);
    }

    #[cfg(unix)]
    fn current() -> Owner {
        use std::os::unix::fs::MetadataExt;

        let me = std::fs::metadata("/proc/self")
            .or_else(|_| std::fs::metadata("."))
            .expect("this process can stat something it owns");
        Owner::new(me.uid(), Some(me.gid()))
    }
}
