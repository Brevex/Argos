//! How fast this window is being spoken to, and how long it takes to pass a
//! message on.
//!
//! Two numbers answer one question. `emit` is the time spent handing a
//! notification to the web view; the gap between notifications is the time the
//! engine took to produce one. If the first is small and the total still
//! outruns the second, the queue is in the web view rather than here — which
//! is the difference between a slow engine and a drowned interface.
//!
//! Off unless `ARGOS_TRACE` is set, and written to stderr. Counts and
//! durations only: no notification content is ever printed, so nothing here
//! can name a recovered file (A-NO-CONTENT-IN-LOGS).

use std::time::{Duration, Instant};

/// Environment variable that turns the trace on. The same one the engine
/// reads, so one setting instruments both halves of a run.
const ARGOS_TRACE: &str = "ARGOS_TRACE";

/// How often the running tally is printed while events flow.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// Whether tracing was asked for.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(ARGOS_TRACE).is_some_and(|value| !value.is_empty() && value != "0")
}

/// The reader thread's tally of what it delivered.
#[derive(Debug)]
pub struct Reader {
    on: bool,
    started: Instant,
    last_report: Instant,
    /// Events delivered since the tally started, and since the last report.
    total: u64,
    since_report: u64,
    /// One-second buckets, for the peak rate.
    bucket_started: Instant,
    bucket: u64,
    peak_per_second: u64,
    /// Time spent inside the call that hands an event to the web view.
    in_emit: Duration,
    slowest_emit: Duration,
}

impl Reader {
    /// Starts a tally, dormant unless tracing is on.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        let on = enabled();
        if on {
            eprintln!("argos-trace  shell           reading");
        }
        Self {
            on,
            started: now,
            last_report: now,
            total: 0,
            since_report: 0,
            bucket_started: now,
            bucket: 0,
            peak_per_second: 0,
            in_emit: Duration::ZERO,
            slowest_emit: Duration::ZERO,
        }
    }

    /// Records one notification handed to the web view, and how long that took.
    pub fn delivered(&mut self, took: Duration) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        self.total += 1;
        self.since_report += 1;
        self.in_emit += took;
        self.slowest_emit = self.slowest_emit.max(took);

        if now.duration_since(self.bucket_started) >= Duration::from_secs(1) {
            self.peak_per_second = self.peak_per_second.max(self.bucket);
            self.bucket_started = now;
            self.bucket = 0;
        }
        self.bucket += 1;

        if now.duration_since(self.last_report) >= REPORT_EVERY {
            let secs = now.duration_since(self.last_report).as_secs_f64();
            #[expect(
                clippy::cast_precision_loss,
                reason = "an event rate needs no more than f64 precision"
            )]
            let rate = self.since_report as f64 / secs.max(f64::MIN_POSITIVE);
            eprintln!(
                "argos-trace  shell           delivered={} rate={rate:.0}/s in_emit={:.3}s \
                 slowest={:.1}ms",
                self.total,
                self.in_emit.as_secs_f64(),
                self.slowest_emit.as_secs_f64() * 1000.0
            );
            self.last_report = now;
            self.since_report = 0;
        }
    }

    /// Prints the total. Called when the engine's output ends.
    pub fn summary(&self) {
        if !self.on {
            return;
        }
        let secs = self.started.elapsed().as_secs_f64();
        let peak = self.peak_per_second.max(self.bucket);
        eprintln!(
            "argos-trace  shell           ended   after={secs:.3}s delivered={} peak={peak}/s \
             in_emit={:.3}s slowest={:.1}ms",
            self.total,
            self.in_emit.as_secs_f64(),
            self.slowest_emit.as_secs_f64() * 1000.0
        );
    }
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}
