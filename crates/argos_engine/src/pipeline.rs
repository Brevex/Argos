//! The staged scan: one sequential reader, a pool of workers, bounded queues.
//!
//! Stages run cheapest-and-most-trusted first and all feed one merged findings
//! set:
//!
//! 1. **Sweep** — one thread reads the medium end to end in chunks; a pool of
//!    workers matches image signatures and filesystem anchors in those chunks.
//!    The medium is read exactly once for both, because reading it is the
//!    expensive part.
//! 2. **Filesystem** — sequential, targeted reads over the volumes the sweep
//!    located: current and residual alike.
//! 3. **Validation** — every signature hit is driven through its format's
//!    state machine, in parallel, each worker on its own view of the medium.
//! 4. **Report** — findings are merged, hashed in medium order and handed to
//!    the sink. Sequential, so the manifest is identical across runs.
//!
//! The reader is a single thread on purpose: rotational throughput collapses
//! under seek storms, and the medium — not the CPU — is the bottleneck of
//! stage 1. Buffers travel between the reader and the workers through a pool
//! channel, so the queue both bounds memory and applies backpressure
//! (`M-MEM-REUSE`, `M-THROUGHPUT`).

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;

use argos_carve::reassemble::{self, Assembled, Broken};

mod carving;
mod filesystem;
mod output;
mod reassembly;

use argos_carve::{Candidate, Detector, Scratch, Verdict};
use argos_core::ports::{
    Artifact, ArtifactSink, Classifier, Digest, ProgressSink, ScanEvent, Unit,
};
use argos_core::{ByteOffset, ByteRange, Confidence, Format, Stage};
use argos_fs::{DeletedFile, FsKind, Volume, residue};
use carving::carve;
use filesystem::recover_filesystems;
use output::emit;
use reassembly::{Reassembling, partial_prefixes, reassemble_broken};
use sha2::{Digest as _, Sha256};

use crate::ScanError;
use crate::config::{CHUNK_OVERLAP_BYTES, ScanConfig};
use crate::finding;
use crate::finding::{Finding, ScanReport};
use crate::session::{Control, Medium};

/// Bytes re-read individually when a whole chunk fails to read, so damage is
/// reported at this granularity instead of a whole chunk's worth.
const RETRY_SPAN_BYTES: usize = 64 * 1024;

/// Bytes copied per step when hashing or streaming an artifact.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Cap on signature hits one worker collects during a sweep.
///
/// Candidates are held in memory until validation, so a medium whose free
/// space is a repeated signature would otherwise exhaust memory before the
/// scan could report anything (A-BOUNDED-ALLOC). A 4 TB disk of 3 MB photos
/// holds under two million files, so a worker reaching this has found a
/// pattern, not a library; the overflow is reported, never silently dropped.
const MAX_CANDIDATES_PER_WORKER: usize = 2_000_000;

/// Most progress events a stage counted in items will emit.
///
/// A stage handling a million candidates must not emit a million events
/// (`M-LOG-OVERHEAD`); at this cap a progress bar still moves in steps too
/// small for an eye to catch.
const PROGRESS_STEPS: u64 = 200;

/// Longest a stage may work without saying so, in milliseconds.
///
/// The stride is a fraction of the total, which says nothing about how long a
/// step takes. Reassembly's items range over two orders of magnitude — a
/// hypothesis costs what the decoder walks through before rejecting it, and a
/// region of photographs is a hundred times a region of noise — so a stride of
/// a fraction of a percent can be hours apart. A stage that goes quiet exactly
/// when it is slowest is a stage that cannot be told from a stalled one.
const PROGRESS_INTERVAL_MS: u64 = 5_000;

/// How often a stage watching the clock rather than its work looks at it.
///
/// Well under [`PROGRESS_INTERVAL_MS`], so an event goes out close to when it
/// is due, and long enough that the watcher costs nothing: it wakes, compares
/// two instants and goes back to sleep (`M-LOG-OVERHEAD`). It also bounds how
/// long the stage waits for the watcher when the search finishes first.
const TICK_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// What a stage's progress is measured against.
#[derive(Clone, Copy, Debug)]
enum Bound {
    /// An amount of work, advanced by [`Counter::step`].
    Work {
        /// Work the stage expects to cover.
        total: u64,
        /// What `total` counts.
        unit: Unit,
        /// Steps between two events, never zero.
        stride: u64,
    },
    /// A wall clock the stage ends on, however much of its queue it reached.
    ///
    /// Reassembly is the only stage bounded this way, and it has to report
    /// this way for the same reason it is bounded this way: a decode's cost is
    /// not a constant, so neither the queue's length nor the position in it is
    /// a fraction of the time left. Elapsed against the budget is.
    Deadline {
        /// How long the stage may run.
        budget: std::time::Duration,
    },
}

impl Bound {
    /// The unit and total a stage announces itself with.
    fn announced(self) -> (Unit, u64) {
        match self {
            Self::Work { total, unit, .. } => (unit, total),
            Self::Deadline { budget } => (Unit::Seconds, budget.as_secs()),
        }
    }
}

/// Reports how far a stage has got, against the amount of work it has to do or
/// against the clock it ends on.
///
/// Shared by the workers of a parallel stage, so the count is one number for
/// the stage rather than one per thread.
pub struct Counter<'a, P: ?Sized> {
    progress: &'a P,
    stage: Stage,
    /// What progress is measured against.
    bound: Bound,
    done: AtomicU64,
    /// When the stage began.
    started: std::time::Instant,
    /// Milliseconds after `started` at which the last event went out.
    spoke_at: AtomicU64,
}

impl<'a, P: ProgressSink + ?Sized> Counter<'a, P> {
    /// Announces the stage and prepares to count `total` of `unit`.
    pub(crate) fn start(progress: &'a P, stage: Stage, total: u64, unit: Unit) -> Self {
        Self::new(
            progress,
            stage,
            Bound::Work {
                total,
                unit,
                stride: total.div_ceil(PROGRESS_STEPS).max(1),
            },
        )
    }

    /// Announces a stage that ends on a deadline, and prepares to report how
    /// much of `budget` it has spent.
    ///
    /// Steps are still counted — [`Counter::step`] is what keeps the clock
    /// honest across a quiet stretch — but what goes out is the clock, because
    /// that is what reaches its end when the stage does.
    pub(crate) fn until(progress: &'a P, stage: Stage, budget: std::time::Duration) -> Self {
        Self::new(progress, stage, Bound::Deadline { budget })
    }

    fn new(progress: &'a P, stage: Stage, bound: Bound) -> Self {
        let (unit, total) = bound.announced();
        progress.emit(ScanEvent::StageStarted { stage, unit, total });
        Self {
            progress,
            stage,
            bound,
            done: AtomicU64::new(0),
            started: std::time::Instant::now(),
            spoke_at: AtomicU64::new(0),
        }
    }

    /// Records one item handled, reporting on stride boundaries, at the end,
    /// and whenever the stage has been quiet for [`PROGRESS_INTERVAL_MS`].
    pub(crate) fn step(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if let Bound::Work { total, stride, .. } = self.bound
            && (done.is_multiple_of(stride) || done == total)
        {
            self.report(done);
            return;
        }
        self.report_if_quiet(done);
    }

    /// Reports without advancing, for a caller watching the clock rather than
    /// the work.
    ///
    /// A stage whose items take tens of minutes cannot report from inside
    /// [`Counter::step`] alone: nothing calls it until an item finishes, and a
    /// stage that goes quiet exactly when it is slowest cannot be told from a
    /// stalled one.
    pub(crate) fn tick(&self) {
        self.report_if_quiet(self.done.load(Ordering::Relaxed));
    }

    /// Reports if nothing has for [`PROGRESS_INTERVAL_MS`].
    ///
    /// One caller wins the interval and speaks for the stage; the others go
    /// straight back to work.
    fn report_if_quiet(&self, done: u64) {
        let now = self.millis();
        if now.saturating_sub(self.spoke_at.load(Ordering::Relaxed)) < PROGRESS_INTERVAL_MS {
            return;
        }
        let last = self.spoke_at.swap(now, Ordering::AcqRel);
        if now.saturating_sub(last) >= PROGRESS_INTERVAL_MS {
            self.report(done);
        }
    }

    /// Milliseconds since the stage began.
    fn millis(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn report(&self, done: u64) {
        self.spoke_at.store(self.millis(), Ordering::Relaxed);
        let (unit, done, total) = match self.bound {
            Bound::Work { total, unit, .. } => (unit, done, total),
            // Capped at the budget: a stage that overran its deadline waiting
            // for an item to finish must not report more than all of it.
            Bound::Deadline { budget } => (
                Unit::Seconds,
                self.started.elapsed().as_secs().min(budget.as_secs()),
                budget.as_secs(),
            ),
        };
        self.progress.emit(ScanEvent::StageProgress {
            stage: self.stage,
            unit,
            done,
            total,
        });
    }
}

/// Runs every configured stage over `medium`.
pub fn run<V, S, P, C>(
    config: &ScanConfig,
    control: &Control,
    medium: Medium<V>,
    sink: &mut S,
    progress: &P,
    classifier: Option<&mut C>,
) -> Result<ScanReport, ScanError>
where
    V: Read + Seek + Send,
    S: ArtifactSink,
    P: ProgressSink + ?Sized,
    C: Classifier + Send,
{
    let (mut views, medium_len) = medium.into_parts();
    let range = config.range_within(medium_len);
    let (range_start, range_end) = (range.start.get(), range.end_saturating().get());
    let mut report = ScanReport::default();
    let mut findings = Vec::new();

    let swept = sweep(
        config,
        control,
        &mut views,
        Span {
            start: range_start,
            end: range_end,
            medium_len,
        },
        progress,
        &mut report,
    );

    if config.stages().filesystem {
        let mut residue = swept.residue;
        let recovered = recover_filesystems(
            control,
            &mut views,
            medium_len,
            &mut residue,
            progress,
            &mut report,
        );
        report.volumes = residue.volumes;
        progress.emit(ScanEvent::StageFinished {
            stage: Stage::Filesystem,
            findings: recovered.len() as u64,
        });
        findings.extend(recovered);
    }

    if config.stages().carving {
        // The volumes the sweep located, so the search steps on the grid the
        // allocator that split these files actually used.
        let volumes = std::mem::take(&mut report.volumes);
        carve(
            config,
            control,
            &mut views,
            swept.candidates,
            range_end,
            &volumes,
            &mut findings,
            progress,
            &mut report,
        );
        report.volumes = volumes;
    }

    report_findings(
        config,
        control,
        &mut views,
        findings,
        sink,
        progress,
        classifier,
        &mut report,
    )?;
    Ok(report)
}

/// Runs the search alone, over fragmentation points a previous run located.
///
/// The stages that find them — the sweep and the validation pass — are what a
/// scan of a large medium spends its hours on, and they establish the same
/// points every time. Starting from them is what makes the search's budget,
/// its size floor and its ceilings something a person can try again in minutes
/// instead of overnight.
///
/// Nothing is assumed about the medium beyond those points: every extent this
/// reports is read back and hashed exactly as a scan's is, so a session pointed
/// at the wrong disk recovers nothing rather than something wrong.
pub fn resume<V, S, P, C>(
    config: &ScanConfig,
    control: &Control,
    medium: Medium<V>,
    broken: &[Broken],
    sink: &mut S,
    progress: &P,
    classifier: Option<&mut C>,
) -> Result<ScanReport, ScanError>
where
    V: Read + Seek + Send,
    S: ArtifactSink,
    P: ProgressSink + ?Sized,
    C: Classifier + Send,
{
    let (mut views, medium_len) = medium.into_parts();
    let range = config.range_within(medium_len);
    let range_end = range.end_saturating().get();
    let mut report = ScanReport {
        fragmentation: broken.to_vec(),
        ..ScanReport::default()
    };
    let mut findings = Vec::new();

    let mut whole_again = HashSet::new();
    if config.stages().reassembly {
        let reassembled = reassemble_broken(
            control,
            &mut views,
            Reassembling {
                broken,
                already_recovered: &[],
                budget: config.reassembly_budget(),
                min_long_side: config.min_long_side(),
                // A resumed run did not locate any volumes, so it steps on the
                // finest grid rather than assuming one.
                volumes: &[],
                medium_len: range_end,
            },
            progress,
            &mut report,
        );
        whole_again.extend(
            reassembled
                .iter()
                .filter_map(|finding| finding.extents.first().map(|extent| extent.start)),
        );
        progress.emit(ScanEvent::StageFinished {
            stage: Stage::Reassembly,
            findings: reassembled.len() as u64,
        });
        findings.extend(reassembled);
    }
    let partials = partial_prefixes(broken, &whole_again, config.min_long_side());
    report.partial_prefixes = partials.len() as u64;
    findings.extend(partials);

    report_findings(
        config,
        control,
        &mut views,
        findings,
        sink,
        progress,
        classifier,
        &mut report,
    )?;
    Ok(report)
}

/// Merges what the stages found, writes it, and annotates what was written.
#[expect(
    clippy::too_many_arguments,
    reason = "the tail of both entry points; naming its inputs twice would be worse"
)]
fn report_findings<V, S, P, C>(
    config: &ScanConfig,
    control: &Control,
    views: &mut [V],
    mut findings: Vec<Finding>,
    sink: &mut S,
    progress: &P,
    classifier: Option<&mut C>,
    report: &mut ScanReport,
) -> Result<(), ScanError>
where
    V: Read + Seek + Send,
    S: ArtifactSink,
    P: ProgressSink + ?Sized,
    C: Classifier + Send,
{
    report.dropped_unreadable = finding::consolidate(&mut findings, &report.unreadable);
    // The size floor is applied inside the report stage or not at all: whether
    // to write an artifact has to be settled before it is written.
    let emitted = emit(
        control,
        views,
        &findings,
        sink,
        config.min_long_side(),
        progress,
        report,
    )?;

    report.cache_runs = same_size_runs(&emitted);
    report.standings = standings(&emitted, &report.cache_runs);

    // Annotation runs last, over artifacts already persisted and recorded:
    // previews and triage labels describe them and can no longer change what
    // was recovered (A-TRIAGE-NOT-VERDICT).
    let work = crate::annotate::Work {
        sink,
        classifier,
        previews: config.previews(),
    };
    if !work.is_idle()
        && let Some(view) = views.first_mut()
    {
        crate::annotate::run(control, view, &findings, &emitted, work, progress, report);
    }
    Ok(())
}

/// What the medium's layout says about the artifacts just written.
///
/// A run of same-sized neighbours is a thumbnail cache, and naming it is what
/// stops a report from presenting a preview of a lost photograph as the
/// photograph.
fn same_size_runs(emitted: &[crate::annotate::Emitted]) -> Vec<crate::finding::CacheRun> {
    let entries: Vec<_> = emitted
        .iter()
        .map(|item| crate::finding::Entry {
            offset: item.offset,
            pixels: item.pixels,
            sha256: item.sha256,
        })
        .collect();
    crate::finding::runs(&entries)
}

/// Where each written artifact stands in a list, by the evidence about it.
///
/// A whole-disk recovery writes far more cache entries and icons than
/// photographs, and the photographs are what a person is looking for. This is
/// the sort key that lets a reader — a report, a window — put them first
/// without anything being hidden: every artifact gets one, and the weakest
/// standing is still a standing (`A-TRIAGE-NOT-VERDICT`).
///
/// The run of same-sized neighbours is folded in here rather than at emit
/// time, because it is a fact about the artifacts around one and is only known
/// once they have all been written.
fn standings(
    emitted: &[crate::annotate::Emitted],
    runs: &[crate::finding::CacheRun],
) -> Vec<(Digest, argos_classify::rank::Standing)> {
    emitted
        .iter()
        .map(|item| {
            let evidence = runs
                .iter()
                .find(|run| run.sha256 == item.sha256)
                .map_or(item.evidence, |run| {
                    item.evidence.among_neighbours(run.neighbours)
                });
            (item.sha256, argos_classify::rank::standing(&evidence))
        })
        .collect()
}

/// The byte span a scan covers, and the medium it sits in.
#[derive(Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
    medium_len: u64,
}

/// One chunk of the medium on its way from the reader to a worker.
struct Chunk {
    start: ByteOffset,
    buf: Vec<u8>,
}

/// What one full-surface sweep found.
#[derive(Default)]
struct Swept {
    candidates: Vec<Candidate>,
    residue: residue::Sweep,
    /// Whether a detector hit its cap and stopped collecting.
    truncated: bool,
}

impl Swept {
    fn merge(&mut self, other: Self) {
        self.candidates.extend(other.candidates);
        self.residue.merge(other.residue);
        self.truncated |= other.truncated;
    }
}

/// Reads the medium once and detects, in parallel, everything detectable from
/// a buffer alone: image signatures and filesystem anchors.
fn sweep<V, P>(
    config: &ScanConfig,
    control: &Control,
    views: &mut [V],
    span: Span,
    progress: &P,
    report: &mut ScanReport,
) -> Swept
where
    V: Read + Seek + Send,
    P: ProgressSink + ?Sized,
{
    let stages = config.stages();
    progress.emit(ScanEvent::StageStarted {
        stage: Stage::Carve,
        unit: Unit::Bytes,
        total: span.end.saturating_sub(span.start),
    });

    let depth = config.queue_depth().max(1);
    let (free_tx, free_rx) = crossbeam_channel::bounded::<Vec<u8>>(depth);
    for _ in 0..depth {
        // A send into a channel with free capacity cannot fail.
        let _ = free_tx.send(Vec::new());
    }
    let (chunk_tx, chunk_rx) = crossbeam_channel::bounded::<Chunk>(depth);

    let Some(reader) = views.first_mut() else {
        return Swept::default();
    };

    let swept = thread::scope(|scope| {
        let mut workers = Vec::with_capacity(config.workers().get());
        for _ in 0..config.workers().get() {
            let chunks = chunk_rx.clone();
            let recycle = free_tx.clone();
            workers.push(scope.spawn(move || {
                let detector = Detector::new();
                let mut found = Swept::default();
                while let Ok(chunk) = chunks.recv() {
                    if control.is_cancelled() {
                        break;
                    }
                    if stages.carving
                        && !detector.hits_in(
                            &chunk.buf,
                            chunk.start,
                            MAX_CANDIDATES_PER_WORKER,
                            &mut found.candidates,
                        )
                    {
                        found.truncated = true;
                    }
                    if stages.filesystem
                        && !residue::scan_window(
                            &chunk.buf,
                            chunk.start,
                            span.medium_len,
                            &mut found.residue,
                        )
                    {
                        found.truncated = true;
                    }
                    // Return the buffer to the pool; a closed pool means the
                    // reader is gone and this worker is about to be too.
                    if recycle.send(chunk.buf).is_err() {
                        break;
                    }
                }
                found
            }));
        }
        // Only the workers may hold these, or the reader waits on itself.
        drop(chunk_rx);
        drop(free_tx);

        read_chunks(
            control, reader, span, config, &chunk_tx, &free_rx, progress, report,
        );
        drop(chunk_tx);

        workers
            .into_iter()
            .fold(Swept::default(), |mut all, worker| {
                match worker.join() {
                    Ok(found) => all.merge(found),
                    // A worker panicked: that is a bug in a detector, and a bug
                    // means stop rather than report a partial surface as complete.
                    Err(panic) => std::panic::resume_unwind(panic),
                }
                all
            })
    });

    let mut swept = swept;
    swept.candidates.sort_unstable();
    swept.candidates.dedup();
    swept.residue.normalize();
    report.ceilings.detection = swept.truncated;
    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Carve,
        findings: swept.candidates.len() as u64,
    });
    swept
}

/// The sequential reader: the only thing in the pipeline that touches the
/// medium during the sweep.
#[expect(
    clippy::too_many_arguments,
    reason = "the reader loop is one cohesive step; splitting its inputs into a struct \
              would only move the same wiring one level out"
)]
fn read_chunks<V, P>(
    control: &Control,
    view: &mut V,
    span: Span,
    config: &ScanConfig,
    chunks: &crossbeam_channel::Sender<Chunk>,
    pool: &crossbeam_channel::Receiver<Vec<u8>>,
    progress: &P,
    report: &mut ScanReport,
) where
    V: Read + Seek,
    P: ProgressSink + ?Sized,
{
    let total = span.end.saturating_sub(span.start);
    let mut pos = span.start;
    while pos < span.end {
        if !control.wait_while_paused() {
            return;
        }
        let Ok(mut buf) = pool.recv() else {
            return;
        };
        let want = usize::try_from((span.end - pos).min(config.chunk_bytes() as u64))
            .unwrap_or(config.chunk_bytes());
        // Deliberately no `clear()` first: a pooled buffer already has the
        // previous chunk's length, so `resize` is a no-op and the reader skips
        // a chunk-sized memset it would immediately overwrite. Clearing first
        // costs one full pass over the medium on the single reader thread
        // (`M-MEM-REUSE`). `fill_chunk` leaves no stale byte behind: every byte
        // is either read from the medium or explicitly zeroed as unreadable.
        buf.resize(want, 0);
        fill_chunk(view, pos, &mut buf, report, progress);

        let last = pos.saturating_add(want as u64) >= span.end;
        if chunks
            .send(Chunk {
                start: ByteOffset::new(pos),
                buf,
            })
            .is_err()
        {
            return;
        }
        report.bytes_swept = pos.saturating_add(want as u64).saturating_sub(span.start);
        progress.emit(ScanEvent::StageProgress {
            stage: Stage::Carve,
            unit: Unit::Bytes,
            done: report.bytes_swept,
            total,
        });
        if last {
            return;
        }
        // Step back by the overlap so a structure straddling the boundary is
        // whole in the next chunk. The chunk size is validated to exceed it.
        pos = pos.saturating_add((want - CHUNK_OVERLAP_BYTES) as u64);
    }
}

/// Fills `buf` from `at`, recording what the medium refused to give.
///
/// A failed span is zeroed rather than left stale and its range is recorded;
/// every finding overlapping a recorded range is dropped before reporting, so
/// no zero ever reaches an artifact.
fn fill_chunk<V, P>(view: &mut V, at: u64, buf: &mut [u8], report: &mut ScanReport, progress: &P)
where
    V: Read + Seek,
    P: ProgressSink + ?Sized,
{
    if read_exact_at(view, at, buf).is_ok() {
        return;
    }
    // Something in this chunk is unreadable. Narrow it down so the damage is
    // reported at retry-span granularity and the readable rest is still swept.
    for (index, span) in buf.chunks_mut(RETRY_SPAN_BYTES).enumerate() {
        let offset = at.saturating_add((index * RETRY_SPAN_BYTES) as u64);
        if read_exact_at(view, offset, span).is_err() {
            span.fill(0);
            let range = ByteRange::new(ByteOffset::new(offset), span.len() as u64);
            report.unreadable.push(range);
            progress.emit(ScanEvent::RegionUnreadable { range });
        }
    }
}

fn read_exact_at<V: Read + Seek>(view: &mut V, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
    view.seek(SeekFrom::Start(at))?;
    view.read_exact(buf)
}

/// Whether two ranges share any byte.
///
/// The test for "can this claim ever matter to a search bounded by that
/// region": one that lies entirely outside it cannot match a block inside it,
/// whatever else is true of it.
fn overlaps(one: ByteRange, other: ByteRange) -> bool {
    one.start.get() < other.end_saturating().get() && other.start.get() < one.end_saturating().get()
}
fn covers(outer: ByteRange, inner: ByteRange) -> bool {
    let outer_end = outer.end_saturating().get();
    let inner_end = inner.end_saturating().get();
    outer.start <= inner.start && inner_end <= outer_end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that counts what a stage said, for the tests below about *when*
    /// it speaks rather than about what it found.
    #[derive(Default)]
    struct Spoken {
        progress: AtomicU64,
        last: std::sync::Mutex<Option<(Unit, u64, u64)>>,
    }

    impl Spoken {
        fn count(&self) -> u64 {
            self.progress.load(Ordering::Relaxed)
        }

        fn last(&self) -> Option<(Unit, u64, u64)> {
            *self
                .last
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
    }

    impl ProgressSink for Spoken {
        fn emit(&self, event: ScanEvent) {
            if let ScanEvent::StageProgress {
                unit, done, total, ..
            } = event
            {
                self.progress.fetch_add(1, Ordering::Relaxed);
                *self
                    .last
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((unit, done, total));
            }
        }
    }

    /// A stage whose items take tens of minutes still has to speak on the
    /// clock.
    ///
    /// [`Counter::step`] was the only thing that reported, and nothing calls it
    /// until an item finishes: the field run behind `docs/defects/09` spent
    /// 4,112 s on one item without a word, from a counter contracted to speak
    /// every five seconds. That is indistinguishable from a stalled run, and it
    /// is what got a working one cancelled.
    #[test]
    fn a_tick_speaks_for_a_stage_that_has_not_finished_an_item() {
        let sink = Spoken::default();
        let mut counter = Counter::until(
            &sink,
            Stage::Reassembly,
            std::time::Duration::from_secs(600),
        );

        // Nothing yet: the stage has only just begun and has nothing to add.
        counter.tick();
        assert_eq!(counter.done.load(Ordering::Relaxed), 0, "no item finished");
        assert_eq!(sink.count(), 0, "a stage that just started has said enough");

        // Quiet for longer than the interval now, and still no step taken.
        counter.started -= std::time::Duration::from_millis(PROGRESS_INTERVAL_MS + 1);
        counter.tick();
        assert_eq!(
            sink.count(),
            1,
            "the clock, not the work, is what makes a deadline-bounded stage speak"
        );
        assert_eq!(
            counter.done.load(Ordering::Relaxed),
            0,
            "a tick is not a step"
        );

        let (unit, done, total) = sink.last().expect("the tick reported");
        assert_eq!(unit, Unit::Seconds, "a budget is spoken of in seconds");
        assert_eq!(total, 600, "the denominator is the budget");
        assert!(
            done >= 5,
            "elapsed is what was spent, not what was finished: {done}"
        );

        // And it does not repeat itself inside one interval.
        counter.tick();
        assert_eq!(sink.count(), 1, "one caller wins the interval");
    }

    /// A deadline-bounded stage that overran its budget waiting for an item to
    /// finish reports all of it, never more.
    #[test]
    fn a_stage_that_overran_its_budget_reports_it_full_and_not_past_full() {
        let sink = Spoken::default();
        let mut counter =
            Counter::until(&sink, Stage::Reassembly, std::time::Duration::from_secs(10));

        counter.started -= std::time::Duration::from_secs(90);
        counter.tick();

        let (_, done, total) = sink.last().expect("the tick reported");
        assert_eq!(
            (done, total),
            (10, 10),
            "a bar past its end reports a run that overshot as one still going"
        );
    }
}
