//! Domain entities
//!
//! Core business objects that represent the fundamental concepts
//! in the file recovery domain.

mod file_signature;
mod recovered_file;
mod scan_result;

pub use file_signature::{FileSignature, FileType, SignatureMatch};
pub use recovered_file::RecoveredFile;
pub use scan_result::{ScanProgress, ScanResult};
