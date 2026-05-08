use argos::bridge::{SessionManager, commands};

fn main() {
    let _ = tracing_subscriber::fmt::try_init();

    let _pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .build_global();

    let session_manager = SessionManager::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session_manager)
        .invoke_handler(tauri::generate_handler![
            commands::start_recovery,
            commands::cancel_recovery,
            commands::list_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
