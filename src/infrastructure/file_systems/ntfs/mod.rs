//! NTFS filesystem parser
//!
//! Parses NTFS filesystem metadata to find deleted files.
//! NTFS uses a Master File Table (MFT) for file metadata.

mod parser;

pub use parser::NtfsParser;
