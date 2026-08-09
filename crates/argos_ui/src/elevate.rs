//! Spawning the engine with the privileges a raw device needs.
//!
//! Reading `/dev/sda` or `\\.\PhysicalDrive0` needs root or Administrator. The
//! window does not: it draws a list and a progress bar. So the privilege lives
//! in the child process and nowhere else — the shell is never elevated, and
//! elevating is only a choice about how the child is spawned. Everything
//! after the spawn is the same JSON-RPC on the same pipes.
//!
//! Scanning a raw image file needs no elevation at all, and the shell does not
//! ask for any.
//!
//! # What is verified and what is not
//!
//! The Linux path uses `pkexec`, which keeps the child's standard streams
//! connected, so nothing else about the connection changes. It is exercised on
//! Linux.
//!
//! **Windows and macOS refuse, and say why.** Both elevate through a shell
//! verb — `ShellExecuteW` with `runas`, and `osascript` with administrator
//! privileges — and **neither preserves the pipes of the process that asked**.
//! A child spawned that way cannot be spoken to over stdio at all, so an
//! elevated engine there needs a rendezvous it connects back on. Refusing with
//! an explanation is the honest behaviour until that exists: elevating and
//! then failing to reach the child looks like a hang, and quietly *not*
//! elevating would produce a scan that reports an empty medium because it
//! could not read one.
//!
//! Step 6 of `docs/DEVICE-SMOKE-CHECKLIST.md` is where this gets confirmed on
//! real hardware; until a row appears in its results table, none of it has
//! been run.

use std::process::Command;

/// A command that runs `binary serve` with raised privileges.
///
/// # Errors
///
/// Fails when this platform has no elevation path implemented.
pub fn command(binary: &str) -> Result<Command, String> {
    platform(binary)
}

/// Linux: `pkexec` prompts through the desktop's authentication agent and
/// keeps the child's standard streams, so the connection is the ordinary one.
#[cfg(target_os = "linux")]
fn platform(binary: &str) -> Result<Command, String> {
    let mut command = Command::new("pkexec");
    // `pkexec` refuses a relative path, and resolving it here rather than
    // letting it search means the elevated process is the binary that was
    // located, not whatever a `PATH` happened to point at.
    let absolute = std::fs::canonicalize(binary)
        .map_err(|err| format!("cannot resolve the engine path {binary}: {err}"))?;
    command.arg(absolute).arg("serve");
    Ok(command)
}

/// Windows: elevation goes through the `runas` shell verb, which does not give
/// the caller the child's pipes.
#[cfg(windows)]
fn platform(_binary: &str) -> Result<Command, String> {
    Err(ELEVATION_NEEDS_RENDEZVOUS.to_owned())
}

/// macOS: `osascript … with administrator privileges` likewise runs the child
/// detached from the caller's streams.
#[cfg(target_os = "macos")]
fn platform(_binary: &str) -> Result<Command, String> {
    Err(ELEVATION_NEEDS_RENDEZVOUS.to_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform(_binary: &str) -> Result<Command, String> {
    Err("this platform has no elevation path".to_owned())
}

/// Said to the user rather than guessed around.
///
/// The alternative to saying this would be to elevate and then fail to talk to
/// the child, which looks like the scan hanging. A scan of an image file needs
/// none of this and still works.
#[cfg(any(windows, target_os = "macos"))]
const ELEVATION_NEEDS_RENDEZVOUS: &str = "this window cannot raise privileges on this platform yet: the operating system's elevation \
     prompt starts the engine without the pipes this connection needs. Run `argos scan` from an \
     elevated terminal for a raw device; scanning an image file needs no privileges and works \
     here.";

#[cfg(test)]
mod tests {
    #[test]
    fn elevation_either_produces_a_command_or_says_why_not() {
        // Whatever the platform, this must be a stated outcome rather than a
        // panic or a silently unprivileged child: a scan that quietly ran
        // without privileges would report an empty medium as an empty medium.
        match super::command("/bin/true") {
            Ok(command) => assert!(
                format!("{command:?}").contains("serve"),
                "an elevated engine is still an engine: {command:?}"
            ),
            Err(reason) => assert!(
                !reason.is_empty(),
                "a refusal has to tell the user what to do instead"
            ),
        }
    }
}
