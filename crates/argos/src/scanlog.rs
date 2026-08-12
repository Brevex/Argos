//! `scan.log`: what a run did, written beside what it recovered.
//!
//! A scan of a large medium runs for hours in stages a person cannot see into.
//! When one of them stops making progress there is nothing on screen to say
//! which, and a graphical launch has no console to fall back on — so the run
//! keeps its own account, in the session directory, in plain text.
//!
//! It records the shape of the work and nothing about its content: stages,
//! times, counts and ceilings. No recovered bytes, no recovered file name, no
//! path from the medium (`A-NO-CONTENT-IN-LOGS`).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argos_core::progress::{ProgressSink, ScanEvent};
use argos_report::Owner;

/// Shortest interval between two progress lines for one stage.
///
/// A stage emits progress two hundred times; at one line each a long run is
/// unreadable and a short one is noise. Five seconds is short enough that a
/// stalled stage is obvious from the gaps and long enough that the file stays
/// a page rather than a scroll.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

/// The log of one run.
pub struct ScanLog {
    out: Mutex<BufWriter<File>>,
    started: Instant,
    /// When the last progress line was written, so they can be spaced.
    last: Mutex<Option<Instant>>,
}

impl ScanLog {
    /// Creates `scan.log` in `dir`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be created.
    pub fn create(dir: &Path, owner: Option<Owner>) -> std::io::Result<Self> {
        let path = dir.join("scan.log");
        let file = File::create(&path)?;
        if let Some(owner) = owner {
            // The same handback the recovered files get: a log the person who
            // ran the scan cannot open is not a log.
            let _ = owner.give(&path);
        }
        let log = Self {
            out: Mutex::new(BufWriter::new(file)),
            started: Instant::now(),
            last: Mutex::new(None),
        };
        log.line("scan started");
        Ok(log)
    }

    /// Writes one line, stamped with how long the run has been going.
    pub fn line(&self, text: &str) {
        let elapsed = self.started.elapsed().as_secs_f64();
        let mut out = lock(&self.out);
        let _ = writeln!(out, "[{elapsed:9.3}s] {text}");
        // Flushed per line on purpose: the reason this file exists is to be
        // readable while the run is still going, and most of all after a run
        // that had to be killed.
        let _ = out.flush();
    }

    /// Records the final counts, including every ceiling the run hit.
    pub fn summary(&self, report: &argos_engine::ScanReport) {
        self.line(&format!("state          {}", report.state));
        self.line(&format!("swept          {} bytes", report.bytes_swept));
        self.line(&format!(
            "findings       {} artifacts, {} rejected, {} duplicates, {} unrecoverable",
            report.artifacts, report.rejected_candidates, report.duplicates, report.unrecoverable
        ));
        self.line(&format!(
            "reassembly     {} recovered of {} attempted",
            report.reassembled, report.reassembly_attempted
        ));
        self.line(&format!(
            "unreadable     {} regions",
            report.unreadable.len()
        ));
        for name in report.ceilings.reached() {
            self.line(&format!(
                "ceiling        {name} reached; the run looked at less than it set out to"
            ));
        }
        self.line("scan ended");
    }
}

impl std::fmt::Debug for ScanLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanLog").finish_non_exhaustive()
    }
}

/// A progress sink that writes to the log on the way to another sink.
///
/// The log has to see the same events the screen sees, and the screen must not
/// wait for the disk, so this forwards first-class and writes at a distance:
/// stage boundaries always, progress at most every few seconds.
#[derive(Debug)]
pub struct Tee<'a, P: ?Sized> {
    /// Where the events were going.
    pub inner: &'a P,
    /// Where they are also recorded.
    pub log: &'a ScanLog,
}

impl<P: ProgressSink + ?Sized> ProgressSink for Tee<'_, P> {
    fn emit(&self, event: ScanEvent) {
        self.record(event);
        self.inner.emit(event);
    }
}

impl<P: ?Sized> Tee<'_, P> {
    /// Decides whether `event` is worth a line, and writes it if so.
    fn record(&self, event: ScanEvent) {
        match event {
            ScanEvent::StageStarted { stage, unit, total } => {
                *lock(&self.log.last) = None;
                self.log.line(&format!("{stage:<10} began, {total} {unit}"));
            }
            ScanEvent::StageProgress {
                stage,
                unit,
                done,
                total,
            } => {
                let mut last = lock(&self.log.last);
                let now = Instant::now();
                if last.is_some_and(|at| now.duration_since(at) < PROGRESS_INTERVAL) {
                    return;
                }
                *last = Some(now);
                drop(last);
                self.log
                    .line(&format!("{stage:<10}  {done}/{total} {unit}"));
            }
            ScanEvent::StageFinished { stage, findings } => {
                *lock(&self.log.last) = None;
                self.log
                    .line(&format!("{stage:<10} ended, {findings} findings"));
            }
            ScanEvent::StateChanged { state } => self.log.line(&format!("state {state}")),
            // Everything else is per-artifact or per-region and belongs to the
            // counts in the summary, not to a line each: a failing disk would
            // otherwise write a log longer than the recovery.
            _ => {}
        }
    }
}

/// Locks a mutex, ignoring poisoning: a panic in one line must not silence the
/// log for the rest of the run.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
