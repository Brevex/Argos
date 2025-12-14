//! Domain services
//!
//! Core business logic services that operate on domain entities.

mod carver;
mod signature_registry;

pub use carver::{CarverError, FileCarver};
pub use signature_registry::SignatureRegistry;
