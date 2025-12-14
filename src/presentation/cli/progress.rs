//! Progress reporting for CLI

use crate::domain::entities::ScanProgress;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;

/// Progress reporter using indicatif
pub struct ProgressReporter {
    bar: Arc<ProgressBar>,
}

impl ProgressReporter {
    /// Creates a new progress reporter
    pub fn new(total: u64, message: &str) -> Self {
        let bar = ProgressBar::new(total);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        bar.set_message(message.to_string());

        Self { bar: Arc::new(bar) }
    }

    /// Creates a progress reporter for scanning
    pub fn for_scan(device_size: u64) -> Self {
        Self::new(device_size, "Scanning device for recoverable files...")
    }

    /// Creates a progress reporter for recovery
    pub fn for_recovery(total_files: u64) -> Self {
        let bar = ProgressBar::new(total_files);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} files ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        bar.set_message("Recovering files...".to_string());

        Self { bar: Arc::new(bar) }
    }

    /// Updates the progress bar
    pub fn update(&self, position: u64) {
        self.bar.set_position(position);
    }

    /// Updates from scan progress
    pub fn update_from_scan(&self, progress: &ScanProgress) {
        self.bar.set_position(progress.scanned_bytes);
        self.bar.set_message(format!(
            "Found {} potential files | Speed: {} MB/s",
            progress.matches_found,
            progress.speed_bps / (1024 * 1024)
        ));
    }

    /// Finishes with a message
    pub fn finish(&self, message: &str) {
        self.bar.finish_with_message(message.to_string());
    }

    /// Gets a callback for scan progress
    pub fn scan_callback(&self) -> Box<dyn Fn(&ScanProgress) + Send + Sync> {
        let bar = Arc::clone(&self.bar);
        Box::new(move |progress: &ScanProgress| {
            bar.set_position(progress.scanned_bytes);
            bar.set_message(format!(
                "Found {} potential files | Speed: {} MB/s",
                progress.matches_found,
                progress.speed_bps / (1024 * 1024)
            ));
        })
    }

    /// Gets a callback for recovery progress
    pub fn recovery_callback(&self) -> Box<dyn Fn(usize, usize) + Send + Sync> {
        let bar = Arc::clone(&self.bar);
        Box::new(move |current: usize, total: usize| {
            bar.set_position(current as u64);
            bar.set_length(total as u64);
        })
    }
}
