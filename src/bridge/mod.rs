use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeErrorKind {
    Io,
    Allocation,
    Unsupported,
    PatternBuild,
    Validation,
    AuditSerialization,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeError {
    pub kind: BridgeErrorKind,
    pub detail: String,
}

impl From<crate::error::ArgosError> for BridgeError {
    fn from(e: crate::error::ArgosError) -> Self {
        use crate::error::ArgosError;
        let (kind, detail) = match &e {
            ArgosError::Io(_) => (BridgeErrorKind::Io, "io operation failed".into()),
            ArgosError::Allocation { size, align } => (
                BridgeErrorKind::Allocation,
                format!("size={size}, align={align}"),
            ),
            ArgosError::Unsupported => (BridgeErrorKind::Unsupported, "unsupported platform".into()),
            ArgosError::PatternBuild(_) => (BridgeErrorKind::PatternBuild, "pattern build failed".into()),
            ArgosError::Validation { kind } => (BridgeErrorKind::Validation, format!("{kind}")),
            ArgosError::AuditSerialization(_) => (
                BridgeErrorKind::AuditSerialization,
                "audit serialization failed".into(),
            ),
        };
        Self { kind, detail }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRequest {
    pub source: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartResponse {
    pub session_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelRequest {
    pub session_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub session_id: u64,
    pub bytes_scanned: u64,
    pub candidates_found: u64,
    pub artifacts_recovered: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Ok,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCompletedEvent {
    pub session_id: u64,
    pub status: SessionStatus,
    pub error: Option<BridgeError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEvent {
    pub session_id: u64,
    pub offset: u64,
    pub length: u64,
    pub format: String,
    pub score: f32,
}

pub struct ScopedPath {
    inner: PathBuf,
}

impl ScopedPath {
    pub fn new(path: &str, allowed_prefixes: &[&Path]) -> Result<Self, BridgeError> {
        let parsed = PathBuf::from(path);
        let canonical = parsed.canonicalize().map_err(|e| BridgeError {
            kind: BridgeErrorKind::Io,
            detail: format!("{e}"),
        })?;

        let valid = allowed_prefixes.iter().any(|prefix| {
            canonical.starts_with(prefix)
        });

        if !valid {
            return Err(BridgeError {
                kind: BridgeErrorKind::Denied,
                detail: "path outside allowed scope".into(),
            });
        }

        Ok(Self { inner: canonical })
    }

    pub fn as_path(&self) -> &Path {
        &self.inner
    }
}

impl std::fmt::Debug for ScopedPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedPath").finish_non_exhaustive()
    }
}

pub struct Session {
    pub id: u64,
    pub cancel: AtomicBool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

pub struct SessionManager {
    next_id: AtomicU64,
    sessions: RwLock<HashMap<u64, Arc<Session>>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub fn create(&self) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let session = Arc::new(Session {
            id,
            cancel: AtomicBool::new(false),
        });
        self.sessions.write().insert(id, session);
        id
    }

    pub fn get(&self, id: u64) -> Option<Arc<Session>> {
        self.sessions.read().get(&id).cloned()
    }

    pub fn cancel(&self, id: u64) -> bool {
        if let Some(session) = self.get(id) {
            session.cancel.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, id: u64) {
        self.sessions.write().remove(&id);
    }
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager").finish_non_exhaustive()
    }
}

pub mod commands;
pub mod devices;
pub mod runner;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_error_from_argos_io() {
        let e = crate::error::ArgosError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test",
        ));
        let be = BridgeError::from(e);
        assert!(matches!(be.kind, BridgeErrorKind::Io));
    }

    #[test]
    fn bridge_error_from_argos_validation() {
        let e = crate::error::ArgosError::Validation {
            kind: crate::error::ValidationKind::MissingSoi,
        };
        let be = BridgeError::from(e);
        assert!(matches!(be.kind, BridgeErrorKind::Validation));
    }

    #[test]
    fn scoped_path_accepts_allowed() {
        let tmp = std::env::temp_dir();
        let path = tmp.join("argos_test_allowed");
        std::fs::create_dir_all(&path).ok();
        let sp = ScopedPath::new(path.to_str().unwrap(), &[&tmp]);
        assert!(sp.is_ok());
        std::fs::remove_dir(&path).ok();
    }

    #[test]
    fn scoped_path_rejects_outside() {
        let tmp = std::env::temp_dir();
        let sp = ScopedPath::new("/etc/passwd", &[&tmp]);
        assert!(sp.is_err());
        let err = sp.unwrap_err();
        assert!(matches!(err.kind, BridgeErrorKind::Denied));
    }
}
