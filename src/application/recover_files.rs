//! Recover files use case
//!
//! Orchestrates the recovery of files from scan matches.
//! Supports both sequential and parallel processing modes.

use crate::application::dto::RecoveryResult;
use crate::domain::entities::{RecoveredFile, ScanResult};
use crate::domain::repositories::{BlockDeviceReader, RecoveredFileWriter, WriteOptions};
use crate::domain::services::FileCarver;
use anyhow::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Progress callback for recovery
pub type RecoveryProgressCallback = Box<dyn Fn(usize, usize) + Send + Sync>;

/// Recover files use case
///
/// Takes scan results and recovers the actual file data,
/// optionally converting and saving to disk.
pub struct RecoverFilesUseCase<C: FileCarver, W: RecoveredFileWriter> {
    carver: C,
    writer: W,
}

impl<C: FileCarver, W: RecoveredFileWriter> RecoverFilesUseCase<C, W> {
    /// Creates a new recover files use case
    pub fn new(carver: C, writer: W) -> Self {
        Self { carver, writer }
    }

    /// Executes the recovery (sequential mode)
    pub fn execute<R: BlockDeviceReader>(
        &self,
        device: &R,
        scan_result: &ScanResult,
        write_options: &WriteOptions,
        progress_callback: Option<RecoveryProgressCallback>,
    ) -> Result<RecoveryResult> {
        let start_time = Instant::now();
        let matches = scan_result.matches();
        let total_matches = matches.len();

        log::info!("Starting recovery of {} potential files", total_matches);

        let mut result = RecoveryResult::new(
            scan_result.source_path().to_string(),
            self.writer.output_dir().to_path_buf(),
        );
        result.bytes_scanned = scan_result.total_bytes();

        let file_id = AtomicU64::new(1);

        for (index, match_info) in matches.iter().enumerate() {
            // Report progress
            if let Some(ref callback) = progress_callback {
                callback(index + 1, total_matches);
            }

            // Skip unsupported types
            if !self.carver.supports(match_info.file_type()) {
                continue;
            }

            // Read data for this file
            let read_size = match_info
                .actual_size()
                .unwrap_or(match_info.estimated_size())
                .min(100 * 1024 * 1024) as usize; // Cap at 100MB

            let data = match device.read_at(match_info.start_offset(), read_size) {
                Ok(d) => d,
                Err(e) => {
                    result.add_error(format!(
                        "Failed to read at offset {}: {}",
                        match_info.start_offset(),
                        e
                    ));
                    continue;
                }
            };

            // Carve the file
            let current_id = file_id.fetch_add(1, Ordering::Relaxed);
            let recovered = match self.carver.carve(&data, match_info, current_id) {
                Ok(f) => f,
                Err(e) => {
                    log::debug!(
                        "Failed to carve file at offset {}: {}",
                        match_info.start_offset(),
                        e
                    );
                    continue; // Skip failed carves silently
                }
            };

            // Write the file
            match self.writer.write(&recovered, write_options) {
                Ok(write_result) => {
                    log::debug!(
                        "Recovered {} to {}",
                        recovered.file_type(),
                        write_result.saved_path.display()
                    );
                    result.add_recovered(recovered.file_type(), write_result.saved_size);
                }
                Err(e) => {
                    result.add_error(format!("Failed to write file {}: {}", recovered.id(), e));
                }
            }
        }

        result.duration = start_time.elapsed();

        log::info!(
            "Recovery complete: {} files recovered in {:.2}s",
            result.files_recovered,
            result.duration.as_secs_f64()
        );

        Ok(result)
    }

    /// Executes recovery using parallel carving
    ///
    /// This method parallelizes the CPU-intensive carving operations.
    /// Reading and writing remain sequential (I/O bound).
    ///
    /// # Arguments
    ///
    /// * `data` - Raw device data (e.g., from memory-mapped file)
    /// * `scan_result` - Scan results containing file matches
    /// * `write_options` - Options for writing files
    /// * `progress_callback` - Optional callback for progress updates
    pub fn execute_parallel(
        &self,
        data: &[u8],
        scan_result: &ScanResult,
        write_options: &WriteOptions,
        progress_callback: Option<RecoveryProgressCallback>,
    ) -> Result<RecoveryResult> {
        let start_time = Instant::now();
        let matches = scan_result.matches();
        let total_matches = matches.len();

        log::info!(
            "Starting parallel recovery of {} potential files",
            total_matches
        );

        let mut result = RecoveryResult::new(
            scan_result.source_path().to_string(),
            self.writer.output_dir().to_path_buf(),
        );
        result.bytes_scanned = scan_result.total_bytes();

        let file_id = AtomicU64::new(1);

        // Parallel carving - collect (match_info, carved_data) pairs
        let carved_files: Vec<(RecoveredFile, String)> = matches
            .par_iter()
            .filter(|m| self.carver.supports(m.file_type()))
            .filter_map(|match_info| {
                // Extract data slice for this file
                let start = match_info.start_offset() as usize;
                let read_size = match_info
                    .actual_size()
                    .unwrap_or(match_info.estimated_size())
                    .min(100 * 1024 * 1024) as usize;
                let end = (start + read_size).min(data.len());
                
                if start >= data.len() {
                    return Some(Err(format!(
                        "Offset {} beyond data length {}",
                        start,
                        data.len()
                    )));
                }

                let file_data = &data[start..end];
                let current_id = file_id.fetch_add(1, Ordering::Relaxed);

                // Carve the file (CPU intensive, runs in parallel)
                match self.carver.carve(file_data, match_info, current_id) {
                    Ok(recovered) => Some(Ok((recovered, String::new()))),
                    Err(e) => {
                        log::debug!(
                            "Failed to carve file at offset {}: {}",
                            match_info.start_offset(),
                            e
                        );
                        None
                    }
                }
            })
            .filter_map(|r| r.ok())
            .collect();

        // Sequential writing (I/O bound)
        for (index, (recovered, _)) in carved_files.iter().enumerate() {
            if let Some(ref callback) = progress_callback {
                callback(index + 1, carved_files.len());
            }

            match self.writer.write(recovered, write_options) {
                Ok(write_result) => {
                    log::debug!(
                        "Recovered {} to {}",
                        recovered.file_type(),
                        write_result.saved_path.display()
                    );
                    result.add_recovered(recovered.file_type(), write_result.saved_size);
                }
                Err(e) => {
                    result.add_error(format!("Failed to write file {}: {}", recovered.id(), e));
                }
            }
        }

        result.duration = start_time.elapsed();

        log::info!(
            "Parallel recovery complete: {} files recovered in {:.2}s",
            result.files_recovered,
            result.duration.as_secs_f64()
        );

        Ok(result)
    }
}

