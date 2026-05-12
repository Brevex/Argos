use std::process::ExitCode;

use argos::bridge::{SessionManager, commands};
use argos::elevation::{self, Outcome};

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt::try_init();

    match elevation::ensure() {
        Ok(Outcome::AlreadyElevated) => run_application(),
        Ok(Outcome::Relaunched { exit_code }) => exit_code_into(exit_code),
        Err(error) => {
            tracing::error!(error = ?error, "privilege elevation failed");
            ExitCode::from(2)
        }
    }
}

fn run_application() -> ExitCode {
    let _pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .build_global();

    let session_manager = SessionManager::new();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session_manager)
        .invoke_handler(tauri::generate_handler![
            commands::start_recovery,
            commands::cancel_recovery,
            commands::list_devices,
            commands::default_output_dir,
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "tauri runtime failed");
            ExitCode::from(1)
        }
    }
}

fn exit_code_into(code: i32) -> ExitCode {
    let byte: u8 = u8::try_from(code).unwrap_or(1);
    ExitCode::from(byte)
}
