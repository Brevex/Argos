//! Recovery result DTO

use crate::domain::entities::FileType;
use crate::utils::format_bytes;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Result of a recovery operation
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    /// Source device/file path
    pub source_path: String,
    /// Output directory
    pub output_dir: PathBuf,
    /// Total files recovered
    pub files_recovered: usize,
    /// Files by type
    pub files_by_type: HashMap<FileType, usize>,
    /// Total bytes recovered
    pub bytes_recovered: u64,
    /// Total bytes scanned
    pub bytes_scanned: u64,
    /// Duration of the operation
    pub duration: Duration,
    /// Errors encountered
    pub errors: Vec<String>,
    /// Files that failed to recover
    pub failed_files: usize,
}

impl RecoveryResult {
    /// Creates a new recovery result
    pub fn new(source_path: String, output_dir: PathBuf) -> Self {
        Self {
            source_path,
            output_dir,
            files_recovered: 0,
            files_by_type: HashMap::new(),
            bytes_recovered: 0,
            bytes_scanned: 0,
            duration: Duration::ZERO,
            errors: Vec::new(),
            failed_files: 0,
        }
    }

    /// Adds a recovered file to the result
    pub fn add_recovered(&mut self, file_type: FileType, size: u64) {
        self.files_recovered += 1;
        *self.files_by_type.entry(file_type).or_insert(0) += 1;
        self.bytes_recovered += size;
    }

    /// Adds an error
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.failed_files += 1;
    }

    /// Returns success rate (0.0 - 1.0)
    pub fn success_rate(&self) -> f64 {
        let total = self.files_recovered + self.failed_files;
        if total == 0 {
            return 1.0;
        }
        self.files_recovered as f64 / total as f64
    }

    /// Returns a summary string
    pub fn summary(&self) -> String {
        let mut summary = String::new();

        summary.push_str(&format!(
            "Recovery complete: {} files recovered ({})\n",
            self.files_recovered,
            format_bytes(self.bytes_recovered)
        ));

        summary.push_str(&format!(
            "Scanned {} in {:.2}s\n",
            format_bytes(self.bytes_scanned),
            self.duration.as_secs_f64()
        ));

        for (file_type, count) in &self.files_by_type {
            summary.push_str(&format!("  - {}: {}\n", file_type, count));
        }

        if !self.errors.is_empty() {
            summary.push_str(&format!("\n{} errors occurred\n", self.errors.len()));
        }

        summary
    }
}
