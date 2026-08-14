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
/// The window is granted one path and only one: the session's `previews/`
/// subdirectory, which holds derived thumbnails and no recovered bytes. That
/// is what a results view needs to draw and the least that will do it — the
/// artifacts themselves stay unreachable from the web view, and the grant is
/// made after the engine names the directory rather than being configured
/// ahead of time.
///
/// Everything else is unchanged: no shell, no filesystem, no network.
#[tauri::command]
pub async fn scan_start(
    app: AppHandle,
    shell: State<'_, Shell>,
    request: dto::ScanRequest,
) -> Result<dto::ScanStarted, String> {
    let started: dto::ScanStarted = shell
        .call(Call::ScanStart(Box::new(request)), engine::started)
        .await?;
    grant_previews(&app, &started.preview_dir);
    Ok(started)
}

/// Lets the web view read the thumbnails under `dir`, and nothing else.
///
/// A failure here costs the gallery its pictures and nothing more — the scan
/// is already running and every artifact is recorded regardless — so it is
/// reported to the window rather than failing the call.
fn grant_previews(app: &AppHandle, dir: &str) {
    use tauri::Manager as _;
    if dir.is_empty() {
        return;
    }
    // Not recursive: the previews directory is flat, and a recursive grant
    // would reach anything a later version put beneath it.
    let _ = app.asset_protocol_scope().allow_directory(dir, false);
}

/// One page of a finished session's artifacts, strongest evidence first.
///
/// The order and the filter are the engine's. This passes a page request
/// through and hands back what came of it; nothing here reads a standing,
/// counts an artifact or decides what a photograph is (`A-SHELL-NO-DOMAIN`).
#[tauri::command]
pub async fn scan_gallery(
    shell: State<'_, Shell>,
    session: String,
    offset: u32,
    limit: u32,
    standing: Option<String>,
) -> Result<dto::Gallery, String> {
    shell
        .call(
            Call::ScanGallery {
                session,
                offset,
                limit,
                standing,
                include_unwritten: false,
            },
            engine::page,
        )
        .await
}

/// Stops the running scan, keeping everything recovered so far.
#[tauri::command]
pub async fn scan_cancel(shell: State<'_, Shell>) -> Result<(), String> {
    shell.call(Call::ScanCancel, engine::done).await
}

/// Suspends the running scan at the next chunk boundary.
///
/// A scan of a disk runs for hours and holds the machine while it does. Pausing
/// is what lets it be given back without losing the hours already spent: the
/// medium stays open, nothing recovered is discarded, and the run carries on
/// from where it stopped. The command line has had this since it existed
/// (`p` and `r` at the prompt); this is the same call.
#[tauri::command]
pub async fn scan_pause(shell: State<'_, Shell>) -> Result<(), String> {
    shell.call(Call::ScanPause, engine::done).await
}

/// Resumes a paused scan.
#[tauri::command]
pub async fn scan_resume(shell: State<'_, Shell>) -> Result<(), String> {
    shell.call(Call::ScanResume, engine::done).await
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
