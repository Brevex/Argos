//! Scan result entity
//!
//! Represents the results of a device scan operation.

use super::file_signature::{FileType, SignatureMatch};
use std::collections::HashMap;
use std::time::Duration;

/// Progress information during a scan
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Total bytes to scan
    pub total_bytes: u64,
    /// Bytes scanned so far
    pub scanned_bytes: u64,
    /// Number of matches found so far
    pub matches_found: usize,
    /// Estimated time remaining
    pub estimated_remaining: Option<Duration>,
    /// Current scan speed in bytes per second
    pub speed_bps: u64,
}

impl ScanProgress {
    /// Creates a new scan progress
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            scanned_bytes: 0,
            matches_found: 0,
            estimated_remaining: None,
            speed_bps: 0,
        }
    }

    /// Returns the progress percentage (0.0 - 100.0)
    pub fn percentage(&self) -> f64 {
        if self.total_bytes == 0 {
            return 100.0;
        }
        (self.scanned_bytes as f64 / self.total_bytes as f64) * 100.0
    }

    /// Updates the progress
    pub fn update(&mut self, scanned_bytes: u64, matches_found: usize, speed_bps: u64) {
        self.scanned_bytes = scanned_bytes;
        self.matches_found = matches_found;
        self.speed_bps = speed_bps;

        if speed_bps > 0 {
            let remaining_bytes = self.total_bytes.saturating_sub(scanned_bytes);
            let remaining_secs = remaining_bytes / speed_bps;
            self.estimated_remaining = Some(Duration::from_secs(remaining_secs));
        }
    }
}

/// Result of a complete device scan
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Device or file that was scanned
    source_path: String,
    /// Total bytes scanned
    total_bytes: u64,
    /// Duration of the scan
    duration: Duration,
    /// All signature matches found
    matches: Vec<SignatureMatch>,
    /// Count of matches by file type
    type_counts: HashMap<FileType, usize>,
    /// Any errors encountered during scan
    errors: Vec<String>,
}

impl ScanResult {
    /// Creates a new scan result
    pub fn new(source_path: String, total_bytes: u64, duration: Duration) -> Self {
        Self {
            source_path,
            total_bytes,
            duration,
            matches: Vec::new(),
            type_counts: HashMap::new(),
            errors: Vec::new(),
        }
    }

    /// Adds a match to the result
    pub fn add_match(&mut self, match_: SignatureMatch) {
        let file_type = match_.file_type();
        *self.type_counts.entry(file_type).or_insert(0) += 1;
        self.matches.push(match_);
    }

    /// Adds an error message
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    /// Returns the source path
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    /// Returns total bytes scanned
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Returns the scan duration
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns all matches
    pub fn matches(&self) -> &[SignatureMatch] {
        &self.matches
    }

    /// Returns matches as a mutable slice
    pub fn matches_mut(&mut self) -> &mut Vec<SignatureMatch> {
        &mut self.matches
    }

    /// Consumes self and returns the matches
    pub fn into_matches(self) -> Vec<SignatureMatch> {
        self.matches
    }

    /// Returns the count for a specific file type
    pub fn count_for_type(&self, file_type: FileType) -> usize {
        self.type_counts.get(&file_type).copied().unwrap_or(0)
    }

    /// Returns the type counts map
    pub fn type_counts(&self) -> &HashMap<FileType, usize> {
        &self.type_counts
    }

    /// Returns total number of matches
    pub fn total_matches(&self) -> usize {
        self.matches.len()
    }

    /// Returns any errors encountered
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Returns whether the scan had errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns a summary string
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "Scanned {} ({} bytes) in {:.2}s\n",
            self.source_path,
            self.total_bytes,
            self.duration.as_secs_f64()
        );
        summary.push_str(&format!(
            "Found {} potential files:\n",
            self.total_matches()
        ));

        for (file_type, count) in &self.type_counts {
            summary.push_str(&format!("  - {}: {}\n", file_type, count));
        }

        if !self.errors.is_empty() {
            summary.push_str(&format!("\nEncountered {} errors\n", self.errors.len()));
        }

        summary
    }
}
