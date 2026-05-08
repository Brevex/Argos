use tauri::{AppHandle, State};

use crate::bridge::{
    BridgeError, CancelRequest, ScopedPath, SessionManager, StartRequest, StartResponse,
};

#[tauri::command]
pub async fn start_recovery(
    request: StartRequest,
    manager: State<'_, SessionManager>,
    app: AppHandle,
) -> Result<StartResponse, BridgeError> {
    let tmp = std::env::temp_dir();
    let source = ScopedPath::new(&request.source, &[&tmp])?;
    let output = ScopedPath::new(&request.output, &[&tmp])?;

    let session_id = manager.create();
    let session = manager.get(session_id).unwrap();

    let src = source.as_path().to_path_buf();
    let out = output.as_path().to_path_buf();

    std::thread::spawn(move || {
        let result = crate::bridge::runner::run(&src, &out, &session, &app);
        if let Err(ref e) = result {
            tracing::error!(error = ?e, session_id, "runner failed");
        }
        crate::bridge::runner::emit_progress(&app, session_id, 0, 0, 0);
    });

    Ok(StartResponse { session_id })
}

#[tauri::command]
pub async fn cancel_recovery(
    request: CancelRequest,
    manager: State<'_, SessionManager>,
) -> Result<(), BridgeError> {
    if manager.cancel(request.session_id) {
        Ok(())
    } else {
        Err(BridgeError {
            kind: crate::bridge::BridgeErrorKind::Denied,
            detail: "session not found".into(),
        })
    }
}
