//! The driving port: a scan you can start, watch, pause and stop.

use std::fmt;
use std::io::{Read, Seek};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use argos_core::artifact::ArtifactSink;
use argos_core::progress::{ProgressSink, RunState, ScanEvent};

use crate::config::{ConfigError, ScanConfig};
use crate::error::ScanError;
use crate::finding::ScanReport;
use crate::pipeline;

/// One or more independent read-only views of the same medium.
///
/// The pipeline reads sequentially through one view and validates candidates
/// through the others in parallel, so a scan wants about one view per worker.
/// Opening them is the caller's job — that is where the medium is known.
#[derive(Debug)]
pub struct Medium<V> {
    views: Vec<V>,
    len: u64,
}

impl<V: Read + Seek + Send> Medium<V> {
    /// A medium of `len` bytes read through `views`.
    ///
    /// Every view must address the same bytes at the same offsets; handing in
    /// views of different media produces extents that mean nothing.
    ///
    /// # Errors
    ///
    /// Fails when `views` is empty.
    pub fn new(views: Vec<V>, len: u64) -> Result<Self, ConfigError> {
        if views.is_empty() {
            return Err(ConfigError::no_views());
        }
        Ok(Self { views, len })
    }

    /// Length of the medium in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the medium holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of independent views available.
    #[must_use]
    pub fn views(&self) -> usize {
        self.views.len()
    }

    pub(crate) fn into_parts(self) -> (Vec<V>, u64) {
        (self.views, self.len)
    }
}

/// A scan run, shared between whoever drives it and whoever controls it.
///
/// Cloning is cheap and shares one run (`M-SERVICES-CLONE`): the thread that
/// calls [`ScanSession::start`] blocks until the scan ends, while any other
/// holder of a clone can [`pause`](ScanSession::pause),
/// [`resume`](ScanSession::resume) or [`cancel`](ScanSession::cancel) it.
///
/// Subscription to a running scan is the [`ProgressSink`] passed to `start`;
/// a CLI renderer, a UI event bridge and a test collector are all the same
/// thing to the engine.
#[derive(Clone, Debug)]
pub struct ScanSession {
    inner: Arc<Inner>,
}

struct Inner {
    config: ScanConfig,
    control: Control,
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inner")
            .field("config", &self.config)
            .field("state", &self.control.state())
            .finish()
    }
}

impl ScanSession {
    /// A session that will run `config` when started.
    #[must_use]
    pub fn new(config: ScanConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                control: Control::new(),
            }),
        }
    }

    /// The configuration this session runs.
    #[must_use]
    pub fn config(&self) -> &ScanConfig {
        &self.inner.config
    }

    /// The run's current lifecycle state.
    #[must_use]
    pub fn state(&self) -> RunState {
        self.inner.control.state()
    }

    /// Suspends the run at the next chunk boundary. A run that already ended
    /// stays ended.
    pub fn pause(&self) {
        self.inner.control.pause();
    }

    /// Resumes a paused run.
    pub fn resume(&self) {
        self.inner.control.resume();
    }

    /// Stops the run at the next chunk boundary, keeping everything recovered
    /// so far. Cancellation is final.
    pub fn cancel(&self) {
        self.inner.control.cancel();
    }

    /// Runs the scan to completion, to cancellation, or to a sink failure.
    ///
    /// Blocks the calling thread. Corruption never stops the scan: unreadable
    /// regions, invalid candidates and unparseable filesystems are recorded in
    /// the returned [`ScanReport`].
    ///
    /// # Errors
    ///
    /// Fails only when `sink` refuses an artifact.
    pub fn start<V, S, P>(
        &self,
        medium: Medium<V>,
        sink: &mut S,
        progress: &P,
    ) -> Result<ScanReport, ScanError>
    where
        V: Read + Seek + Send,
        S: ArtifactSink,
        P: ProgressSink + ?Sized,
    {
        self.inner.control.begin();
        progress.emit(ScanEvent::StateChanged {
            state: RunState::Running,
        });
        let outcome = pipeline::run(
            &self.inner.config,
            &self.inner.control,
            medium,
            sink,
            progress,
        );
        let state = if self.inner.control.is_cancelled() {
            RunState::Cancelled
        } else {
            RunState::Finished
        };
        self.inner.control.finish(state);
        progress.emit(ScanEvent::StateChanged { state });
        outcome.map(|mut report| {
            report.state = state;
            report
        })
    }
}

/// The pause/cancel flag every stage checks at chunk granularity.
///
/// A plain atomic answers "should I stop?" without contention on the hot path;
/// the mutex and condvar exist only so a *paused* run sleeps instead of
/// spinning (`M-THROUGHPUT`).
pub(crate) struct Control {
    state: AtomicU8,
    gate: Mutex<()>,
    resumed: Condvar,
}

/// [`RunState`] as stored in the atomic.
const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;
const FINISHED: u8 = 3;

impl Control {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(RUNNING),
            gate: Mutex::new(()),
            resumed: Condvar::new(),
        }
    }

    fn state(&self) -> RunState {
        match self.state.load(Ordering::Acquire) {
            PAUSED => RunState::Paused,
            CANCELLED => RunState::Cancelled,
            FINISHED => RunState::Finished,
            _ => RunState::Running,
        }
    }

    /// Puts a fresh or finished session back into the running state; a
    /// cancelled session stays cancelled.
    fn begin(&self) {
        let _ = self
            .state
            .compare_exchange(FINISHED, RUNNING, Ordering::AcqRel, Ordering::Acquire);
    }

    fn finish(&self, state: RunState) {
        if state == RunState::Finished {
            let _ =
                self.state
                    .compare_exchange(RUNNING, FINISHED, Ordering::AcqRel, Ordering::Acquire);
        }
        self.resumed.notify_all();
    }

    fn pause(&self) {
        let _ = self
            .state
            .compare_exchange(RUNNING, PAUSED, Ordering::AcqRel, Ordering::Acquire);
    }

    fn resume(&self) {
        if self
            .state
            .compare_exchange(PAUSED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.resumed.notify_all();
        }
    }

    fn cancel(&self) {
        self.state.store(CANCELLED, Ordering::Release);
        self.resumed.notify_all();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == CANCELLED
    }

    /// Blocks while the run is paused. Returns `false` when the run was
    /// cancelled and the caller should stop.
    pub(crate) fn wait_while_paused(&self) -> bool {
        if self.state.load(Ordering::Acquire) != PAUSED {
            return !self.is_cancelled();
        }
        let mut gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while self.state.load(Ordering::Acquire) == PAUSED {
            gate = self
                .resumed
                .wait(gate)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        !self.is_cancelled()
    }
}
