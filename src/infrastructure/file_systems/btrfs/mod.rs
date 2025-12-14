//! Btrfs filesystem parser
//!
//! Parses Btrfs filesystem metadata to find deleted files.
//! Btrfs uses a copy-on-write (COW) B-tree structure.

mod parser;

pub use parser::BtrfsParser;
