//! Who this process is recovering files for.
//!
//! Reading a raw device needs administrator privileges, so a scan of one runs
//! as root and every file it writes is created by root. The person who chose
//! the destination folder would then be unable to open, move or delete what
//! was recovered for them — a recovery that succeeded and is still useless.
//!
//! Every way of becoming root leaves a trace of who asked, and this reads
//! whichever is there. Absent all of them, the process was root to begin with
//! and there is nobody to hand anything back to.

use argos_report::Owner;

/// Set by the Argos shell when it starts an engine on someone's behalf.
const SHELL_UID: &str = "ARGOS_INVOKER_UID";
/// Group counterpart of [`SHELL_UID`].
const SHELL_GID: &str = "ARGOS_INVOKER_GID";

/// The account to give recovered files to, if this process is acting for one.
#[must_use]
pub fn owner() -> Option<Owner> {
    // In order of how much they know. The shell passes both parts; `sudo`
    // publishes both; `pkexec` publishes only the user, and leaving the group
    // alone is better than inventing one.
    for (uid, gid) in [(SHELL_UID, Some(SHELL_GID)), ("SUDO_UID", Some("SUDO_GID"))] {
        if let Some(uid) = read(uid) {
            return Some(Owner::new(uid, gid.and_then(read)));
        }
    }
    read("PKEXEC_UID").map(|uid| Owner::new(uid, None))
}

/// One environment variable as a user or group id.
fn read(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}
