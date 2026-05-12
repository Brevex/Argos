use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::bridge::{
    BridgeError, CancelRequest, ScopedPath, SessionManager, SessionStatus, StartRequest,
    StartResponse,
    devices::{self, DeviceInfo},
};

const RECOVERED_SUBDIR: &str = "Argos_Recovered";

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

fn same_device_warning(source: &Path, output: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let source_meta = std::fs::metadata(source).ok()?;
        let output_meta = std::fs::metadata(output).ok()?;
        let source_dev = if source_meta.file_type().is_block_device()
            || source_meta.file_type().is_char_device()
        {
            source_meta.rdev()
        } else {
            source_meta.dev()
        };
        let output_dev = output_meta.dev();
        if source_dev == output_dev {
            return Some(
                "Source and output are on the same filesystem. Writing recovered data to the analyzed device is not recommended because it may overwrite recoverable data."
                    .into(),
            );
        }
    }
    #[cfg(windows)]
    {
        let source_prefix = source.components().next()?;
        let output_prefix = output.components().next()?;
        if source_prefix == output_prefix {
            return Some(
                "Source and output are on the same volume. Writing recovered data to the analyzed device is not recommended because it may overwrite recoverable data."
                    .into(),
            );
        }
    }
    None
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

    let warning = same_device_warning(source.as_path(), output.as_path());

    let session_id = manager.create();
    if warning.is_some() {
        tracing::warn!(same_device_warning = true, session_id);
    }
    let session = manager.get(session_id).ok_or_else(|| BridgeError {
        kind: crate::bridge::BridgeErrorKind::Denied,
        detail: "session creation failed".into(),
    })?;

    let src = source.as_path().to_path_buf();
    let out = output.as_path().join(RECOVERED_SUBDIR);
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

    Ok(StartResponse { session_id, warning })
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

#[tauri::command]
pub async fn default_output_dir() -> Result<String, BridgeError> {
    Ok(default_output_path().to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
fn default_output_path() -> PathBuf {
    invoking_user_home().unwrap_or_else(|| PathBuf::from("/home"))
}

#[cfg(target_os = "windows")]
fn default_output_path() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

#[cfg(target_os = "linux")]
fn invoking_user_home() -> Option<PathBuf> {
    let uid: u32 = std::env::var("PKEXEC_UID").ok()?.parse().ok()?;
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 6 {
            continue;
        }
        if parts[2].parse::<u32>().ok() == Some(uid) {
            return Some(PathBuf::from(parts[5]));
        }
    }
    None
}
