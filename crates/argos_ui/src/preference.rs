//! Where a view preference is kept.
//!
//! The window runs as an administrator, so everything a web view stores —
//! `localStorage` included — lands in the administrator's profile. A theme
//! chosen there belongs to the machine rather than to the person who chose it:
//! every user of the computer shares it, and none of them can find it to
//! change it back.
//!
//! So the preference is a file in the home directory of whoever opened Argos,
//! given to that account like everything else the run produces. It holds one
//! thing, and nothing about it is evidence: losing it costs a colour scheme.

use std::path::{Path, PathBuf};

/// Directory under the invoker's home holding this file.
const DIR: &str = ".config/argos";

/// The file itself.
const FILE: &str = "ui.json";

/// Where the preference lives, when there is somewhere to put it.
///
/// `None` on a platform or a start with no invoker to attribute it to, and the
/// window then keeps its preference for as long as it is open and no longer.
fn path() -> Option<PathBuf> {
    let home = crate::elevate::invoker_home()?;
    Some(PathBuf::from(home).join(DIR).join(FILE))
}

/// The stored preferences, as the JSON text the window last wrote.
///
/// An empty string when there is nothing stored, which the window reads as
/// "use the defaults". A file that cannot be read is the same answer: a
/// preference is not worth a failure.
#[must_use]
pub fn read() -> String {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Replaces the stored preferences with `text`.
///
/// # Errors
///
/// Fails when the directory or the file cannot be written.
pub fn write(text: &str) -> Result<(), String> {
    let Some(path) = path() else {
        return Ok(());
    };
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    std::fs::write(&path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    // Written by an administrator into someone else's home: without this the
    // person whose preference it is could not change it again.
    give_to_invoker(dir);
    give_to_invoker(&path);
    Ok(())
}

/// Gives `path` to the account the window is acting for, if there is one.
///
/// Failures are dropped: a preference whose ownership could not be handed over
/// is still a preference, and a colour scheme is worth no error path.
#[cfg(unix)]
fn give_to_invoker(path: &Path) {
    let read = |name: &str| std::env::var(name).ok()?.parse::<u32>().ok();
    if let Some(uid) = read(crate::elevate::session::INVOKER_UID) {
        let group = read(crate::elevate::session::INVOKER_GID);
        let _ = std::os::unix::fs::chown(path, Some(uid), group);
    }
}

/// Windows has nothing to do here: a file written into a user's profile
/// inherits that folder's access rules.
#[cfg(not(unix))]
fn give_to_invoker(path: &Path) {
    let _ = path;
}
