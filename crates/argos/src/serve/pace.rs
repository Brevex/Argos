//! The rate at which a client is told things.
//!
//! The report stage emits two events per artifact, and artifacts are small: a
//! disk full of icons produces them faster than ten thousand a second. Passed
//! straight through, that is a JSON line, a pipe write, an inter-process
//! message, a parse and a re-render *each*, and it drowns whatever is
//! listening — a window stops drawing, its clock stops, and it is killed for
//! not responding. The engine is not slow when that happens; it is loud.
//!
//! So the same protection the console renderer has always had
//! ([`crate::console`] redraws at most ten times a second) is applied to the
//! wire. Two properties make it lossless:
//!
//! - Progress and stored-artifact counts are **cumulative**, so dropping an
//!   intermediate value loses nothing. The client that receives one in ten
//!   sees the same numbers arrive, later by at most one tick.
//! - Everything that is *not* cumulative — a stage beginning or ending, a
//!   state change — passes through immediately, and flushes whatever is
//!   pending first, so a stage never ends carrying a stale figure.
//!
//! The ticker also **repeats** the current stage's progress when nothing has
//! been sent for a while. That is what makes a silent stage impossible: a
//! client can always tell a working engine from a stopped one, including in a
//! stage that has no way to say how far along it is.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use argos_core::ports::ScanEvent;
use argos_ipc::dto;
use argos_ipc::wire::Notification;

use super::Wire;
use super::trace::Trace;
use super::translate;

/// Shortest interval between two cumulative notifications.
///
/// Ten a second is what the console renderer has always used: fast enough to
/// read as live, slow enough that no client can be outrun by it.
const TICK: Duration = Duration::from_millis(100);

/// How long the engine may go without saying anything before it repeats
/// itself.
///
/// A stage that cannot report progress still has to prove it is running. One
/// second is short enough that a person never wonders, and long enough that
/// the repetition is invisible when work is flowing.
const HEARTBEAT: Duration = Duration::from_secs(1);

/// What has happened since the last tick and has not been sent yet.
///
/// Kept free of I/O so the decision — what goes out, and when — can be tested
/// without a pipe on the other end (`M-MOCKABLE-SYSCALLS`).
#[derive(Debug, Default)]
struct Pending {
    /// Latest progress of the stage running, if it reported any.
    progress: Option<ScanEvent>,
    /// Latest cumulative count of artifacts stored.
    stored: Option<ScanEvent>,
    /// Regions the medium refused, as a running total. The extents themselves
    /// are in the manifest; a client needs to know how much was lost, not to
    /// receive one message per bad sector.
    unreadable_regions: u64,
    unreadable_bytes: u64,
    unreadable_dirty: bool,
    /// The last progress actually sent, for the heartbeat to repeat.
    last_progress: Option<ScanEvent>,
}

/// One thing the pacer decided to send.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Outgoing {
    /// A cumulative event, translated at send time.
    Event(ScanEvent),
    /// The running total of what the medium refused.
    Unreadable { regions: u64, bytes: u64 },
}

impl Pending {
    /// Takes everything waiting, in the order a client should see it.
    ///
    /// `quiet` asks for the heartbeat: when nothing is waiting and the wire
    /// has gone silent, the last progress is repeated so that a stage which
    /// cannot report anything is still distinguishable from a stopped engine.
    fn take(&mut self, quiet: bool) -> Vec<Outgoing> {
        let mut out = Vec::new();
        if let Some(progress) = self.progress.take() {
            self.last_progress = Some(progress);
            out.push(Outgoing::Event(progress));
        }
        if let Some(stored) = self.stored.take() {
            out.push(Outgoing::Event(stored));
        }
        if self.unreadable_dirty {
            self.unreadable_dirty = false;
            out.push(Outgoing::Unreadable {
                regions: self.unreadable_regions,
                bytes: self.unreadable_bytes,
            });
        }
        if out.is_empty()
            && quiet
            && let Some(progress) = self.last_progress
        {
            out.push(Outgoing::Event(progress));
        }
        out
    }
}

/// Sends notifications at a bounded rate.
#[derive(Debug)]
pub struct Pacer {
    out: Arc<Wire>,
    trace: Option<Arc<Trace>>,
    pending: Mutex<Pending>,
    /// When something was last written to the wire.
    last_sent: Mutex<Instant>,
    stop: Arc<(Mutex<bool>, Condvar)>,
    running: Arc<AtomicBool>,
}

impl Pacer {
    /// Starts pacing, with a ticker thread that flushes and keeps the beat.
    pub fn start(out: Arc<Wire>, trace: Option<Arc<Trace>>) -> Arc<Self> {
        let pacer = Arc::new(Self {
            out,
            trace,
            pending: Mutex::new(Pending::default()),
            last_sent: Mutex::new(Instant::now()),
            stop: Arc::new((Mutex::new(false), Condvar::new())),
            running: Arc::new(AtomicBool::new(true)),
        });

        let ticker = Arc::clone(&pacer);
        let stop = Arc::clone(&pacer.stop);
        let running = Arc::clone(&pacer.running);
        // Detached on purpose: it holds only an `Arc` to the pacer and exits
        // on the flag, so nothing waits on it and nothing leaks if a scan
        // thread unwinds.
        std::thread::spawn(move || {
            let (lock, cv) = &*stop;
            loop {
                let guard = lock
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let (guard, _) = cv
                    .wait_timeout(guard, TICK)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if *guard {
                    return;
                }
                drop(guard);
                if !running.load(Ordering::Acquire) {
                    return;
                }
                ticker.tick();
            }
        });
        pacer
    }

    /// Takes one event, sending it now or folding it into the next tick.
    pub fn emit(&self, event: ScanEvent) {
        if let Some(trace) = self.trace.as_ref() {
            trace.record(&event);
        }
        match event {
            // Cumulative: the newest value says everything the ones before it
            // said, so only the newest is kept.
            ScanEvent::StageProgress { .. } => {
                self.lock_pending().progress = Some(event);
            }
            ScanEvent::ArtifactStored { .. } => {
                self.lock_pending().stored = Some(event);
            }
            ScanEvent::RegionUnreadable { range } => {
                let mut pending = self.lock_pending();
                pending.unreadable_regions = pending.unreadable_regions.saturating_add(1);
                pending.unreadable_bytes = pending.unreadable_bytes.saturating_add(range.len);
                pending.unreadable_dirty = true;
            }
            // Not cumulative, and ordered against everything above: flush what
            // is pending, then send this.
            _ => {
                self.flush();
                if let Some(notification) = translate::event(event) {
                    self.send(&notification);
                }
            }
        }
    }

    /// Sends whatever is pending, right now.
    pub fn flush(&self) {
        self.dispatch(false);
    }

    /// One tick: flush, and repeat the current progress if the wire has been
    /// quiet for longer than the heartbeat.
    fn tick(&self) {
        let quiet = self.lock_last_sent().elapsed() >= HEARTBEAT;
        self.dispatch(quiet);
    }

    fn dispatch(&self, quiet: bool) {
        let outgoing = self.lock_pending().take(quiet);
        for item in outgoing {
            match item {
                Outgoing::Event(event) => {
                    if let Some(notification) = translate::event(event) {
                        self.send(&notification);
                    }
                }
                Outgoing::Unreadable { regions, bytes } => {
                    self.send(&Notification::Unreadable(dto::Unreadable {
                        regions,
                        bytes,
                    }));
                }
            }
        }
    }

    /// Stops the ticker and sends the last of what is pending.
    pub fn finish(&self) {
        self.running.store(false, Ordering::Release);
        let (lock, cv) = &*self.stop;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        cv.notify_all();
        self.flush();
    }

    fn send(&self, notification: &Notification) {
        self.out.send(notification);
        if let Some(trace) = self.trace.as_ref() {
            trace.sent();
        }
        *self.lock_last_sent() = Instant::now();
    }

    fn lock_pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_last_sent(&self) -> std::sync::MutexGuard<'_, Instant> {
        self.last_sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use argos_core::ports::Unit;
    use argos_core::{ByteOffset, ByteRange, Stage};

    use super::{Outgoing, Pending};

    fn progress(done: u64) -> super::ScanEvent {
        super::ScanEvent::StageProgress {
            stage: Stage::Report,
            unit: Unit::Bytes,
            done,
            total: 100,
        }
    }

    fn stored(artifacts: u64) -> super::ScanEvent {
        super::ScanEvent::ArtifactStored {
            artifacts,
            bytes: artifacts * 10,
        }
    }

    #[test]
    fn only_the_newest_of_a_cumulative_run_is_sent() {
        // Ten thousand progress events between two ticks must cost one
        // message, and it must be the last one: the figures are cumulative, so
        // the newest says everything the ones before it said.
        let mut pending = Pending::default();
        for done in 1..=10_000 {
            pending.progress = Some(progress(done));
            pending.stored = Some(stored(done));
        }

        let out = pending.take(false);

        assert_eq!(
            out,
            vec![
                Outgoing::Event(progress(10_000)),
                Outgoing::Event(stored(10_000)),
            ]
        );
        assert!(
            pending.take(false).is_empty(),
            "nothing may be sent twice, or a client would count an artifact twice"
        );
    }

    #[test]
    fn unreadable_regions_arrive_as_a_running_total() {
        // A failing medium produces these faster than any client can draw
        // them, and what a client needs is how much was lost. The extents
        // themselves are in the manifest.
        let mut pending = Pending::default();
        for start in 0..500_u64 {
            pending.unreadable_regions += 1;
            pending.unreadable_bytes += 512;
            pending.unreadable_dirty = true;
            let _ = ByteRange {
                start: ByteOffset::new(start * 512),
                len: 512,
            };
        }

        assert_eq!(
            pending.take(false),
            vec![Outgoing::Unreadable {
                regions: 500,
                bytes: 256_000,
            }]
        );
    }

    #[test]
    fn a_quiet_wire_repeats_the_last_progress_and_a_busy_one_does_not() {
        // This is what makes a silent stage impossible. A stage that cannot
        // say how far along it is still has to prove it is running, or a
        // person watching cannot tell it from a stopped engine.
        let mut pending = Pending {
            progress: Some(progress(42)),
            ..Pending::default()
        };
        assert_eq!(pending.take(false), vec![Outgoing::Event(progress(42))]);

        // Nothing new, and the wire has gone quiet: say it again.
        assert_eq!(pending.take(true), vec![Outgoing::Event(progress(42))]);
        // Nothing new, but something was sent recently: stay silent.
        assert!(pending.take(false).is_empty());
    }

    #[test]
    fn a_heartbeat_before_any_progress_says_nothing() {
        // Repeating a figure that was never reported would invent one.
        let mut pending = Pending::default();
        assert!(pending.take(true).is_empty());
    }
}
