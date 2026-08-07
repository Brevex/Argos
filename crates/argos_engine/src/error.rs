//! What can stop a scan.
//!
//! Corruption cannot: bad sectors, truncated structures and unknown
//! filesystems are recorded in the [`ScanReport`](crate::ScanReport) and the
//! scan continues. Only a failure that makes the *run* meaningless — the sink
//! refusing output — reaches this type.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;

use argos_core::geometry::ByteOffset;

/// A scan could not be completed.
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

    /// Whether the medium changed underneath the scan.
    #[must_use]
    pub fn is_unstable_medium(&self) -> bool {
        matches!(self.kind, ScanErrorKind::UnstableMedium { .. })
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
