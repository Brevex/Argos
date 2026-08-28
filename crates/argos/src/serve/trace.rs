//! Where a serve session spends its time, and how fast it talks.
//!
//! A scan that goes quiet is indistinguishable from a scan that has hung, and
//! a scan that talks too fast can drown the client it is talking to. Neither
//! is visible from the outside, so this records both: how long each stage ran,
//! how many events it emitted, and — the number that matters most — the
//! longest silence between two events.
//!
//! Off unless [`ARGOS_TRACE`] is set, and written to **stderr**. In serve mode
//! stdout is the protocol, so a diagnostic on it would corrupt the stream.
//!
//! Stage names, counts and durations only: nothing here can name a recovered
//! file, a path or a byte of content (A-NO-CONTENT-IN-LOGS).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use argos_core::Stage;
use argos_core::ports::{ScanEvent, Unit};

/// Environment variable that turns the trace on.
const ARGOS_TRACE: &str = "ARGOS_TRACE";

/// Whether tracing was asked for.
///
/// Any value other than empty or `0` enables it, so `ARGOS_TRACE=1` and
/// `ARGOS_TRACE=yes` both work and `ARGOS_TRACE=0` does not.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(ARGOS_TRACE).is_some_and(|value| !value.is_empty() && value != "0")
}

/// One stage's running tally.
#[derive(Debug)]
struct StageRun {
    stage: Stage,
    started: Instant,
    events: u64,
    unit: Unit,
    total: u64,
    done: u64,
}

#[derive(Debug)]
struct State {
    started: Instant,
    /// The stage currently running, if one has announced itself.
    stage: Option<StageRun>,
    /// When the current one-second rate bucket opened, and its count.
    bucket_started: Instant,
    bucket_events: u64,
    /// Highest one-second event count seen.
    peak_per_second: u64,
    events: u64,
    /// Notifications that actually reached the wire, and their peak rate. The
    /// gap between this and `events` is what the pacer absorbed.
    sent: u64,
    sent_bucket_started: Instant,
    sent_bucket: u64,
    sent_peak_per_second: u64,
    /// When the last event was emitted, for the silence measurement.
    last_event: Instant,
    /// Longest silence between two events, over the whole session.
    longest_gap: Duration,
    /// The stage that silence fell in.
    longest_gap_stage: Option<Stage>,
}

/// Records what a session did, one event at a time.
#[derive(Debug)]
pub struct Trace {
    state: Mutex<State>,
}

impl Trace {
    /// Starts a trace and says so.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        eprintln!("argos-trace  session         began");
        Self {
            state: Mutex::new(State {
                started: now,
                stage: None,
                bucket_started: now,
                bucket_events: 0,
                peak_per_second: 0,
                events: 0,
                sent: 0,
                sent_bucket_started: now,
                sent_bucket: 0,
                sent_peak_per_second: 0,
                last_event: now,
                longest_gap: Duration::ZERO,
                longest_gap_stage: None,
            }),
        }
    }

    /// Folds one event into the tally, printing at stage boundaries.
    pub fn record(&self, event: &ScanEvent) {
        let now = Instant::now();
        let mut state = self.lock();

        let gap = now.duration_since(state.last_event);
        if gap > state.longest_gap {
            state.longest_gap = gap;
            state.longest_gap_stage = state.stage.as_ref().map(|run| run.stage);
        }
        state.last_event = now;
        state.events += 1;

        // One-second rate buckets. The peak is what says whether a client was
        // ever asked to keep up with more than it can.
        if now.duration_since(state.bucket_started) >= Duration::from_secs(1) {
            state.peak_per_second = state.peak_per_second.max(state.bucket_events);
            state.bucket_started = now;
            state.bucket_events = 0;
        }
        state.bucket_events += 1;
        if let Some(run) = state.stage.as_mut() {
            run.events += 1;
        }

        match *event {
            ScanEvent::StageStarted { stage, unit, total } => {
                eprintln!(
                    "argos-trace  {:<15} began   unit={unit} total={total}",
                    stage.to_string()
                );
                state.stage = Some(StageRun {
                    stage,
                    started: now,
                    events: 1,
                    unit,
                    total,
                    done: 0,
                });
            }
            ScanEvent::StageProgress { done, .. } => {
                if let Some(run) = state.stage.as_mut() {
                    run.done = done;
                }
            }
            ScanEvent::StageFinished { stage, findings } => {
                let Some(run) = state.stage.take() else {
                    return;
                };
                let secs = run.started.elapsed().as_secs_f64();
                let rate = if secs > 0.0 {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "an event rate needs no more than f64 precision"
                    )]
                    let events = run.events as f64;
                    events / secs
                } else {
                    0.0
                };
                eprintln!(
                    "argos-trace  {:<15} ended   after={secs:.3}s events={} rate={rate:.0}/s \
                     {}={}/{} findings={findings}",
                    stage.to_string(),
                    run.events,
                    run.unit,
                    run.done,
                    run.total
                );
            }
            _ => {}
        }
    }

    /// Records one notification that actually reached the wire.
    pub fn sent(&self) {
        let now = Instant::now();
        let mut state = self.lock();
        state.sent += 1;
        if now.duration_since(state.sent_bucket_started) >= Duration::from_secs(1) {
            state.sent_peak_per_second = state.sent_peak_per_second.max(state.sent_bucket);
            state.sent_bucket_started = now;
            state.sent_bucket = 0;
        }
        state.sent_bucket += 1;
    }

    /// Prints the session total. Called once, when the connection ends.
    ///
    /// Two rates are printed on purpose. `events` is what the pipeline
    /// produced; `sent` is what a client was asked to keep up with. Their
    /// ratio is the whole point of the pacer.
    pub fn summary(&self) {
        let state = self.lock();
        let secs = state.started.elapsed().as_secs_f64();
        let peak = state.peak_per_second.max(state.bucket_events);
        let sent_peak = state.sent_peak_per_second.max(state.sent_bucket);
        let quiet = state
            .longest_gap_stage
            .map_or_else(|| "-".to_owned(), |stage| stage.to_string());
        eprintln!(
            "argos-trace  session         ended   after={secs:.3}s events={} peak={peak}/s \
             sent={} sent_peak={sent_peak}/s longest_silence={:.3}s in={quiet}",
            state.events,
            state.sent,
            state.longest_gap.as_secs_f64()
        );
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}
