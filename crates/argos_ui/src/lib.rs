//! Argos' presentation shell.
//!
//! This crate draws windows. It does not recover anything, and it could not:
//! its only dependency from this project is `argos_ipc`, the wire format, and
//! that crate depends on nothing at all. There is no carver here to call, no
//! filesystem parser, no confidence model and no classifier — not by
//! discipline but because none of them are linked (`A-SHELL-NO-DOMAIN`).
//!
//! What it does is spawn `argos --serve` and translate between that process
//! and a web view: commands out, events in. Every command here is a subcommand
//! of `argos` as well, so anything this window can do is reproducible from a
//! terminal (`A-CLI-FIRST`).
//!
//! This process is privileged before it draws anything, on every platform (see
//! [`elevate`]), and the engine inherits that. The web view runs inside it, so
//! what keeps the privilege contained is the capability list, the content
//! security policy, and the fact that this crate links no recovery logic to
//! reach: nothing here loads a remote origin, and nothing here is allowed to
//! run a shell, touch the filesystem or open a socket.

mod commands;
mod elevate;
mod engine;
mod preference;
mod trace;

use std::sync::Mutex;

use argos_ipc::wire::{Call, Reply};
use tauri::Manager;

use engine::Engine;

/// The shell's whole state: at most one engine connection.
///
/// One at a time, because a scan holds a medium open and two scans of the same
/// medium would compete for it. A second `connect` replaces the first, which
/// closes its pipes and stops whatever it was doing.
#[derive(Debug, Default)]
pub struct Shell {
    engine: Mutex<Option<std::sync::Arc<Engine>>>,
}

impl Shell {
    /// Installs `engine` as the connection, dropping any previous one.
    fn replace(&self, engine: Engine) {
        *self.lock() = Some(std::sync::Arc::new(engine));
    }

    /// Sends one call to the connected engine and narrows its answer.
    ///
    /// Runs on a blocking thread: the engine answers at the speed of a disk,
    /// and the window has to keep drawing while it does.
    async fn call<T: Send + 'static>(
        &self,
        call: Call,
        narrow: fn(Reply) -> Option<T>,
    ) -> Result<T, String> {
        let engine = self
            .lock()
            .clone()
            .ok_or("not connected to the recovery engine")?;
        tauri::async_runtime::spawn_blocking(move || {
            engine
                .call(call)
                .and_then(|reply| engine::expect(reply, narrow))
        })
        .await
        .map_err(|err| format!("the call did not finish: {err}"))?
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<std::sync::Arc<Engine>>> {
        self.engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Runs the shell.
///
/// # Panics
///
/// Panics when the application cannot be built or the event loop cannot start,
/// which is a broken installation rather than anything a user did
/// (`M-PANIC-ON-BUG`).
pub fn run() {
    match elevate::start() {
        elevate::Start::Proceed => {}
        // The privileged copy is the one that draws. Two windows over one
        // medium would compete for it.
        elevate::Start::Relaunched => return,
        elevate::Start::Refused(reason) => {
            elevate::report(&reason);
            return;
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(Shell::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::invoker_home,
            commands::preferences_read,
            commands::preferences_write,
            commands::devices,
            commands::scan_start,
            commands::scan_pause,
            commands::scan_resume,
            commands::scan_cancel,
            commands::acquire_start,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| panic!("the shell could not start: {err}"));
}
