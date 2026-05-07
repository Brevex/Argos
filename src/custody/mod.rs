use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

use crate::error::ArgosError;

pub fn hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Open,
    Read,
    Recover,
    Close,
    BadSector,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Error,
    Partial,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub operation: Operation,
    pub source_id: String,
    pub output_id: Option<String>,
    pub byte_range: Option<(u64, u64)>,
    pub status: Status,
    pub previous_hash: Option<String>,
}

impl AuditEntry {
    pub fn new(
        operation: Operation,
        source_id: String,
        output_id: Option<String>,
        byte_range: Option<(u64, u64)>,
        status: Status,
    ) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            operation,
            source_id,
            output_id,
            byte_range,
            status,
            previous_hash: None,
        }
    }
}

pub struct AuditLog {
    file: std::fs::File,
    last_hash: Option<[u8; 32]>,
}

impl AuditLog {
    pub fn open(path: &Path) -> Result<Self, ArgosError> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file, last_hash: None })
    }

    pub fn append(&mut self, mut entry: AuditEntry) -> Result<(), ArgosError> {
        entry.previous_hash = self.last_hash.map(hex::encode);
        let mut buf = Vec::with_capacity(256);
        serde_json::to_writer(&mut buf, &entry)?;
        buf.push(b'\n');
        self.file.write_all(&buf)?;
        self.file.flush()?;
        self.last_hash = Some(hash(&buf));
        Ok(())
    }
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog").finish_non_exhaustive()
    }
}

pub struct BadSectorMap {
    entries: Vec<(u64, u64)>,
}

impl Default for BadSectorMap {
    fn default() -> Self {
        Self::new()
    }
}

impl BadSectorMap {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn record(&mut self, offset: u64, length: u64) {
        self.entries.push((offset, length));
    }

    pub fn entries(&self) -> &[(u64, u64)] {
        &self.entries
    }

    pub fn write_to(&self, path: &Path) -> Result<(), ArgosError> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        for (offset, length) in &self.entries {
            writeln!(file, "{offset},{length}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for BadSectorMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BadSectorMap")
            .field("count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn hash_is_deterministic() {
        let a = hash(b"hello");
        let b = hash(b"hello");
        assert_eq!(a, b);
        assert_ne!(a, hash(b"world"));
    }

    #[test]
    fn audit_log_appends_and_chains() -> Result<(), ArgosError> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let mut log = AuditLog::open(&path)?;

        let entry = AuditEntry::new(
            Operation::Open,
            "src_1".into(),
            None,
            None,
            Status::Ok,
        );
        log.append(entry)?;

        let entry = AuditEntry::new(
            Operation::Read,
            "src_1".into(),
            None,
            Some((0, 4096)),
            Status::Ok,
        );
        log.append(entry)?;

        drop(log);

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["operation"], "open");
        assert!(first["previous_hash"].is_null());

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["operation"], "read");
        assert!(second["previous_hash"].is_string());
        Ok(())
    }

    #[test]
    fn bad_sector_map_records_and_writes() -> Result<(), ArgosError> {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad_sectors.csv");
        let mut map = BadSectorMap::new();
        map.record(0, 512);
        map.record(4096, 512);
        map.write_to(&path)?;

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("0,512"));
        assert!(content.contains("4096,512"));
        assert_eq!(map.entries().len(), 2);
        Ok(())
    }
}
