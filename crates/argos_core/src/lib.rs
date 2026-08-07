//! Domain vocabulary of Argos: storage newtypes, port traits and canonical errors.
//!
//! Everything here is sans-IO. The ports are the edges of the hexagon: every other
//! crate either implements one or calls through one.
//!
//! | Port | Direction | Adapters |
//! | --- | --- | --- |
//! | [`source::BlockSource`] | read the medium | device HAL, image file, [`fixture`] |
//! | [`artifact::ArtifactSink`] | deliver results | output directory, test collector |
//! | [`progress::ProgressSink`] | report progress | CLI renderer, UI bridge, [`progress::Discard`] |
//!
//! Where `std` already provides the abstraction, the `std` trait *is* the port:
//! parsers and carvers consume `impl Read + Seek`. [`source::BlockSource`] exists only
//! for what that cannot express — sector addressing, bad sectors and geometry.

pub mod artifact;
pub mod geometry;
pub mod progress;
pub mod source;

#[cfg(feature = "test-util")]
pub mod fixture;

mod recovery;

pub use recovery::{Confidence, Format, Stage, Timestamps};
