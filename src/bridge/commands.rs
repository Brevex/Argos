use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::bridge::{
    BridgeError, CancelRequest, ScopedPath, SessionManager, SessionStatus, StartRequest,
    StartResponse,
    devices::{self, DeviceInfo},
};

#[cfg(target_os = "linux")]
const SOURCE_SCOPES: &[&str] = &["/dev", "/tmp", "/var/tmp", "/home", "/media", "/mnt", "/run/media"];

#[cfg(target_os = "linux")]
const OUTPUT_SCOPES: &[&str] = &["/tmp", "/var/tmp", "/home", "/media", "/mnt", "/run/media"];

#[cfg(not(target_os = "linux"))]
const SOURCE_SCOPES: &[&str] = &[];

#[cfg(not(target_os = "linux"))]
const OUTPUT_SCOPES: &[&str] = &[];

fn scope_paths<'a>(prefixes: &'a [&'a str]) -> Vec<&'a Path> {
    prefixes.iter().map(Path::new).collect()
}

#[tauri::command]
pub async fn start_recovery(
    request: StartRequest,
    manager: State<'_, SessionManager>,
    app: AppHandle,
) -> Result<StartResponse, BridgeError> {
    let source_scopes = scope_paths(SOURCE_SCOPES);
    let output_scopes = scope_paths(OUTPUT_SCOPES);
    let source = ScopedPath::new(&request.source, &source_scopes)?;
    let output = ScopedPath::new(&request.output, &output_scopes)?;

    let session_id = manager.create();
    let session = manager.get(session_id).ok_or_else(|| BridgeError {
        kind: crate::bridge::BridgeErrorKind::Denied,
        detail: "session creation failed".into(),
    })?;

    let src = source.as_path().to_path_buf();
    let out = output.as_path().to_path_buf();
    let app = Arc::new(app);

    rayon::spawn(move || {
        let result = crate::bridge::runner::run(&src, &out, &session, app.as_ref());
        let (status, error) = match result {
            Err(e) => {
                tracing::error!(error = ?e, session_id, "runner failed");
                (SessionStatus::Failed, Some(BridgeError::from(e)))
            }
            Ok(()) if session.cancel.load(Ordering::Relaxed) => {
                (SessionStatus::Cancelled, None)
            }
            Ok(()) => (SessionStatus::Ok, None),
        };
        crate::bridge::runner::emit_completed(app.as_ref(), session_id, status, error);
    });

    Ok(StartResponse { session_id })
}

#[tauri::command]
pub async fn list_devices() -> Result<Vec<DeviceInfo>, BridgeError> {
    Ok(devices::list()?)
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
