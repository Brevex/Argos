//! ext4 filesystem parser
//!
//! Parses ext4 filesystem metadata to find deleted files.
//! This module provides enhanced recovery by using inode information.

mod parser;

pub use parser::Ext4Parser;
