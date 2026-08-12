//! The commands the window may call.
//!
//! Every one of them is the same three steps: take a DTO, hand it to the
//! engine, give back what the engine said. There is no branching on recovery
//! data anywhere in this file, and there must not be — a condition on a
//! confidence tier, a threshold on a score or a decision about an extent would
//! be recovery logic living in a presentation layer (`A-SHELL-NO-DOMAIN`).
//!
//! Each of these is also reachable from `argos` as a subcommand. That is not a
//! coincidence to be maintained by hand: both this and the command line drive
//! the engine's one scan driver (`A-CLI-FIRST`).

use argos_ipc::dto;
use argos_ipc::wire::Call;
use tauri::{AppHandle, State};

use crate::Shell;
use crate::engine::{self, Engine};

/// Connects to the engine, replacing any previous connection.
///
/// Nothing here decides about privileges. This process holds them already —
/// `crate::elevate` made sure of that before the window existed — and the
/// engine inherits them by being its child.
#[tauri::command]
pub async fn connect(app: AppHandle, shell: State<'_, Shell>) -> Result<(), String> {
    let handle = app.clone();
    let engine = tauri::async_runtime::spawn_blocking(move || Engine::connect(&handle))
        .await
        .map_err(|err| format!("the connection attempt did not finish: {err}"))??;
    shell.replace(engine);
    Ok(())
}

/// The home directory of the user this administrator session is acting for.
///
/// Empty when the platform has no answer. A path is not filesystem access:
/// this is a string the folder picker opens at, and the picker is the only
/// thing in this window that touches a directory.
#[expect(
    clippy::unnecessary_wraps,
    reason = "a Tauri command's signature is its contract with the window, and every other one \
              here can fail"
)]
#[tauri::command]
pub fn invoker_home() -> Result<String, String> {
    Ok(crate::elevate::invoker_home().unwrap_or_default())
}

/// The media this machine exposes.
#[tauri::command]
pub async fn devices(shell: State<'_, Shell>) -> Result<dto::Inventory, String> {
    shell.call(Call::DevicesList, engine::inventory).await
}

/// Starts a scan.
///
/// The window is granted no path into the session directory, by design: it
/// shows counts and progress, never recovered content. What was recovered is
/// read from the output directory by whoever is entitled to it, or through
/// `argos report` and `argos export`.
#[tauri::command]
pub async fn scan_start(
    shell: State<'_, Shell>,
    request: dto::ScanRequest,
) -> Result<dto::ScanStarted, String> {
    shell
        .call(Call::ScanStart(Box::new(request)), engine::started)
        .await
}

/// Stops the running scan, keeping everything recovered so far.
#[tauri::command]
pub async fn scan_cancel(shell: State<'_, Shell>) -> Result<(), String> {
    shell.call(Call::ScanCancel, engine::done).await
}

/// The view preferences this account last stored, as JSON text.
///
/// Empty when there are none. The window parses it; nothing here looks inside
/// it, because a preference is presentation and this file carries no decisions
/// about presentation (`A-SHELL-NO-DOMAIN`).
#[expect(
    clippy::unnecessary_wraps,
    reason = "a Tauri command's signature is its contract with the window, and every other one \
              here can fail"
)]
#[tauri::command]
pub fn preferences_read() -> Result<String, String> {
    Ok(crate::preference::read())
}

/// Replaces the stored view preferences with `text`.
///
/// # Errors
///
/// Fails when the file cannot be written, which the window may ignore: a
/// preference that did not persist still applies to the window that set it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a Tauri command receives its arguments owned, deserialized from the invocation"
)]
#[tauri::command]
pub fn preferences_write(text: String) -> Result<(), String> {
    crate::preference::write(&text)
}
