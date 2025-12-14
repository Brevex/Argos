//! Scan options DTO

use crate::domain::entities::FileType;

/// Options for scanning a device
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Path to the device or image file
    pub device_path: String,
    /// File types to search for (empty = all)
    pub file_types: Vec<FileType>,
    /// Chunk size for reading (affects memory usage)
    pub chunk_size: usize,
    /// Maximum file size to carve
    pub max_file_size: u64,
    /// Whether to use parallel scanning
    pub parallel: bool,
    /// Number of threads for parallel scanning
    pub thread_count: usize,
    /// Minimum confidence threshold (0.0 - 1.0)
    pub min_confidence: f32,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            device_path: String::new(),
            file_types: Vec::new(),           // All types
            chunk_size: 4 * 1024 * 1024,      // 4MB chunks
            max_file_size: 100 * 1024 * 1024, // 100MB max
            parallel: true,
            thread_count: num_cpus::get().max(1),
            min_confidence: 0.5,
        }
    }
}

impl ScanOptions {
    /// Creates new scan options for the given device
    pub fn new(device_path: &str) -> Self {
        Self {
            device_path: device_path.to_string(),
            ..Default::default()
        }
    }

    /// Sets the file types to search for
    pub fn with_types(mut self, types: Vec<FileType>) -> Self {
        self.file_types = types;
        self
    }

    /// Sets the chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Disables parallel scanning
    pub fn sequential(mut self) -> Self {
        self.parallel = false;
        self
    }
}

// Note: num_cpus is a common crate, but for simplicity we'll use a fallback
mod num_cpus {
    pub fn get() -> usize {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
    }
}
