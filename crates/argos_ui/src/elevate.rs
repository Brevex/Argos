//! Making sure this process is privileged before it draws anything.
//!
//! Reading `/dev/sda` or `\\.\PhysicalDrive0` needs root or Administrator, and
//! there is no way to acquire that after a process has started. So the whole
//! application asks for it up front, on every platform, and the engine it
//! spawns inherits it as an ordinary child. There is no unprivileged mode to
//! fall into: a scan that quietly ran without privileges would report a medium
//! it could not read as an empty one.
//!
//! - **Windows** declares `requireAdministrator` in its manifest (see
//!   `build.rs`), so consent is given before the window exists.
//! - **macOS** relaunches itself through `osascript` and exits.
//! - **Linux** relaunches itself through `pkexec` and exits. `pkexec` hands the
//!   child a deliberately minimal environment, so the session the window needs
//!   to draw at all travels as arguments and is put back by the privileged
//!   process — see [`session`].
//!
//! Step 6 of `docs/DEVICE-SMOKE-CHECKLIST.md` is where each platform gets
//! confirmed on real hardware; until a row appears in its results table, none
//! of it has been run.

use std::process::Command;

/// Marks the relaunched process on macOS, so it does not relaunch again.
#[cfg(target_os = "macos")]
const MARKER: &str = "ARGOS_ELEVATED";

/// Root's home directory, readable by root and by nobody else.
///
/// Whether this process actually holds the privileges it asked for is a
/// question with one honest answer and no way to guess it.
#[cfg(target_os = "macos")]
const ROOT_ONLY: &str = "/var/root";

/// What the shell must do before it draws anything.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos")),
    expect(
        dead_code,
        reason = "only a platform that relaunches itself can report anything but Proceed; \
                  Windows was already prompted by its manifest"
    )
)]
#[derive(Debug)]
pub enum Start {
    /// Carry on: this process is privileged.
    Proceed,
    /// This process relaunched itself with privileges and should now exit,
    /// leaving the elevated copy to draw the window.
    Relaunched,
    /// Privileges were refused. The reason is for the user, who chose this.
    Refused(String),
}

/// Decides whether this process may draw a window yet.
pub fn start() -> Start {
    platform_start()
}

/// Tells the user why the application is closing.
///
/// A refusal happens before any window exists and, when the application was
/// launched from a desktop menu, with no console to write to either. So it
/// goes through whatever the platform will show: an alert on macOS, the
/// desktop's own dialog helper on Linux, and stderr when there is none.
pub fn report(reason: &str) {
    platform_report(reason);
}

/// The home directory of the user this privileged process is acting for.
///
/// A file dialog run by root would otherwise open in root's home, which is not
/// where anyone keeps anything. `None` on a platform or a start where the
/// question does not arise, and the dialog then does what it did before.
#[must_use]
pub fn invoker_home() -> Option<String> {
    std::env::var(session::INVOKER_HOME)
        .ok()
        .filter(|home| !home.is_empty())
}

/// Runs `binary serve`.
///
/// No elevation happens here: this process is already privileged and the child
/// inherits that, on every platform.
#[must_use]
pub fn engine(binary: &str) -> Command {
    let mut command = Command::new(binary);
    command.arg("serve");
    command
}

// ---------------------------------------------------------------- Linux

/// Linux: relaunch through `pkexec`, or put the session back and proceed.
#[cfg(target_os = "linux")]
fn platform_start() -> Start {
    match session::carried() {
        // Privileged, and holding the session as arguments: replace this
        // process with one that has them in its environment, where GTK reads
        // them. `restore` only returns when the replacement failed.
        Some(carried) if identity::is_root() => Start::Refused(format!(
            "Argos could not restart itself with the desktop session it needs: {}",
            session::restore(&carried)
        )),
        // Privileged and already environed: `sudo argos-shell` in a terminal,
        // or the process the replacement above produced.
        None if identity::is_root() => Start::Proceed,
        // Not privileged. Ask.
        _ => match relaunch() {
            Ok(()) => Start::Relaunched,
            Err(reason) => Start::Refused(reason),
        },
    }
}

/// Asks `pkexec` to start a privileged copy, carrying the session with it.
#[cfg(target_os = "linux")]
fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| {
        format!("Argos could not locate its own program file, so it cannot restart itself with the privileges reading a disk needs: {err}")
    })?;

    let status = Command::new("pkexec")
        .arg(exe)
        .args(session::arguments())
        .status()
        .map_err(|err| {
            format!(
                "Argos could not ask for administrator privileges — is `pkexec` installed? ({err})"
            )
        })?;
    if status.success() {
        return Ok(());
    }
    // 126 is `pkexec`'s "not authorised", 127 its "dismissed or failed".
    match status.code() {
        Some(126 | 127) => Err(
            "Argos needs administrator privileges to read a disk. The request \
                                was dismissed, so it did not open."
                .to_owned(),
        ),
        _ => Err(format!(
            "Argos could not obtain administrator privileges ({status})."
        )),
    }
}

#[cfg(target_os = "linux")]
fn platform_report(reason: &str) {
    // Whichever of these the desktop has. A menu launch has no console, so a
    // line on stderr would be a refusal nobody sees.
    for dialog in [
        Command::new("zenity")
            .args(["--error", "--title=Argos", "--text", reason])
            .status(),
        Command::new("kdialog")
            .args(["--title", "Argos", "--error", reason])
            .status(),
    ] {
        if matches!(dialog, Ok(status) if status.success()) {
            return;
        }
    }
    eprintln!("argos: {reason}");
}

// ---------------------------------------------------------------- Windows

/// Windows: the manifest already asked, and Windows already prompted.
#[cfg(windows)]
fn platform_start() -> Start {
    Start::Proceed
}

#[cfg(windows)]
fn platform_report(reason: &str) {
    eprintln!("argos: {reason}");
}

// ---------------------------------------------------------------- macOS

/// macOS: relaunch through `osascript` unless this is the relaunched copy.
#[cfg(target_os = "macos")]
fn platform_start() -> Start {
    if std::env::var_os(MARKER).is_some() {
        return if std::fs::read_dir(ROOT_ONLY).is_ok() {
            Start::Proceed
        } else {
            Start::Refused(
                "Argos was restarted to gain administrator privileges and did not receive them, \
                 so it cannot read a disk. Nothing was scanned."
                    .to_owned(),
            )
        };
    }

    let Ok(exe) = std::env::current_exe() else {
        return Start::Refused(
            "Argos could not locate its own program file, so it cannot restart itself with the \
             privileges reading a disk needs."
                .to_owned(),
        );
    };
    match relaunch(&exe.to_string_lossy()) {
        Ok(()) => Start::Relaunched,
        Err(reason) => Start::Refused(reason),
    }
}

/// Starts a privileged copy of `exe` and leaves it running.
///
/// The command is backgrounded so `osascript` returns as soon as the copy is
/// started rather than holding the session for the length of a scan, and its
/// streams are redirected because nothing is listening on them.
#[cfg(target_os = "macos")]
fn relaunch(exe: &str) -> Result<(), String> {
    let assignments = session::assignments()
        .iter()
        .map(|pair| script::shell_quote(pair))
        .collect::<Vec<_>>()
        .join(" ");
    let shell = format!(
        "{MARKER}=1 env {assignments} {} >/dev/null 2>&1 &",
        script::shell_quote(exe)
    );
    let statement = format!(
        "do shell script {} with administrator privileges",
        script::applescript_quote(&shell)
    );
    let output = Command::new("osascript")
        .arg("-e")
        .arg(statement)
        .output()
        .map_err(|err| format!("Argos could not ask for administrator privileges: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    // -128 is what `osascript` reports when the person dismisses the prompt.
    // It is a decision, not a failure, and saying so beats repeating the error
    // text at someone who just made it.
    if said.contains("-128") {
        return Err(
            "Argos needs administrator privileges to read a disk. The request was dismissed, so \
             it did not open."
                .to_owned(),
        );
    }
    Err(format!(
        "Argos could not obtain administrator privileges: {}",
        said.trim()
    ))
}

#[cfg(target_os = "macos")]
fn platform_report(reason: &str) {
    let alert = format!(
        "display alert \"Argos\" message {} as critical",
        script::applescript_quote(reason)
    );
    let _ = Command::new("osascript").arg("-e").arg(alert).status();
}

// ---------------------------------------------------------------- other

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_start() -> Start {
    Start::Proceed
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_report(reason: &str) {
    eprintln!("argos: {reason}");
}

/// Carrying a desktop session across an elevation that deliberately drops it.
///
/// `pkexec` replaces the environment with a minimal one, which is the whole
/// point of it: `LD_PRELOAD`, `LD_LIBRARY_PATH`, `GTK_MODULES` and
/// `GIO_EXTRA_MODULES` all make a process load code chosen by whoever called
/// it, and a privileged process must not inherit any of them. A window still
/// needs to know which display to draw on, so a fixed list of variables — and
/// nothing outside it — is passed as arguments and put back on the other side.
///
/// The list is closed in both directions: this side only reads these names,
/// and the privileged side only accepts these names.
#[cfg_attr(
    not(target_os = "linux"),
    expect(
        dead_code,
        reason = "carrying the session as arguments is pkexec's problem alone: macOS hands the \
                  assignments to a shell and Windows keeps the environment it started with, so \
                  neither reads them back"
    )
)]
pub mod session {
    /// User id this process is running on behalf of.
    pub const INVOKER_UID: &str = "ARGOS_INVOKER_UID";
    /// Group id this process is running on behalf of.
    pub const INVOKER_GID: &str = "ARGOS_INVOKER_GID";
    /// Home directory of that user, for a file dialog to open in.
    pub const INVOKER_HOME: &str = "ARGOS_INVOKER_HOME";

    /// Every variable an elevated copy is allowed to be given.
    ///
    /// Displays and locale, so a window can be drawn and labelled, plus the
    /// three that say who asked. Nothing here can make the process load code.
    pub const ALLOWED: [&str; 11] = [
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "DBUS_SESSION_BUS_ADDRESS",
        "LANG",
        INVOKER_UID,
        INVOKER_GID,
        INVOKER_HOME,
    ];

    /// Argument introducing one carried variable.
    const FLAG: &str = "--session";

    /// This process' session as `NAME=VALUE` pairs.
    #[must_use]
    pub fn assignments() -> Vec<String> {
        ALLOWED
            .iter()
            .filter_map(|name| value_of(name).map(|value| format!("{name}={value}")))
            .collect()
    }

    /// The same, as the arguments a privileged copy is started with.
    #[must_use]
    pub fn arguments() -> Vec<String> {
        assignments()
            .into_iter()
            .flat_map(|pair| [FLAG.to_owned(), pair])
            .collect()
    }

    /// Reads one variable, resolving the three this process derives itself.
    #[cfg(unix)]
    fn value_of(name: &str) -> Option<String> {
        match name {
            INVOKER_UID => super::identity::uid().map(|id| id.to_string()),
            INVOKER_GID => super::identity::gid().map(|id| id.to_string()),
            INVOKER_HOME => std::env::var("HOME").ok(),
            other => std::env::var(other).ok(),
        }
        .filter(|value| !value.is_empty())
    }

    #[cfg(not(unix))]
    fn value_of(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    /// The variables this process was given, if it was given any.
    ///
    /// `None` when no `--session` argument is present, which is how a process
    /// that is already environed recognises itself.
    #[must_use]
    pub fn carried() -> Option<Vec<(String, String)>> {
        let mut carried = Vec::new();
        let mut args = std::env::args().skip(1);
        let mut seen = false;
        while let Some(arg) = args.next() {
            if arg != FLAG {
                continue;
            }
            seen = true;
            if let Some(pair) = args.next()
                && let Some(assignment) = accept(&pair)
            {
                carried.push(assignment);
            }
        }
        seen.then_some(carried)
    }

    /// Splits one `NAME=VALUE` argument, rejecting any name off the list.
    ///
    /// The rejection is the security boundary, not a formality: these values
    /// arrive from an unprivileged caller and are about to become the
    /// environment of a root process.
    #[must_use]
    pub fn accept(pair: &str) -> Option<(String, String)> {
        let (name, value) = pair.split_once('=')?;
        ALLOWED
            .contains(&name)
            .then(|| (name.to_owned(), value.to_owned()))
    }

    /// Replaces this process with one holding `carried` in its environment.
    ///
    /// Returns only when the replacement failed, and returns the reason: on
    /// success there is no longer a process to return into.
    #[cfg(unix)]
    #[must_use]
    pub fn restore(carried: &[(String, String)]) -> std::io::Error {
        use std::os::unix::process::CommandExt;

        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(err) => return err,
        };
        let mut command = std::process::Command::new(exe);
        // Everything except the arguments that got us here, so the replacement
        // sees itself as an ordinary privileged start.
        command.args(without_session(std::env::args().skip(1)));
        for (name, value) in carried {
            command.env(name, value);
        }
        command.exec()
    }

    /// The arguments with every `--session NAME=VALUE` pair removed.
    fn without_session(args: impl Iterator<Item = String>) -> Vec<String> {
        let mut kept = Vec::new();
        let mut skip_next = false;
        for arg in args {
            if std::mem::take(&mut skip_next) {
                continue;
            }
            if arg == FLAG {
                skip_next = true;
                continue;
            }
            kept.push(arg);
        }
        kept
    }

    #[cfg(test)]
    mod tests {
        use super::{accept, without_session};

        #[test]
        fn a_carried_variable_on_the_list_is_taken() {
            assert_eq!(
                accept("WAYLAND_DISPLAY=wayland-0"),
                Some(("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned()))
            );
        }

        #[test]
        fn a_value_containing_an_equals_sign_survives() {
            // `DBUS_SESSION_BUS_ADDRESS` always looks like this, and splitting
            // on the last `=` instead of the first would truncate it.
            assert_eq!(
                accept("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus"),
                Some((
                    "DBUS_SESSION_BUS_ADDRESS".to_owned(),
                    "unix:path=/run/user/1000/bus".to_owned()
                ))
            );
        }

        #[test]
        fn anything_that_could_load_code_is_refused() {
            // The whole reason `pkexec` clears the environment. A name off the
            // list must not reach a root process, whatever it is.
            for hostile in [
                "LD_PRELOAD=/tmp/evil.so",
                "LD_LIBRARY_PATH=/tmp",
                "GTK_MODULES=/tmp/evil",
                "GIO_EXTRA_MODULES=/tmp",
                "PATH=/tmp",
                "HOME=/tmp",
            ] {
                assert_eq!(accept(hostile), None, "{hostile} must not be carried");
            }
        }

        #[test]
        fn a_malformed_argument_is_refused() {
            assert_eq!(accept("DISPLAY"), None);
            assert_eq!(accept(""), None);
        }

        #[test]
        fn the_replacement_does_not_carry_the_session_again() {
            let args = [
                "--session".to_owned(),
                "DISPLAY=:0".to_owned(),
                "--other".to_owned(),
                "--session".to_owned(),
                "LANG=pt_BR.UTF-8".to_owned(),
            ];
            assert_eq!(
                without_session(args.into_iter()),
                vec!["--other".to_owned()],
                "a replacement that saw --session again would elevate in a loop"
            );
        }
    }
}

/// Who this process is, without asking libc.
///
/// `argos_ui` forbids `unsafe`, so `geteuid` is out of reach. A file this
/// process owns answers the same question, and `/proc/self` is exactly that:
/// the kernel creates it owned by the process' effective user.
#[cfg(unix)]
mod identity {
    use std::os::unix::fs::MetadataExt;

    /// Whether this process is root.
    #[cfg(target_os = "linux")]
    pub fn is_root() -> bool {
        uid() == Some(0)
    }

    /// This process' user id, or `None` where `/proc` cannot be read.
    ///
    /// `None` is reported rather than guessed: a wrong answer here would hand
    /// the recovered files to the wrong account.
    pub fn uid() -> Option<u32> {
        self_metadata().map(|meta| meta.uid())
    }

    /// This process' group id, on the same terms as [`uid`].
    pub fn gid() -> Option<u32> {
        self_metadata().map(|meta| meta.gid())
    }

    fn self_metadata() -> Option<std::fs::Metadata> {
        std::fs::metadata("/proc/self").ok()
    }
}

/// Quoting for the two languages an elevated relaunch passes through on macOS.
///
/// A `.app` can sit in a directory whose name contains a space, a quote or a
/// backslash, and the path travels through `AppleScript` into `sh`. Both of
/// these are pure functions over a string, so they are compiled and tested on
/// every platform even though only macOS calls them: a change to them fails on
/// the machine making the change rather than on a release runner.
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(
        dead_code,
        reason = "only macOS relaunches through a shell; the tests below are what keep the \
                  quoting honest on every other platform"
    )
)]
mod script {
    /// Wraps `text` as one POSIX shell word.
    ///
    /// Single quotes suspend every expansion the shell has; the only character
    /// they cannot carry is a single quote, which is closed, escaped and
    /// reopened.
    pub fn shell_quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    /// Wraps `text` as one `AppleScript` string literal.
    pub fn applescript_quote(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::script::{applescript_quote, shell_quote};

    #[test]
    fn the_engine_is_spawned_as_an_ordinary_child() {
        // No elevation at this point on any platform: this process already
        // holds what the engine needs. A command that tried to elevate again
        // would prompt a second time for privileges it already has.
        let command = format!("{:?}", super::engine("/bin/true"));
        assert!(command.contains("serve"), "{command}");
        assert!(
            !command.contains("pkexec") && !command.contains("osascript"),
            "the engine inherits privileges rather than asking for them: {command}"
        );
    }

    #[test]
    fn a_quoted_path_survives_the_shell() {
        assert_eq!(
            shell_quote("/Applications/Argos.app"),
            "'/Applications/Argos.app'"
        );
        assert_eq!(shell_quote("/My Apps/Argos.app"), "'/My Apps/Argos.app'");
        assert_eq!(shell_quote("/it's here/Argos"), r"'/it'\''s here/Argos'");
    }

    #[test]
    fn a_quoted_path_survives_applescript() {
        assert_eq!(applescript_quote("plain"), "\"plain\"");
        assert_eq!(applescript_quote(r"back\slash"), r#""back\\slash""#);
        assert_eq!(applescript_quote(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn a_path_through_both_layers_comes_out_whole() {
        // What the elevated `sh` finally receives: AppleScript unescapes its
        // literal, then the shell unescapes the single-quoted word.
        let quoted = applescript_quote(&shell_quote(r#"/A "B"\C/Argos"#));
        assert_eq!(quoted, r#""'/A \"B\"\\C/Argos'""#);
    }
}
