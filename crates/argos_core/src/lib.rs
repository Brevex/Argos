//! Domain vocabulary of Argos: storage newtypes, port traits and canonical errors.
//!
//! Everything here is sans-IO. The [`source::BlockSource`] trait is the read-only port
//! through which every other crate sees a medium under analysis; adapters for real
//! devices live in `argos_device`, and an in-memory fixture lives in [`fixture`]
//! (behind the `test-util` feature).

pub mod geometry;
pub mod source;

#[cfg(feature = "test-util")]
pub mod fixture;

mod confidence;

pub use confidence::Confidence;
