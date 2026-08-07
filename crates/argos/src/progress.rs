//! The console face of a running scan: a live status line and keyboard control.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argos_core::progress::{ProgressSink, RunState, ScanEvent};
use argos_engine::ScanSession;

/// Shortest interval between status-line redraws. Ten a second reads as live
/// without the scan spending its time on terminal writes.
const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

/// Bytes in a mebibyte, for the rate display.
const MIB: f64 = 1024.0 * 1024.0;

/// Renders scan events to stderr, leaving stdout for the result.
///
/// On a terminal this is one status line rewritten in place; when stderr is
/// redirected it degrades to one line per stage transition, so a log file does
/// not fill with carriage returns.
#[derive(Debug)]
pub struct Renderer {
    line: Mutex<Line>,
    interactive: bool,
}

#[derive(Debug)]
struct Line {
    started: Instant,
    last_drawn: Option<Instant>,
    /// Whether an unterminated status line is on screen.
    pending: bool,
}

impl Renderer {
    /// A renderer bound to stderr.
    #[must_use]
    pub fn new() -> Self {
        Self {
            line: Mutex::new(Line {
                started: Instant::now(),
                last_drawn: None,
                pending: false,
            }),
            interactive: std::io::stderr().is_terminal(),
        }
    }

    /// Clears any status line still on screen.
    pub fn finish(&self) {
        let mut line = self.lock();
        if line.pending {
            eprintln!();
            line.pending = false;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Line> {
        self.line
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the status line so the next message starts on a clean row.
    fn break_line(line: &mut Line) {
        if line.pending {
            eprintln!();
            line.pending = false;
        }
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink for Renderer {
    fn emit(&self, event: ScanEvent) {
        let mut line = self.lock();
        match event {
            ScanEvent::StageProgress {
                stage,
                bytes_done,
                bytes_total,
            } => {
                if !self.interactive {
                    return;
                }
                let now = Instant::now();
                let due = line
                    .last_drawn
                    .is_none_or(|last| now.duration_since(last) >= REDRAW_INTERVAL);
                if !due {
                    return;
                }
                line.last_drawn = Some(now);
                let elapsed = now
                    .duration_since(line.started)
                    .as_secs_f64()
                    .max(f64::MIN_POSITIVE);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a rate display does not need more than f64 precision"
                )]
                let rate = bytes_done as f64 / MIB / elapsed;
                let percent = bytes_done.saturating_mul(100).checked_div(bytes_total);
                match percent {
                    Some(percent) => {
                        eprint!("\r  {stage:<10} {percent:>3}%   {rate:>7.1} MiB/s");
                    }
                    None => eprint!("\r  {stage:<10}         {rate:>7.1} MiB/s"),
                }
                let _ = std::io::stderr().flush();
                line.pending = true;
            }
            ScanEvent::StageFinished { stage, findings } => {
                Renderer::break_line(&mut line);
                eprintln!("  {stage:<10} done, {findings} findings");
            }
            ScanEvent::StateChanged { state } => {
                if matches!(state, RunState::Paused | RunState::Cancelled) {
                    Renderer::break_line(&mut line);
                    eprintln!("  {state}");
                }
            }
            ScanEvent::RegionUnreadable { range } => {
                Renderer::break_line(&mut line);
                eprintln!("  unreadable {range}");
            }
            // A stage announcing itself needs no line of its own; its progress
            // and its result already say so.
            ScanEvent::StageStarted { stage, .. } => {
                let _ = stage;
            }
            _ => {}
        }
    }
}

/// A background reader that turns console keys into session control.
#[derive(Debug)]
pub struct Controls {
    active: Arc<AtomicBool>,
}

impl Controls {
    /// Stops acting on further input.
    pub fn stop(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Watches stdin for `p`, `r` and `q` and drives `session` accordingly.
///
/// The thread is detached: it may be parked on a read from a console that
/// never sends anything, and the process must still be able to exit.
#[must_use]
pub fn spawn_console_controls(session: ScanSession) -> Controls {
    let active = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&active);
    std::thread::spawn(move || {
        let mut command = String::new();
        loop {
            command.clear();
            match std::io::stdin().read_line(&mut command) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if !flag.load(Ordering::Acquire) {
                return;
            }
            match command.trim() {
                "p" => session.pause(),
                "r" => session.resume(),
                "q" => {
                    session.cancel();
                    return;
                }
                _ => {}
            }
        }
    });
    Controls { active }
}
