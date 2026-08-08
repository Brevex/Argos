//! Read-only [`BlockSource`](argos_core::source::BlockSource) adapters: the per-OS
//! device HAL, raw image files, and multi-pass acquisition.
//!
//! This is the only crate in the workspace allowed to contain `unsafe`, and the
//! `unsafe` is confined to the platform syscall layer. Every open is read-only at
//! the lowest layer; no write path to a source medium exists.

pub mod acquire;
pub mod class;
pub mod inventory;
pub mod naming;
pub mod shadow;

mod device;
mod image;
mod reader;

#[cfg(feature = "test-util")]
pub mod mock;

pub use class::TrimState;
pub use device::{Device, DeviceError};
pub use image::ImageSource;
pub use inventory::{DeviceInfo, MountPoint};
pub use reader::BlockReader;
pub use shadow::ShadowCopy;
