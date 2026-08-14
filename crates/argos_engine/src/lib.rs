//! Scan pipeline orchestration: session lifecycle, staged concurrency, finding merge and confidence.
//!
//! This crate is the centre of the hexagon. It owns *how* a medium is scanned
//! and in what order the evidence is trusted; it owns nothing about where the
//! bytes come from, where the results go, or how progress is displayed. Those
//! arrive as ports: an `impl Read + Seek` view of the medium, an
//! [`ArtifactSink`](argos_core::artifact::ArtifactSink) and a
//! [`ProgressSink`](argos_core::progress::ProgressSink).
//!
//! ```no_run
//! # use std::io::Cursor;
//! # fn demo(views: Vec<Cursor<Vec<u8>>>, len: u64) -> Result<(), Box<dyn std::error::Error>> {
//! use argos_core::progress::Discard;
//! use argos_engine::{Medium, ScanConfig, ScanSession};
//!
//! let session = ScanSession::new(ScanConfig::default());
//! let medium = Medium::new(views, len)?;
//! let mut sink = argos_engine::fixture::Collector::new();
//! let report = session.start(medium, &mut sink, &Discard)?;
//! println!("{} artifacts", report.artifacts);
//! # Ok(())
//! # }
//! ```
//!
//! What the engine will not do: raise a finding's confidence tier, fabricate
//! bytes for a region the medium refused, or report a signature hit that did
//! not validate. Unreadable regions and rejected candidates are counted in the
//! [`ScanReport`] instead.

pub mod config;
pub mod finding;

mod annotate;
mod cache_run;
mod error;
mod merge;
mod pipeline;
mod search;
mod session;

#[cfg(feature = "test-util")]
pub mod fixture;

pub use annotate::TriageOutcome;
/// The fragmentation and filesystem vocabulary a [`ScanReport`] speaks,
/// re-exported so callers need no direct dependency on the crates that define
/// it (`M-FOREIGN-REEXPORTS`).
pub use argos_carve::reassemble::Broken;
pub use argos_fs::{FsKind, Origin, Volume};
pub use cache_run::CacheRun;
pub use config::{
    DEFAULT_MIN_LONG_SIDE, DEFAULT_REASSEMBLY_BUDGET, ScanConfig, ScanConfigBuilder, Stages,
};
pub use error::ScanError;
pub use finding::{Ceilings, Finding, ScanReport};
pub use session::{Medium, ScanSession};

/// Merges findings the way a scan does, for tests that need to drive the
/// merge rules directly rather than through a whole medium.
#[cfg(feature = "test-util")]
#[must_use]
pub fn merge_for_test(mut findings: Vec<Finding>) -> Vec<Finding> {
    merge::consolidate(&mut findings, &[]);
    findings
}
