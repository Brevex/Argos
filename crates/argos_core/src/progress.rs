//! The port through which a running scan reports what it is doing.
//!
//! Events are **structured**: named variants with named fields, never
//! pre-formatted strings, so a renderer decides how (and whether) to show them
//! (`M-LOG-STRUCTURED`). They are also **batched** — the pipeline emits one
//! event per chunk or per stage, never one per sector or per candidate, so a
//! sink can never become the bottleneck (`M-LOG-OVERHEAD`).
//!
//! No event carries recovered content, a recovered filename or an identifying
//! path (A-NO-CONTENT-IN-LOGS); offsets, sizes and counts only.

use crate::geometry::ByteRange;
use crate::recovery::Stage;

/// Lifecycle state of a scan run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RunState {
    /// Work is progressing.
    #[default]
    Running,
    /// Work is suspended and will resume where it stopped.
    Paused,
    /// Work was stopped early; results so far are still reported.
    Cancelled,
    /// The run reached the end of its work.
    Finished,
}

impl RunState {
    /// Whether no further work will happen in this state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Finished)
    }
}

impl std::fmt::Display for RunState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Finished => "finished",
        };
        f.write_str(name)
    }
}

/// One structured progress event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanEvent {
    /// A stage began; `bytes_total` is the work it expects to cover.
    StageStarted {
        /// Stage that began.
        stage: Stage,
        /// Bytes the stage expects to process, zero when not known ahead.
        bytes_total: u64,
    },
    /// Cumulative progress within a stage, emitted once per chunk of work.
    StageProgress {
        /// Stage reporting progress.
        stage: Stage,
        /// Bytes processed so far.
        bytes_done: u64,
        /// Bytes the stage expects to process, zero when not known ahead.
        bytes_total: u64,
    },
    /// A stage ended, having produced `findings` results.
    StageFinished {
        /// Stage that ended.
        stage: Stage,
        /// Findings the stage contributed.
        findings: u64,
    },
    /// The run changed lifecycle state.
    StateChanged {
        /// State entered.
        state: RunState,
    },
    /// A region of the medium could not be read and is reported unrecovered,
    /// never fabricated (A-CONFIDENCE-HONEST).
    RegionUnreadable {
        /// Byte range that failed to read.
        range: ByteRange,
    },
}

/// Receives [`ScanEvent`]s from a running scan.
///
/// Sinks are shared across the pipeline's threads, so `emit` takes `&self` and
/// implementations must be cheap: a slow sink stalls the scan.
pub trait ProgressSink: Send + Sync {
    /// Handles one event.
    fn emit(&self, event: ScanEvent);
}

/// A [`ProgressSink`] that drops every event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Discard;

impl ProgressSink for Discard {
    fn emit(&self, _event: ScanEvent) {}
}

impl<T: ProgressSink + ?Sized> ProgressSink for &T {
    fn emit(&self, event: ScanEvent) {
        (**self).emit(event);
    }
}
