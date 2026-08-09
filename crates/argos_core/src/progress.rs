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

/// What a stage counts its work in.
///
/// A stage that reads the medium measures itself in bytes; one that examines
/// candidates or labels artifacts measures itself in those. Naming the unit is
/// what lets a display say `43%` for either without claiming a candidate count
/// is a byte count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Unit {
    /// Bytes of the medium.
    #[default]
    Bytes,
    /// Whatever the stage handles one at a time: a candidate to validate, a
    /// fragment set to reassemble, an artifact to label.
    Items,
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bytes => "bytes",
            Self::Items => "items",
        })
    }
}

/// One structured progress event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanEvent {
    /// A stage began.
    ///
    /// Emitted by every stage, including one that will never report progress,
    /// because a stage that cannot say how far along it is must at least be
    /// able to say that it is the one running. Silence between two stages is
    /// indistinguishable from a stall.
    StageStarted {
        /// Stage that began.
        stage: Stage,
        /// What `total` counts.
        unit: Unit,
        /// Work the stage expects to cover, zero when not known ahead.
        total: u64,
    },
    /// Cumulative progress within a stage, emitted once per chunk of work.
    StageProgress {
        /// Stage reporting progress.
        stage: Stage,
        /// What `done` and `total` count.
        unit: Unit,
        /// Work processed so far.
        done: u64,
        /// Work the stage expects to cover, zero when not known ahead.
        total: u64,
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
    /// An artifact reached the sink. Counts are cumulative for the run.
    ///
    /// Emitted once per artifact, which is the granularity a recovery happens
    /// at — not per sector and not per candidate (`M-LOG-OVERHEAD`). It exists
    /// so a display can say what has actually been recovered while it is
    /// happening, rather than only once the stage ends.
    ///
    /// Both figures describe artifacts **stored**, never candidates seen: a
    /// signature hit that has not passed its format's state machine is not a
    /// recovery, and counting one as such would overstate the result
    /// (A-CONFIDENCE-HONEST).
    ArtifactStored {
        /// Artifacts handed to the sink so far.
        artifacts: u64,
        /// Sum of their lengths in bytes.
        bytes: u64,
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
