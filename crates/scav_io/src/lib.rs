//! # Scav I/O
//!
//! I/O infrastructure implementations for the Scav forensic image recovery tool.
//!
//! This crate provides concrete implementations of the `BlockSource` trait
//! defined in `scav_core`, allowing the forensic scanner to read raw block data
//! from physical disks, disk images, and other block devices.
//!
//! ## Key Components
//!
//! - **DiskReader**: Read-only block source for physical disks and image files
//!
//! ## Example
//!
//! ```ignore
//! use scav_io::DiskReader;
//! use scav_core::BlockSource;
//!
//! let mut reader = DiskReader::new("/dev/sda")?;
//! let mut buffer = vec![0u8; 512];
//! let bytes_read = reader.read_chunk(0, &mut buffer)?;
//! ```

mod reader;

pub use reader::DiskReader;
