//! Scan device use case
//!
//! Orchestrates the scanning of a device for recoverable files.

use crate::application::dto::ScanOptions;
use crate::domain::entities::{ScanProgress, ScanResult, SignatureMatch};
use crate::domain::repositories::BlockDeviceReader;
use crate::domain::services::SignatureRegistry;
use anyhow::Result;
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

    /// Executes the scan
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
