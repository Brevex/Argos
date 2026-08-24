//! Stdout: the console's user interface, and the only place printing happens.
//!
//! Everything that reaches a terminal is written here. The scan driver, the
//! engine and every library crate stay silent and hand back structured values
//! (`M-LOG-NOT-PRINT`); this module decides what a person sees.
//!
//! That includes what a run shows while it is still running: the live status
//! line a scan redraws, and the keys that pause and stop it.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argos_core::progress::{ProgressSink, RunState, ScanEvent, Unit};
use argos_engine::{ScanReport, ScanSession};
use argos_report::Manifest;

use crate::scan::Notice;

/// What a finished scan wrote, and whether it got there.
pub fn finished(finished: &crate::scan::Finished) {
    match &finished.report {
        Some(report) => summarize(report),
        None => println!("state     failed"),
    }
    println!("manifest  {}", finished.manifest.display());
}

/// What a resumed run has to go back to.
pub fn resuming(points: usize) {
    println!("resuming  {points} fragmentation points");
}

/// The frame the grafted strips are read in.
pub fn reference(dimensions: (u16, u16)) {
    let (width, height) = dimensions;
    println!("reference  {width}x{height}");
}

/// What a graft sweep entered and decoded, and what that is not.
pub fn grafted(entered: usize, written: usize) {
    println!("entered {entered} orphaned runs, {written} decoded to a picture");
    println!(
        "these are pixels in a header this tool supplied, not files the medium held: the frame \
         size is the reference's and each strip's position inside it is unknown"
    );
}

/// What an export copied, and what it refused.
pub fn exported(exported: &crate::results::Exported) {
    println!("exported  {} artifacts", exported.copied.len());
    if exported.previews > 0 {
        println!("previews  {} copied", exported.previews);
    }
    for name in &exported.missing {
        println!("missing   {name} is recorded in the manifest but not in the session directory");
    }
    for name in &exported.tampered {
        println!(
            "refused   {name} no longer reproduces the digest the scan recorded and was not \
             exported"
        );
    }
}

/// Prints what a scan says about itself as it says it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Console;

impl Notice for Console {
    fn opened(&self, description: &str, workers: usize) {
        println!("source    {description}");
        println!("workers   {workers}");
    }

    fn reduced_expectation(&self) {
        println!(
            "note      this medium reports solid-state storage with TRIM; deleted content is \
             often already gone from the host-visible surface before a scan begins"
        );
    }

    fn warning(&self, text: &str) {
        println!("warning   {text}");
    }
}

impl crate::acquire::Notice for Console {
    fn progress(&self, progress: argos_device::acquire::Progress) {
        use argos_device::acquire::Progress::{Refined, Swept};
        match progress {
            Swept { done, total } => {
                println!("sweep     {done} of {total} sectors");
            }
            Refined { done, total } => {
                println!("refine    {done} of {total} sectors the sweep could not read");
            }
        }
    }

    fn finished(&self, report: &argos_device::acquire::Report) {
        println!(
            "acquired  {} of {} sectors",
            report.recovered_sectors(),
            report.sector_count()
        );
        if report.is_complete() {
            println!("image     every sector was read");
            return;
        }
        // Named individually, because these are the ranges where the image
        // holds zeroes that were never on the medium. A scan of the image
        // reports them as unreadable, and nothing is ever recovered from them
        // (A-CONFIDENCE-HONEST).
        println!(
            "damage    {} unreadable runs are zero-filled placeholders in the image, never \
             read data",
            report.unreadable().len()
        );
        for range in report.unreadable() {
            println!("          {range}");
        }
    }
}

/// Prints the media this machine exposes, and the shadow copies it holds.
pub fn devices() {
    let devices = argos_device::inventory::list();
    if devices.is_empty() {
        println!(
            "no media found. Argos can still scan a raw image file, or a device path given \
             directly — enumeration needs no privileges, so an empty list means this platform \
             does not publish one rather than that access was refused"
        );
    }
    for device in &devices {
        let kind = match device.kind {
            argos_device::naming::NodeKind::WholeDisk => "disk",
            argos_device::naming::NodeKind::Partition => "partition",
        };
        print!("{:<24} {kind:<9}", device.path.display());
        match device.capacity_bytes {
            Some(bytes) => print!(" {bytes:>16} bytes"),
            None => print!(" {:>16}      ", "size unknown"),
        }
        print!("  {}", device.class);
        if device.trim != argos_device::TrimState::Unknown {
            print!(", trim {}", device.trim);
        }
        if let Some(model) = &device.model {
            print!("  {model}");
        }
        println!();
        for mount in &device.mounts {
            println!("    mounted  {mount}");
        }
    }

    let shadows = argos_device::shadow::list();
    if !shadows.is_empty() {
        println!();
        println!(
            "{} shadow copies. A file deleted before one of these was taken is present in it \
             whole, which is stronger evidence than anything carving can reconstruct:",
            shadows.len()
        );
        for shadow in &shadows {
            println!("  {}", shadow.path.display());
        }
    }
}

/// Prints what a finished scan found.
#[expect(
    clippy::too_many_lines,
    reason = "one branch per thing a scan can have to report; splitting it would \
              scatter the output format across functions without making any of it clearer"
)]
pub fn summarize(report: &ScanReport) {
    println!("state     {}", report.state);
    println!("scanned   {} bytes", report.bytes_swept);
    println!("recovered {} artifacts", report.artifacts);
    println!(
        "rejected  {} candidates that failed validation",
        report.rejected_candidates
    );
    if report.reassembly_attempted > 0 {
        println!(
            "reassembled {} images from {} fragmented candidates",
            report.reassembled, report.reassembly_attempted
        );
    }
    if report.ceilings.reassembly_decodes {
        println!(
            "budget    reassembly ran out of its decode budget; candidates were left \
             untried and the medium may hold more"
        );
    }
    if report.duplicates > 0 {
        println!(
            "duplicate {} artifacts collapsed by content hash",
            report.duplicates
        );
    }
    if report.previews_written > 0 || report.previews_failed > 0 {
        println!(
            "previews  {} rendered into {}/",
            report.previews_written,
            argos_report::PREVIEW_DIR
        );
        if report.previews_failed > 0 {
            println!(
                "          {} could not be written; the artifacts themselves are stored \
                 and recorded",
                report.previews_failed
            );
        }
    }
    if let Some(model) = report.triage_model {
        let photographs = report
            .triage
            .iter()
            .filter_map(|outcome| outcome.score)
            .filter(|score| score.label == argos_core::classify::TriageLabel::Photograph)
            .count();
        let near_duplicates = report
            .triage
            .iter()
            .filter(|outcome| outcome.near_duplicate_of.is_some())
            .count();
        println!(
            "triage    {photographs} of {} scored artifacts look like photographs \
             ({near_duplicates} near-duplicates), model {}",
            report.triage_scored, model.version
        );
        println!("          labels order the results; every artifact above is in the manifest");
        if report.triage_unscored > 0 {
            println!(
                "          {} artifacts could not be scored and are reported unlabelled",
                report.triage_unscored
            );
        }
        if report.triage_degraded {
            println!("          the classifier failed partway; artifacts after it are unlabelled");
        }
    }
    if !report.volumes.is_empty() {
        let residual = report
            .volumes
            .iter()
            .filter(|volume| volume.origin == argos_engine::Origin::Residual)
            .count();
        println!(
            "volumes   {} found ({residual} left by earlier formats)",
            report.volumes.len()
        );
    }
    if report.journal_deletions > 0 {
        println!(
            "journal   {} deletions read from change journals; artifacts sharing one moment \
             were removed in one action",
            report.journal_deletions
        );
    }
    if report.unattributed_residue > 0 {
        println!(
            "residue   {} orphaned metadata regions could not be tied to a volume, so \
             their extents were not resolved",
            report.unattributed_residue
        );
    }
    if !report.lost_files.is_empty() {
        let named = report
            .lost_files
            .iter()
            .filter(|lost| lost.name.is_some())
            .count();
        // Counted apart from the artifacts and said apart from them: not one
        // byte of these was read. What the medium still remembers is that the
        // files were there, and that is worth saying out loud — a run that
        // stayed quiet about them would describe a re-formatted disk as a disk
        // that never held anything (A-CONFIDENCE-HONEST).
        println!(
            "lost      {} deleted files are still named and dated by surviving metadata \
             ({named} with a name); no bytes of them were recovered — see `lost_files` \
             in the manifest",
            report.lost_files.len()
        );
    }
    if !report.unreadable.is_empty() {
        let bytes: u64 = report.unreadable.iter().map(|range| range.len).sum();
        println!(
            "damaged   {} regions ({bytes} bytes) could not be read; their contents are \
             unknown and nothing was recovered from them",
            report.unreadable.len()
        );
    }
    if report.ceilings.detection {
        println!(
            "capped    the surface held more signature or anchor matches than one scan \
             reports; results are incomplete"
        );
    }
    if report.unrecoverable > 0 {
        println!(
            "dropped   {} findings whose bytes could not be read back",
            report.unrecoverable
        );
    }
    if report.dropped_unreadable > 0 {
        println!(
            "lost      {} findings overlapped a damaged region and were dropped; the \
             signature that started each was real",
            report.dropped_unreadable
        );
    }
    if report.omitted_assets > 0 {
        println!(
            "omitted   {} artifacts were under the size floor and not written; each is in \
             the manifest with its dimensions and extents",
            report.omitted_assets
        );
    }
    if report.partial_prefixes > 0 {
        println!(
            "partial   {} images were reported as the part of themselves that decodes",
            report.partial_prefixes
        );
    }
    if report.reassembly_skipped_small > 0 {
        println!(
            "skipped   {} fragmented candidates declare a picture under the size floor and \
             were not searched; a run with a lower floor searches them",
            report.reassembly_skipped_small
        );
    }
}

/// Prints what a run reached and what it stopped short of.
///
/// These are the figures that separate a medium that held nothing more from a
/// run that did not look: what was recognised and deliberately not written,
/// what the search skipped, what damage cost, and which volumes were there to
/// read metadata from at all (`A-CONFIDENCE-HONEST`).
///
/// Printed before the artifact list, because a scan of a used disk has
/// hundreds of thousands of those and these numbers are what a reader needs
/// first.
fn coverage(manifest: &Manifest) {
    let Some(coverage) = &manifest.coverage else {
        return;
    };
    println!("swept     {} bytes", coverage.bytes_swept);
    if !manifest.volumes.is_empty() {
        let residual = manifest
            .volumes
            .iter()
            .filter(|volume| volume.origin == "residual")
            .count();
        println!(
            "volumes   {} located ({residual} left by earlier formats)",
            manifest.volumes.len()
        );
        // Counted by family first. A residue sweep of a re-formatted disk can
        // report thousands of anchors, and which families were found is the
        // fact that matters; the individual offsets are in the manifest.
        let mut families: Vec<(&str, usize)> = Vec::new();
        for volume in &manifest.volumes {
            match families.iter_mut().find(|(kind, _)| *kind == volume.kind) {
                Some((_, count)) => *count += 1,
                None => families.push((&volume.kind, 1)),
            }
        }
        families.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (kind, count) in families {
            println!("          {count} {kind}");
        }
        for volume in manifest.volumes.iter().take(LISTED_BY_DEFAULT) {
            println!(
                "          {:<6} {:<8} at {}, {} bytes, {} per allocation unit",
                volume.kind, volume.origin, volume.offset, volume.length, volume.allocation_bytes
            );
        }
        if manifest.volumes.len() > LISTED_BY_DEFAULT {
            println!(
                "          … and {} more anchors, all in the manifest",
                manifest.volumes.len() - LISTED_BY_DEFAULT
            );
        }
    }
    // Each of these is a place a recovery could have been and was not, and each
    // says which of the reasons it was.
    for (count, text) in [
        (
            coverage.journal_deletions,
            "deletions were read from change journals, naming and dating what was removed",
        ),
        (
            coverage.unattributed_residue,
            "orphaned metadata regions could not be tied to a volume, so their extents were \
             never resolved",
        ),
        (
            coverage.omitted_assets,
            "artifacts were under the size floor and not written; each is below with its \
             dimensions",
        ),
        (
            coverage.partial_prefixes,
            "images were reported as the part of themselves that decodes",
        ),
        (
            coverage.reassembly_skipped_small,
            "fragmented candidates declare a picture under the size floor and were not searched",
        ),
        (
            coverage.dropped_unreadable,
            "findings overlapped a damaged region and were dropped",
        ),
        (
            coverage.unrecoverable,
            "findings claimed bytes that could not be read back",
        ),
        (
            coverage.duplicates,
            "artifacts were collapsed by content hash",
        ),
    ] {
        if count > 0 {
            println!("          {count} {text}");
        }
    }
    if coverage.reassembly_attempted > 0 {
        println!(
            "reassembly {} recovered of {} fragmented candidates searched",
            coverage.reassembled, coverage.reassembly_attempted
        );
    }
    for ceiling in &coverage.ceilings {
        println!("ceiling   {ceiling} reached; the run looked at less than it set out to");
    }
}

/// Prints what a session directory holds, read back from its manifest.
///
/// The headless counterpart of a results view: it answers "what did that scan
/// recover?" from the record the scan left, without re-reading the medium.
/// Artifacts listed by default, before `--all` is needed.
///
/// A scan of a used disk records hundreds of thousands, and a list that long
/// is a list nobody reads. The head of it, strongest evidence first, is what
/// answers "did my photographs come back"; the manifest answers everything
/// else (`M-DOCUMENTED-MAGIC`).
const LISTED_BY_DEFAULT: usize = 40;

pub fn manifest(manifest: &Manifest, all: bool) {
    println!("source    {}", manifest.source);
    println!("state     {}", manifest.scan_state);
    println!("tool      {}", manifest.tool_version);
    // Recorded and written are different numbers, and saying so is the whole
    // point of recording an artifact that was not written: the manifest is a
    // complete account of the medium, the directory is what the caller asked
    // for (`A-CONFIDENCE-HONEST`).
    let written = manifest
        .artifacts
        .iter()
        .filter(|record| record.written)
        .count();
    println!(
        "recorded  {} artifacts, {written} of them written to this directory",
        manifest.artifacts.len()
    );
    println!(
        "rejected  {} candidates that failed validation",
        manifest.rejected_candidates
    );
    if !manifest.unreadable.is_empty() {
        let bytes: u64 = manifest.unreadable.iter().map(|range| range.length).sum();
        println!(
            "damaged   {} regions ({bytes} bytes) could not be read",
            manifest.unreadable.len()
        );
    }
    if let Some(triage) = &manifest.triage {
        match triage.status.as_str() {
            "scored" => println!(
                "triage    {} scored, {} unscored, model {}",
                triage.scored,
                triage.unscored,
                triage.model_version.as_deref().unwrap_or("unknown")
            ),
            _ => println!(
                "triage    disabled: {}",
                triage
                    .disabled_reason
                    .as_deref()
                    .unwrap_or("no reason given")
            ),
        }
    }
    coverage(manifest);

    // Strongest evidence first, then the largest picture, so what a person is
    // looking for is at the top of the list rather than somewhere in it. The
    // order is presentation; the manifest's own order is untouched.
    let listed = crate::results::ordered(manifest, None, true);

    let shown = if all {
        listed.len()
    } else {
        listed.len().min(LISTED_BY_DEFAULT)
    };
    for record in &listed[..shown] {
        print!(
            "  {:<12} {:>12} bytes  {:<16} at {}",
            record.name.as_deref().unwrap_or("(not written)"),
            record.length,
            record.confidence,
            record.source_offset
        );
        print!("  {}", crate::results::standing_of(record));
        if let (Some(width), Some(height)) = (record.width, record.height) {
            print!("  {width}x{height}");
        }
        if let Some(taken) = &record.taken {
            print!("  taken {taken}");
        }
        if let Some(deleted) = record.deleted_unix {
            print!("  deleted {deleted}");
        }
        if let Some(label) = &record.triage_label {
            print!("  {label}");
        }
        if record.preview.is_some() {
            print!("  +preview");
        }
        println!();
    }
    if shown < listed.len() {
        println!(
            "  … and {} more, weaker evidence first onwards; `--all` lists them, and the \
             manifest records every one either way",
            listed.len() - shown
        );
    }
}

/// Shortest interval between status-line redraws. Ten a second reads as live
/// without the scan spending its time on terminal writes.
const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

/// Bytes in a mebibyte, for the rate display.
const MIB: f64 = 1024.0 * 1024.0;

/// `seconds` as a clock a reader takes in at a glance.
///
/// `2h00m`, `52m10s`, `9s`: the largest two units that carry information, so a
/// budget and how much of it is gone can be compared without counting digits.
fn clock(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

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
    /// Artifacts stored so far, as the report stage last said.
    stored: u64,
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
                stored: 0,
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
                unit,
                done,
                total,
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
                // Only from a unit that supports one. Reassembly's steps cost
                // different amounts and the expensive ones are handed out
                // first, so a percentage of them reports a run doing its
                // heaviest work as barely started (`docs/defects/09`).
                let percent = unit
                    .supports_percentage()
                    .then(|| done.saturating_mul(100).checked_div(total))
                    .flatten();
                let measure = match unit {
                    // A read rate belongs to a stage that reads; a stage
                    // counting candidates reports the count itself.
                    Unit::Bytes => {
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "a rate display does not need more than f64 precision"
                        )]
                        let rate = done as f64 / MIB / elapsed;
                        format!("{rate:>7.1} MiB/s")
                    }
                    Unit::Items | Unit::Steps => format!("{done} of {total} {unit}"),
                    // A budget reads as a clock, not as a count of seconds.
                    Unit::Seconds => format!("{} of {}", clock(done), clock(total)),
                };
                match percent {
                    Some(percent) => eprint!("\r  {stage:<10} {percent:>3}%   {measure}"),
                    None => eprint!("\r  {stage:<10}         {measure}"),
                }
                let _ = std::io::stderr().flush();
                line.pending = true;
            }
            ScanEvent::ArtifactStored { artifacts, bytes } => {
                line.stored = artifacts;
                if !self.interactive {
                    return;
                }
                Renderer::break_line(&mut line);
                eprint!("\r  recovered  {artifacts} artifacts, {bytes} bytes");
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
            // Named as it starts, so a stage that turns out to have nothing to
            // report is still visibly the one running.
            ScanEvent::StageStarted { stage, .. } => {
                Renderer::break_line(&mut line);
                eprint!("\r  {stage:<10} started");
                let _ = std::io::stderr().flush();
                line.pending = true;
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

/// Watches stdin for `q` and raises `stop` when it arrives.
///
/// An acquisition of a large medium runs for hours and has no stages to pause
/// between, so it takes the one control that means something: stop. What was
/// copied before the stop stays copied, and the report says how much of the
/// medium was never reached.
///
/// The thread is detached for the same reason as [`spawn_console_controls`].
#[must_use]
pub fn spawn_stop_control(stop: Arc<AtomicBool>) -> Controls {
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
            if command.trim() == "q" {
                stop.store(true, Ordering::Release);
                return;
            }
        }
    });
    Controls { active }
}

/// Watches stdin for `p`, `r` and `q` and drives `session` accordingly.
///
/// `q` stops the search and lets the run write what it found; a second `q`,
/// while it is writing, stops that too. So the reader keeps listening after the
/// first one.
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
                "q" => session.cancel(),
                _ => {}
            }
        }
    });
    Controls { active }
}

#[cfg(test)]
mod tests {
    use super::clock;

    /// A budget and how much of it is gone are read side by side, so the two
    /// must never be shown in different units or in a form that has to be
    /// counted out. These are the boundaries where that could slip.
    #[test]
    fn a_clock_carries_the_largest_two_units_that_say_anything() {
        assert_eq!(clock(0), "0s");
        assert_eq!(clock(9), "9s");
        assert_eq!(clock(59), "59s");
        // A minute is the first thing worth naming, and the seconds beside it
        // stay two digits so 1m05s does not read as 1m50s.
        assert_eq!(clock(60), "1m00s");
        assert_eq!(clock(65), "1m05s");
        assert_eq!(clock(3599), "59m59s");
        // Past an hour the seconds stop earning their place and the minutes
        // take the same two digits for the same reason.
        assert_eq!(clock(3600), "1h00m");
        assert_eq!(clock(3661), "1h01m");
        assert_eq!(clock(7200), "2h00m");
        assert_eq!(clock(86_399), "23h59m");
    }
}
