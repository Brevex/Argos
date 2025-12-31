//! # Argos Core
//!
//! Core domain definitions and traits for the Argos forensic image recovery tool.
//!
//! This crate provides the foundational abstractions that enable decoupled,
//! testable implementations following the Ports & Adapters architecture pattern.
//!
//! ## Key Components
//!
//! - **Types**: Strongly-typed enums for file formats (`FileType`)
//! - **Error types**: Typed errors using `thiserror` for ergonomic error handling
//! - **BlockSource trait**: Abstraction for reading raw block data from any source
//! - **FileScanner trait**: Interface for detecting file signatures (headers/footers)
//!
//! ## Example
//!
//! ```ignore
//! use argos_core::{BlockSource, FileScanner, FileType, CoreError, Result};
//!
//! // Implement BlockSource for your data source
//! // Implement FileScanner for formats you want to recover
//! ```

mod error;
pub mod scanners;
mod traits;
mod types;

pub use error::{CoreError, Result};
pub use scanners::{JpegScanner, PngScanner};
pub use traits::{BlockSource, FileScanner};
pub use types::FileType;

/// Extracts image dimensions from raw bytes in memory.
///
/// # Note
///
/// This function operates **entirely in-memory** on the provided byte slice.
/// It does NOT perform any disk I/O, maintaining the hexagonal architecture
/// principle that the core domain should be free of infrastructure concerns.
///
/// # Arguments
///
/// * `data` - Raw bytes containing the image header (first few KB is usually sufficient)
///
/// # Returns
///
/// `Some((width, height))` if dimensions could be extracted, `None` otherwise.
pub fn get_image_dimensions(data: &[u8]) -> Option<(usize, usize)> {
    imagesize::blob_size(data)
        .ok()
        .map(|size| (size.width, size.height))
}
