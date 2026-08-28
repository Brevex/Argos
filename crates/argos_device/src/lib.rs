//! Read-only [`BlockSource`](argos_core::ports::BlockSource) adapters: the per-OS
//! device HAL, raw image files, and multi-pass acquisition.
//!
//! This is the only crate in the workspace allowed to contain `unsafe`, and the
//! `unsafe` is confined to the platform syscall layer. Every open is read-only at
//! the lowest layer; no write path to a source medium exists.

pub mod acquire;
pub mod class;
pub mod inventory;
pub mod mount;
pub mod naming;

mod device;

/// Everything this crate asks an operating system, one file per platform.
///
/// The decisions — which node is a whole disk, what a rotational flag means,
/// how a mount table is spelled — live in [`naming`], [`class`] and [`mount`],
/// compiled and tested on every target. Only the syscalls are behind `cfg`,
/// and they are here.
mod platform {
    #[cfg(target_os = "linux")]
    pub mod linux;
    #[cfg(target_os = "macos")]
    pub(crate) mod macos;
    #[cfg(windows)]
    pub(crate) mod windows;
}

pub use class::TrimState;
pub use device::{BlockReader, Device, DeviceError, ImageSource};
pub use inventory::{DeviceInfo, MountPoint, ShadowCopy};

#[cfg(feature = "test-util")]
pub use device::Ctrl;
