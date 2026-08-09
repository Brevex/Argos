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
/// `elevated` asks the operating system for the privileges a raw device needs.
/// An image-file scan needs none and should not ask.
#[tauri::command]
pub async fn connect(
    app: AppHandle,
    shell: State<'_, Shell>,
    elevated: bool,
) -> Result<(), String> {
    let handle = app.clone();
    let engine = tauri::async_runtime::spawn_blocking(move || Engine::connect(&handle, elevated))
        .await
        .map_err(|err| format!("the connection attempt did not finish: {err}"))??;
    shell.replace(engine);
    Ok(())
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
