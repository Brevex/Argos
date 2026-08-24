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

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;

use argos_core::geometry::ByteOffset;

pub mod config;
pub mod finding;
pub mod graft;

mod annotate;
mod pipeline;
mod session;

#[cfg(feature = "test-util")]
pub mod fixture;

pub use annotate::TriageOutcome;
/// The fragmentation and filesystem vocabulary a [`ScanReport`] speaks,
/// re-exported so callers need no direct dependency on the crates that define
/// it (`M-FOREIGN-REEXPORTS`).
pub use argos_carve::reassemble::Broken;
pub use argos_fs::{FsKind, Origin, Volume};
pub use config::{
    DEFAULT_MIN_LONG_SIDE, DEFAULT_REASSEMBLY_BUDGET, ScanConfig, ScanConfigBuilder, Stages,
};
pub use finding::{CacheRun, Ceilings, Finding, ScanReport};
pub use session::{Medium, ScanSession};

/// Merges findings the way a scan does, for tests that need to drive the
/// merge rules directly rather than through a whole medium.
#[cfg(feature = "test-util")]
#[must_use]
pub fn merge_for_test(mut findings: Vec<Finding>) -> Vec<Finding> {
    finding::consolidate(&mut findings, &[]);
    findings
}

/// A scan could not be completed.
///
/// Corruption never reaches this type: bad sectors, truncated structures and
/// unknown filesystems are recorded in the [`ScanReport`] and the scan
/// continues. Only a failure that makes the *run* meaningless — the sink
/// refusing output — stops a scan.
#[derive(Debug)]
pub struct ScanError {
    kind: ScanErrorKind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum ScanErrorKind {
    Sink(Box<dyn Error + Send + Sync>),
    UnstableMedium { at: ByteOffset },
}

impl ScanError {
    /// The artifact sink refused an artifact.
    pub(crate) fn sink(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: ScanErrorKind::Sink(Box::new(source)),
            backtrace: Backtrace::capture(),
        }
    }

    /// The medium returned different bytes for the same range within one run.
    pub(crate) fn unstable_medium(at: ByteOffset) -> Self {
        Self {
            kind: ScanErrorKind::UnstableMedium { at },
            backtrace: Backtrace::capture(),
        }
    }

    /// Whether the artifact sink was what failed.
    #[must_use]
    pub fn is_sink(&self) -> bool {
        matches!(self.kind, ScanErrorKind::Sink(_))
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ScanErrorKind::Sink(source) => write!(f, "cannot store a recovered artifact: {source}"),
            ScanErrorKind::UnstableMedium { at } => write!(
                f,
                "the medium returned different bytes for the range at byte {at} within one \
                 scan; results from it cannot be trusted"
            ),
        }?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for ScanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ScanErrorKind::Sink(source) => Some(source.as_ref()),
            ScanErrorKind::UnstableMedium { .. } => None,
        }
    }
}
