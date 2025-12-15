//! Scan device use case
//!
//! Orchestrates the scanning of a device for recoverable files.
//! Supports both sequential and parallel scanning modes.

use crate::application::dto::ScanOptions;
use crate::domain::entities::{ScanProgress, ScanResult, SignatureMatch};
use crate::domain::repositories::BlockDeviceReader;
use crate::domain::services::SignatureRegistry;
use anyhow::Result;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Progress callback type
pub type ProgressCallback = Box<dyn Fn(&ScanProgress) + Send + Sync>;

/// Scan device use case
///
/// Scans a block device or image file for recoverable files
/// by searching for known file signatures.
pub struct ScanDeviceUseCase {
    signature_registry: Arc<SignatureRegistry>,
}

impl ScanDeviceUseCase {
    /// Creates a new scan device use case
    pub fn new(signature_registry: Arc<SignatureRegistry>) -> Self {
        Self { signature_registry }
    }

    /// Creates with default image signatures
    pub fn with_default_signatures() -> Self {
        Self::new(Arc::new(SignatureRegistry::default_images()))
    }

    /// Executes the scan (sequential mode)
    pub fn execute<R: BlockDeviceReader>(
        &self,
        device: &R,
        options: &ScanOptions,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<ScanResult> {
        let start_time = Instant::now();
        let device_size = device.size();

        log::info!("Starting scan of {} ({} bytes)", device.path(), device_size);

        let mut progress = ScanProgress::new(device_size);
        let mut matches: Vec<SignatureMatch> = Vec::new();
        let mut bytes_scanned: u64 = 0;
        let chunk_size = options.chunk_size;

        // Read and scan chunks
        device.read_chunks(0, chunk_size, |offset, data| {
            // Search for signatures in this chunk
            let chunk_matches = self.scan_chunk(data, offset);

            for m in chunk_matches {
                let file_type = m.file_type();
                if options.file_types.is_empty() || options.file_types.contains(&file_type) {
                    matches.push(m);
                }
            }

            // Update progress
            bytes_scanned = offset + data.len() as u64;
            let elapsed = start_time.elapsed().as_secs().max(1);
            let speed = bytes_scanned / elapsed;

            progress.update(bytes_scanned, matches.len(), speed);

            // Call progress callback if provided
            if let Some(ref callback) = progress_callback {
                callback(&progress);
            }

            true // Continue scanning
        })?;

        let duration = start_time.elapsed();

        // Build result
        let mut result = ScanResult::new(device.path().to_string(), bytes_scanned, duration);

        for m in matches {
            result.add_match(m);
        }

        log::info!(
            "Scan complete: found {} potential files in {:.2}s",
            result.total_matches(),
            duration.as_secs_f64()
        );

        Ok(result)
    }

    /// Executes the scan using parallel processing
    ///
    /// This method divides the data into chunks and processes them in parallel
    /// using rayon. Best used with MmapBlockDevice for zero-copy access.
    ///
    /// # Arguments
    ///
    /// * `data` - The raw bytes to scan (e.g., from memory-mapped file)
    /// * `device_path` - Path to the device (for result reporting)
    /// * `options` - Scan options including chunk size
    /// * `progress_callback` - Optional callback for progress updates
    pub fn execute_parallel(
        &self,
        data: &[u8],
        device_path: &str,
        options: &ScanOptions,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<ScanResult> {
        let start_time = Instant::now();
        let data_size = data.len() as u64;
        let chunk_size = options.chunk_size;

        log::info!(
            "Starting parallel scan of {} ({} bytes) with {} chunks",
            device_path,
            data_size,
            (data_size as usize + chunk_size - 1) / chunk_size
        );

        // Atomic counters for thread-safe progress updates
        let bytes_scanned = AtomicU64::new(0);
        let matches_found = AtomicUsize::new(0);

        // Create chunks with their offsets
        let chunks: Vec<(u64, &[u8])> = data
            .chunks(chunk_size)
            .enumerate()
            .map(|(i, chunk)| ((i * chunk_size) as u64, chunk))
            .collect();

        // Process chunks in parallel and collect results
        let all_matches: Vec<SignatureMatch> = chunks
            .par_iter()
            .flat_map(|(offset, chunk)| {
                let chunk_matches = self.scan_chunk(chunk, *offset);

                // Update progress atomically
                bytes_scanned.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                matches_found.fetch_add(chunk_matches.len(), Ordering::Relaxed);

                // Filter by file types if specified
                chunk_matches
                    .into_iter()
                    .filter(|m| {
                        options.file_types.is_empty()
                            || options.file_types.contains(&m.file_type())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        // Call progress callback with final state
        if let Some(callback) = progress_callback {
            let elapsed = start_time.elapsed().as_secs().max(1);
            let speed = data_size / elapsed;
            let mut progress = ScanProgress::new(data_size);
            progress.update(data_size, all_matches.len(), speed);
            callback(&progress);
        }

        let duration = start_time.elapsed();

        // Build result
        let mut result = ScanResult::new(device_path.to_string(), data_size, duration);

        for m in all_matches {
            result.add_match(m);
        }

        log::info!(
            "Parallel scan complete: found {} potential files in {:.2}s",
            result.total_matches(),
            duration.as_secs_f64()
        );

        Ok(result)
    }

    /// Scans a single chunk for file signatures using Aho-Corasick
    ///
    /// This uses O(n + m + z) complexity where:
    /// - n = chunk size
    /// - m = total pattern length
    /// - z = number of matches
    fn scan_chunk(&self, data: &[u8], base_offset: u64) -> Vec<SignatureMatch> {
        let mut matches = Vec::new();

        // Use Aho-Corasick for efficient multi-pattern matching
        for (offset, sig) in self.signature_registry.find_all_matches_with_offsets(data) {
            let start_offset = base_offset + offset as u64;
            let remaining = &data[offset..];

            // Try to find the footer
            let end_offset = sig
                .find_footer(remaining)
                .map(|pos| start_offset + pos as u64);

            let estimated_size = end_offset
                .map(|e| e - start_offset)
                .unwrap_or(sig.max_size());

            // Skip if this looks like a false positive
            if estimated_size < 100 {
                continue; // Too small to be a real file
            }

            matches.push(SignatureMatch::new(
                sig.file_type(),
                start_offset,
                end_offset,
                estimated_size,
            ));
        }

        matches
    }
}

