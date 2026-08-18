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

use argos_carve::reassemble::{self, Broken};
use argos_carve::{Candidate, Detector, Scratch, Verdict};
use argos_core::artifact::{Artifact, ArtifactSink, Digest};
use argos_core::classify::Classifier;
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::progress::{ProgressSink, ScanEvent, Unit};
use argos_core::{Confidence, Format, Stage};
use argos_fs::{DeletedFile, FsKind, Volume, residue};
use sha2::{Digest as _, Sha256};

use crate::config::{CHUNK_OVERLAP_BYTES, ScanConfig};
use crate::error::ScanError;
use crate::finding::{Finding, ScanReport};
use crate::merge;
use crate::search;
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

/// Reports how far a stage that counts items has got.
///
/// Shared by the workers of a parallel stage, so the count is one number for
/// the stage rather than one per thread.
pub(crate) struct Counter<'a, P: ?Sized> {
    progress: &'a P,
    stage: Stage,
    total: u64,
    /// What `total` counts.
    unit: Unit,
    /// Steps between two events, never zero.
    stride: u64,
    done: AtomicU64,
    /// When the stage began.
    started: std::time::Instant,
    /// Milliseconds after `started` at which the last event went out.
    spoke_at: AtomicU64,
}

impl<'a, P: ProgressSink + ?Sized> Counter<'a, P> {
    /// Announces the stage and prepares to count `total` of `unit`.
    pub(crate) fn start(progress: &'a P, stage: Stage, total: u64, unit: Unit) -> Self {
        progress.emit(ScanEvent::StageStarted { stage, unit, total });
        Self {
            progress,
            stage,
            total,
            unit,
            stride: total.div_ceil(PROGRESS_STEPS).max(1),
            done: AtomicU64::new(0),
            started: std::time::Instant::now(),
            spoke_at: AtomicU64::new(0),
        }
    }

    /// Records one item handled, reporting on stride boundaries, at the end,
    /// and whenever the stage has been quiet for [`PROGRESS_INTERVAL_MS`].
    pub(crate) fn step(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if done.is_multiple_of(self.stride) || done == self.total {
            self.report(done);
            return;
        }
        // Otherwise on the clock. One caller wins the interval and speaks for
        // the stage; the others go straight back to work.
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
        self.progress.emit(ScanEvent::StageProgress {
            stage: self.stage,
            unit: self.unit,
            done,
            total: self.total,
        });
    }
}

/// Runs every configured stage over `medium`.
pub(crate) fn run<V, S, P, C>(
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
pub(crate) fn resume<V, S, P, C>(
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
    report.dropped_unreadable = merge::consolidate(&mut findings, &report.unreadable);
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
fn same_size_runs(emitted: &[crate::annotate::Emitted]) -> Vec<crate::cache_run::CacheRun> {
    let entries: Vec<_> = emitted
        .iter()
        .map(|item| crate::cache_run::Entry {
            offset: item.offset,
            pixels: item.pixels,
            sha256: item.sha256,
        })
        .collect();
    crate::cache_run::runs(&entries)
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
    runs: &[crate::cache_run::CacheRun],
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

/// Recovers deleted files from every volume the sweep located, current and
/// residual, plus the orphaned NTFS records left behind by a re-format.
fn recover_filesystems<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    views: &mut [V],
    medium_len: u64,
    sweep: &mut residue::Sweep,
    progress: &P,
    report: &mut ScanReport,
) -> Vec<Finding> {
    let Some(view) = views.first_mut() else {
        return Vec::new();
    };

    // Before anything is resolved against them: an anchor is a sector that
    // parsed, and the sweep has no way to tell the boot sector at a volume's
    // start from the copy at its end, or either from 512 bytes that satisfied
    // the checks by chance.
    // The geometry is kept, not just the corrected range: when the primary
    // boot sector is what a later format overwrote, re-deriving it from the
    // volume's start would read the very sector that is gone.
    let geometries = confirm_ntfs(view, &mut sweep.volumes, medium_len);

    let current: Vec<ByteRange> = argos_fs::part::scan(view, medium_len)
        .map(|tables| {
            tables
                .partitions
                .iter()
                .map(|partition| partition.range)
                .collect()
        })
        .unwrap_or_default();
    sweep.mark_current(&current);

    // One scratch for the whole stage: structural validation of a recovered
    // file reuses the same working memory as every other validation.
    let mut scratch = Scratch::new();
    let mut found = Vec::new();
    // Counted in volumes: one volume's metadata can take minutes to walk, and
    // a count of volumes is the only honest denominator this stage has before
    // it opens them.
    let counter = Counter::start(
        progress,
        Stage::Filesystem,
        sweep.volumes.len() as u64,
        Unit::Items,
    );
    for volume in &sweep.volumes {
        if control.is_cancelled() {
            return found;
        }
        let geometry = geometries
            .iter()
            .find(|geometry| geometry.volume_offset == volume.range.start)
            .copied();
        found.extend(recover_volume(view, *volume, geometry, &mut scratch));
        counter.step();
    }

    // Orphaned `FILE` records store volume-relative cluster numbers, so they
    // can only be resolved against the volume they belong to. A region no
    // confirmed NTFS volume covers is counted, never resolved against a guess.
    let ntfs_volumes: Vec<Volume> = sweep
        .volumes
        .iter()
        .copied()
        .filter(|volume| volume.kind == FsKind::Ntfs)
        .collect();
    for region in &sweep.ntfs_records {
        if control.is_cancelled() {
            return found;
        }
        let Some(geometry) = ntfs_volumes
            .iter()
            .find(|volume| covers(volume.range, *region))
            .and_then(|volume| {
                geometries
                    .iter()
                    .find(|geometry| geometry.volume_offset == volume.range.start)
            })
        else {
            report.unattributed_residue += 1;
            // No geometry means no extents, and that is the whole of what is
            // missing: a record still names its file, states its size and
            // carries the times it was made and last written, none of which
            // depend on where the volume began. Reading them costs one pass
            // over records already located, and skipping it is what made a
            // re-formatted disk look like a disk that never held anything.
            if let Ok(lost) = argos_fs::ntfs::orphan_records(view, *region) {
                report.lost_files.extend(lost);
            }
            continue;
        };
        if let Ok(files) = argos_fs::ntfs::orphan_scan(
            view,
            *region,
            geometry.volume_offset,
            geometry.cluster_bytes,
        ) {
            found.extend(
                files
                    .into_iter()
                    .filter_map(|file| finding_from(view, file, &mut scratch)),
            );
        }
    }

    name_from_index_slack(
        control,
        view,
        &sweep.ntfs_indexes,
        &ntfs_volumes,
        &mut found,
    );
    name_from_change_journal(control, view, &geometries, &mut found, report);
    found
}

/// Names and dates findings from what each volume's change journal recorded.
///
/// The `$UsnJrnl:$J` stream is the only place an NTFS volume records *when* a
/// file stopped existing — a `FILE` record keeps the times the file was made
/// and last written, not the moment it was removed. So a batch of files
/// deleted in one action is recognisable here and nowhere else, as a run of
/// entries sharing a timestamp.
///
/// Nothing here creates an extent or raises a tier. An event is evidence that
/// a file was deleted, which is not evidence that its bytes survived, and a
/// finding that already carries a name from its own record keeps it — that
/// record is the better evidence (`A-CONFIDENCE-HONEST`).
fn name_from_change_journal<V: Read + Seek>(
    control: &Control,
    view: &mut V,
    geometries: &[argos_fs::ntfs::Ntfs],
    found: &mut [Finding],
    report: &mut ScanReport,
) {
    if found.iter().all(|finding| finding.source_object.is_none()) {
        return;
    }
    for geometry in geometries {
        if control.is_cancelled() {
            return;
        }
        let Ok(events) = geometry.change_journal(view) else {
            continue;
        };
        report.journal_deletions = report.journal_deletions.saturating_add(events.len() as u64);
        for event in events {
            for finding in found.iter_mut() {
                if finding.source_object != Some(event.source_object) {
                    continue;
                }
                if finding.name.is_none() {
                    finding.name = Some(event.event.name.clone().into_boxed_str());
                }
                // The deletion time is a fact about the event, not about the
                // file's own timestamps, so it never overwrites one.
                if finding.deleted.is_none() {
                    finding.deleted = event.event.timestamp;
                }
            }
        }
    }
}

/// Confirms every NTFS anchor against the volume it claims, in place.
///
/// The residue sweep reports a volume for any sector that parses as an NTFS
/// boot sector, and three different things do:
///
/// - the boot sector at a volume's first sector, which is what it looks like;
/// - the copy NTFS keeps in the volume's **last** sector, byte-identical, which
///   read as a start puts the volume and its `$MFT` almost a volume's length
///   past where they are;
/// - 512 bytes that satisfied the structural checks by coincidence, which a
///   sweep of a terabyte produces in quantity.
///
/// Only the first is usable as it stands, and nothing in the bytes tells them
/// apart. [`argos_fs::ntfs::locate`] settles it by reading the `$MFT` each
/// interpretation implies: the one with a real record behind it is the volume.
/// A copy is corrected to the volume it belongs to, and a coincidence is
/// dropped rather than offered to the stages that resolve extents against a
/// volume's geometry (`A-CONFIDENCE-HONEST`).
///
/// Corrected anchors collapse: the two ends of one volume name one volume.
///
/// Returns the geometry of each confirmed volume, which is what the stages
/// below resolve against. Handing back the geometry rather than re-deriving it
/// from the volume's start is the point: the case this exists for is a primary
/// boot sector a later format overwrote, and reading it again would read the
/// sector that is gone.
fn confirm_ntfs<V: Read + Seek>(
    view: &mut V,
    volumes: &mut Vec<Volume>,
    medium_len: u64,
) -> Vec<argos_fs::ntfs::Ntfs> {
    let mut confirmed: Vec<Volume> = Vec::with_capacity(volumes.len());
    let mut geometries: Vec<argos_fs::ntfs::Ntfs> = Vec::new();
    for volume in volumes.drain(..) {
        if volume.kind != FsKind::Ntfs {
            confirmed.push(volume);
            continue;
        }
        // Unreadable counts as unconfirmed: a geometry that cannot be checked
        // is one nothing may be resolved against.
        let Ok(Some(geometry)) = argos_fs::ntfs::locate(view, volume.range.start) else {
            continue;
        };
        let remaining = medium_len.saturating_sub(geometry.volume_offset.get());
        confirmed.push(Volume {
            kind: FsKind::Ntfs,
            range: ByteRange::new(geometry.volume_offset, geometry.volume_bytes.min(remaining)),
            origin: volume.origin,
            allocation_bytes: geometry.cluster_bytes,
        });
        geometries.push(geometry);
    }
    confirmed.sort_by_key(|volume| (volume.range.start, volume.range.len));
    confirmed.dedup();
    geometries.sort_by_key(|geometry| geometry.volume_offset);
    geometries.dedup();
    *volumes = confirmed;
    geometries
}

/// Names findings from the `$FILE_NAME` copies a directory index kept.
///
/// A directory that removes an entry leaves it in the index buffer's slack, so
/// a file's name can survive when its own record's `$FILE_NAME` did not. The
/// entry carries the MFT record number, which is what ties a name to a
/// recovery — nothing here creates an extent, and a finding that already has a
/// name keeps it, because its own record is the better evidence
/// (`A-CONFIDENCE-HONEST`).
fn name_from_index_slack<V: Read + Seek>(
    control: &Control,
    view: &mut V,
    regions: &[ByteRange],
    volumes: &[Volume],
    found: &mut [Finding],
) {
    let nameless: Vec<usize> = found
        .iter()
        .enumerate()
        .filter(|(_, finding)| finding.name.is_none() && finding.source_object.is_some())
        .map(|(index, _)| index)
        .collect();
    if nameless.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for region in regions {
        if control.is_cancelled() {
            return;
        }
        // An index entry numbers a record; a finding is identified by where
        // its record sat. The two meet only through the geometry of the volume
        // the index belongs to, so an index no located volume covers names
        // nothing rather than naming by coincidence.
        let Some(geometry) = volumes
            .iter()
            .find(|volume| covers(volume.range, *region))
            .and_then(|volume| {
                argos_fs::ntfs::Ntfs::open(view, volume.range.start)
                    .ok()
                    .flatten()
            })
        else {
            continue;
        };
        let len = usize::try_from(region.len).unwrap_or(0);
        buf.clear();
        buf.resize(len, 0);
        if len == 0 || read_exact_at(view, region.start.get(), &mut buf).is_err() {
            continue;
        }
        for ghost in argos_fs::ntfs::indx_names(&buf) {
            // Where that record number sits, for an unfragmented `$MFT`. A
            // fragmented one puts it elsewhere, and then this names nothing —
            // a miss, never a wrong name.
            let Some(at) = ghost
                .mft_record
                .checked_mul(u64::from(geometry.record_size))
                .and_then(|offset| geometry.mft_offset.checked_add(offset))
            else {
                continue;
            };
            for &index in &nameless {
                let finding = &mut found[index];
                if finding.name.is_none() && finding.source_object == Some(at.get()) {
                    finding.name = Some(ghost.name.clone().into_boxed_str());
                    if finding.timestamps == argos_core::Timestamps::default() {
                        finding.timestamps = ghost.timestamps;
                    }
                }
            }
        }
    }
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

/// Recovers what one located volume's metadata still describes.
///
/// `ntfs` is the geometry [`confirm_ntfs`] established for an NTFS volume,
/// carried in rather than read again: the volume this stage most needs to
/// recover from is one whose first sector a later format overwrote, and that
/// is exactly the sector re-deriving it would read.
fn recover_volume<V: Read + Seek>(
    view: &mut V,
    volume: Volume,
    ntfs: Option<argos_fs::ntfs::Ntfs>,
    scratch: &mut Scratch,
) -> Vec<Finding> {
    let at = volume.range.start;
    let files = match volume.kind {
        FsKind::Ntfs => ntfs.and_then(|fs| fs.recover_deleted(view).ok()),
        FsKind::Ext4 => argos_fs::ext4::Ext4::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_from_journal(view).ok()),
        FsKind::Fat32 | FsKind::ExFat => argos_fs::fat::Fat::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_deleted(view).ok()),
        FsKind::Apfs => argos_fs::apfs::Apfs::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_deleted(view).ok()),
        // A filesystem family with no metadata parser yet. Carving still
        // covers its surface; claiming a recovery here would not be honest.
        _ => None,
    };
    files
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| finding_from(view, file, scratch))
        .collect()
}

/// Turns a filesystem's claim about a deleted file into a finding — but only
/// once the medium confirms the claim.
///
/// Metadata that survives a format can point anywhere: at a boot sector, at
/// clusters since reallocated, at nothing. So the claim is checked twice. The
/// signature must be there, and the *assembled* extents must pass the format's
/// state machine — the same validation a carved candidate has to pass. A tier
/// is a statement about evidence, so the strongest tier cannot be the least
/// checked one (A-CONFIDENCE-HONEST).
///
/// A file whose bytes are there but whose structure breaks — the shape a
/// spliced hole or a reallocated run produces — is still reported, because the
/// metadata is real evidence that a file lived here, but it is reported as the
/// partial recovery it is.
fn finding_from<V: Read + Seek>(
    view: &mut V,
    file: DeletedFile,
    scratch: &mut Scratch,
) -> Option<Finding> {
    let first = file.extents.first()?;
    let mut prefix = [0_u8; argos_carve::MAX_SIGNATURE_BYTES];
    read_exact_at(view, first.start.get(), &mut prefix).ok()?;
    let format = argos_carve::identify(&prefix)?;

    let extents = file.extents.into_boxed_slice();
    let recovered = extents
        .iter()
        .fold(0_u64, |sum, extent| sum.saturating_add(extent.len));
    // Metadata claiming more bytes than the extents cover is already a partial
    // recovery, whatever the structure says.
    // Whole means: the metadata's own size fits in what was recovered, and the
    // assembled bytes really are an image. Trailing slack past the image's end
    // is not a truncation — some cameras append to their own files — so any
    // complete verdict counts.
    let whole = file.size <= recovered
        && structure_of(view, &extents, recovered, format, scratch).is_some();

    Some(Finding {
        format,
        stage: Stage::Filesystem,
        confidence: if whole {
            file.confidence
        } else {
            Confidence::PartialOrThumbnail
        },
        extents,
        declared_size: Some(file.size),
        timestamps: file.timestamps,
        deleted: None,
        name: file.name.map(String::into_boxed_str),
        source_object: file.source_object,
        parent: None,
    })
}

/// Validates the concatenated extents as `format`, returning the image length
/// the state machine confirmed.
///
/// The extents are presented as one contiguous stream, so a file the
/// filesystem stored in pieces is validated as the file it was, not as
/// whatever follows its first fragment on the medium.
fn structure_of<V: Read + Seek>(
    view: &mut V,
    extents: &[ByteRange],
    length: u64,
    format: Format,
    scratch: &mut Scratch,
) -> Option<u64> {
    let mut assembled = ExtentReader::new(view, extents);
    match argos_carve::validate(format, &mut assembled, ByteOffset::new(0), length, scratch) {
        Ok(Verdict::Complete { length, .. }) => Some(length),
        Ok(Verdict::Corrupt { .. }) | Err(_) => None,
    }
}

/// Drives every signature hit through its format's state machine.
///
/// Each worker owns one view of the medium, so validation is parallel without
/// the views ever being shared. A hit that fails to validate is counted, never
/// reported: a magic number is not evidence.
fn validate_candidates<V, P>(
    control: &Control,
    views: &mut [V],
    candidates: Vec<Candidate>,
    medium_end: u64,
    progress: &P,
    report: &mut ScanReport,
) -> (Vec<Finding>, Vec<Broken>)
where
    V: Read + Seek + Send,
    P: ProgressSink + ?Sized,
{
    // Announced even when there is nothing to do, so the stage order a client
    // sees is the stage order that ran.
    let counter = Counter::start(
        progress,
        Stage::Validation,
        candidates.len() as u64,
        Unit::Items,
    );
    if candidates.is_empty() || views.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let (queue_tx, queue_rx) = crossbeam_channel::unbounded::<Candidate>();
    for candidate in candidates {
        // The queue is unbounded and pre-filled: it holds work already in
        // memory, so there is nothing to apply backpressure against.
        let _ = queue_tx.send(candidate);
    }
    drop(queue_tx);

    let outputs = thread::scope(|scope| {
        let mut workers = Vec::with_capacity(views.len());
        for view in views.iter_mut() {
            let queue = queue_rx.clone();
            let counter = &counter;
            workers.push(scope.spawn(move || {
                let mut scratch = Scratch::new();
                let mut found = Vec::new();
                let mut broken = Vec::new();
                let mut rejected = 0_u64;
                while let Ok(candidate) = queue.recv() {
                    if control.is_cancelled() {
                        break;
                    }
                    let validated = validate_one(view, candidate, medium_end, &mut scratch);
                    if validated.rejected {
                        rejected += 1;
                    }
                    found.extend(validated.findings);
                    broken.extend(validated.broken);
                    counter.step();
                }
                (found, broken, rejected)
            }));
        }
        drop(queue_rx);
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Vec<_>>()
    });

    let mut found = Vec::new();
    let mut broken = Vec::new();
    for (results, breaks, rejected) in outputs {
        found.extend(results);
        broken.extend(breaks);
        report.rejected_candidates = report.rejected_candidates.saturating_add(rejected);
    }
    // Deterministic order, so the stage that follows does not depend on which
    // worker finished first.
    broken.sort_unstable_by_key(|entry: &Broken| (entry.header, entry.break_at));
    (found, broken)
}

/// What validating one candidate established.
#[derive(Default)]
struct Validated {
    /// The image itself when it validated, plus its thumbnail when that did.
    findings: Vec<Finding>,
    /// Where the stream stopped being this file, when it broke. This is what
    /// reassembly starts from, so it comes from the entropy decoder rather
    /// than the marker grammar (see `argos_carve::reassemble::locate_break`).
    broken: Option<Broken>,
    /// Whether the candidate produced no finding at all.
    rejected: bool,
}

/// Validates one candidate.
///
/// A candidate that breaks is not simply discarded: it may still carry an
/// embedded thumbnail, and it always carries a fragmentation point, which is
/// the only thing that makes reassembly tractable.
fn validate_one<V: Read + Seek>(
    view: &mut V,
    candidate: Candidate,
    medium_end: u64,
    scratch: &mut Scratch,
) -> Validated {
    let limit = medium_end.min(
        candidate
            .offset
            .get()
            .saturating_add(argos_carve::MAX_IMAGE_BYTES),
    );
    let Ok(verdict) =
        argos_carve::validate(candidate.format, view, candidate.offset, limit, scratch)
    else {
        // A candidate we cannot read is a candidate we cannot report.
        return Validated {
            rejected: true,
            ..Validated::default()
        };
    };

    let (complete, thumbnail) = match verdict {
        Verdict::Complete { length, thumbnail } => (Some(length), thumbnail),
        Verdict::Corrupt { thumbnail, .. } => (None, thumbnail),
    };

    // A candidate that did not carve whole is where reassembly begins.
    let broken = if complete.is_none() {
        argos_carve::reassemble::locate_break(
            view,
            candidate.offset,
            candidate.format,
            medium_end,
            scratch,
        )
        .ok()
        .flatten()
    } else {
        None
    };

    let mut found = Vec::new();
    if let Some(length) = complete {
        found.push(Finding {
            format: candidate.format,
            stage: Stage::Carve,
            confidence: Confidence::ContiguousCarve,
            extents: Box::from([ByteRange::new(candidate.offset, length)]),
            declared_size: None,
            timestamps: argos_core::Timestamps::default(),
            deleted: None,
            name: None,
            source_object: None,
            parent: None,
        });
    }
    if let Some(thumb) = thumbnail
        && let Ok(Some(length)) = argos_carve::validate_thumbnail(view, thumb, medium_end, scratch)
    {
        found.push(Finding {
            format: Format::Jpeg,
            stage: Stage::Carve,
            confidence: Confidence::PartialOrThumbnail,
            extents: Box::from([ByteRange::new(thumb.offset, length)]),
            declared_size: None,
            timestamps: argos_core::Timestamps::default(),
            deleted: None,
            name: None,
            source_object: None,
            parent: Some(candidate.offset),
        });
    }

    Validated {
        rejected: found.is_empty(),
        findings: found,
        broken,
    }
}

/// Validates every signature hit, then recovers what did not carve whole.
///
/// Three outcomes reach `findings`: the images that carved contiguously, the
/// ones reassembly put back together, and — for the rest — the part of each
/// that decodes, because a photograph whose remainder was overwritten still
/// has a beginning on the medium.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; bundling the stage's inputs would name them twice"
)]
fn carve<V, P>(
    config: &ScanConfig,
    control: &Control,
    views: &mut [V],
    candidates: Vec<Candidate>,
    range_end: u64,
    volumes: &[Volume],
    findings: &mut Vec<Finding>,
    progress: &P,
    report: &mut ScanReport,
) where
    V: Read + Seek + Send,
    P: ProgressSink + ?Sized,
{
    let (carved, broken) =
        validate_candidates(control, views, candidates, range_end, progress, report);
    // Kept whatever the search then does with them: locating a fragmentation
    // point is what the sweep and the validation stage cost, and a later run
    // that starts from these skips both.
    report.fragmentation.clone_from(&broken);
    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Validation,
        findings: carved.len() as u64,
    });
    findings.extend(carved);

    let mut whole_again = HashSet::new();
    if config.stages().reassembly && !broken.is_empty() {
        let reassembled = reassemble_broken(
            control,
            views,
            Reassembling {
                broken: &broken,
                already_recovered: findings,
                budget: config.reassembly_budget(),
                min_long_side: config.min_long_side(),
                volumes,
                medium_len: range_end,
            },
            progress,
            report,
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

    let partials = partial_prefixes(&broken, &whole_again, config.min_long_side());
    report.partial_prefixes = partials.len() as u64;
    findings.extend(partials);
}

/// What the reassembly stage is given to work with.
#[derive(Clone, Copy)]
struct Reassembling<'a> {
    /// Fragmentation points carving localized, in medium order.
    broken: &'a [Broken],
    /// Findings from the earlier stages, whose extents are already accounted
    /// for and must not be offered to the search as free space.
    already_recovered: &'a [Finding],
    /// How long the stage may search, or `None` for every candidate.
    budget: Option<std::time::Duration>,
    /// Smallest long side a frame may declare and still be searched.
    min_long_side: u32,
    /// Volumes the sweep located, whose allocation units are the grid a
    /// fragment boundary really falls on.
    volumes: &'a [Volume],
    /// End of the searchable region.
    medium_len: u64,
}

/// Recovers images the medium stored in pieces.
///
/// The medium is walked in regions, each read once and held while every header
/// inside it is searched: the gap search first, because two fragments with a
/// gap is the dominant real pattern, then the graph walk over the region's
/// classified blocks for whatever the gap search could not complete. Serving
/// the search from memory is what makes a hypothesis cost its own bytes rather
/// than a seek.
///
/// The stage shares one wall-clock budget. Reassembly runs by default, so a
/// medium carrying thousands of fragmentation points must not be able to turn
/// a scan into an overnight job; when the budget runs out the report says so
/// rather than implying the medium held nothing more.
fn reassemble_broken<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    views: &mut [V],
    work: Reassembling<'_>,
    progress: &P,
    report: &mut ScanReport,
) -> Vec<Finding> {
    let Reassembling {
        broken,
        already_recovered,
        budget,
        min_long_side,
        volumes,
        medium_len,
    } = work;
    // Held before the reader is borrowed: a region is memory once loaded, so
    // the search over it wants every core the run was given rather than the one
    // thread that read it.
    let workers = views.len();
    let Some(view) = views.first_mut() else {
        return Vec::new();
    };
    let (broken, plans) = plan_search(broken, min_long_side, medium_len, report);
    let broken = &*broken;
    let deadline = budget.map(|limit| std::time::Instant::now() + limit);
    let spent = || deadline.is_some_and(|at| std::time::Instant::now() >= at);

    // Each header is counted twice — once for the gap search, once for the
    // walk — plus one step per region read, because they are one stage on
    // screen. That total is steps and not headers, and it says so: a reader who
    // takes it for a header count reads a quarter of the queue searched as an
    // eighth, and the manifest's `reassembly_attempted` is the header count
    // (`A-CONFIDENCE-HONEST`). `StageFinished` settles the bar at the end.
    let counter = Counter::start(
        progress,
        Stage::Reassembly,
        (broken.len() as u64)
            .saturating_mul(2)
            .saturating_add(plans.len() as u64),
        Unit::Steps,
    );

    let mut scratch = Scratch::new();
    let mut found = Vec::new();
    // Extents another recovery already accounts for. The search must not offer
    // them again: two artifacts over the same bytes are two reports of one
    // file, and the merge step cannot collapse them because their content
    // hashes differ (A-PROVENANCE). Filesystem metadata counts here too — a
    // run list is a stronger statement about which bytes belong together than
    // anything this stage can derive.
    let mut spoken_for: Vec<ByteRange> = already_recovered
        .iter()
        .flat_map(|finding| finding.extents.iter().copied())
        .collect();
    let mut buffer = Vec::new();

    for plan in plans {
        if control.is_cancelled() {
            return found;
        }
        if spent() {
            report.ceilings.reassembly_decodes = true;
            return found;
        }
        let region = search::Region::load(view, plan.range, buffer);
        counter.step();
        let searched = search_region(
            &region,
            &broken[plan.headers.clone()],
            allocation_grid(volumes, region.range()),
            &spoken_for,
            &mut Searching {
                control,
                counter: &counter,
                spent: &spent,
                scratch: &mut scratch,
                report,
                workers,
            },
        );
        // What this region claimed carries to the next one, which narrows from
        // it again.
        for finding in &searched {
            spoken_for.extend_from_slice(&finding.extents);
        }
        found.extend(searched);

        buffer = region.into_buffer();
    }
    report.ceilings.reassembly_decodes |= spent();
    found
}

/// What one region's search shares with the stage around it.
///
/// Bundled because they travel together and none of them is about the region:
/// the flag that stops the run, the bar it reports on, the clock it is
/// rationed by, the working memory it reuses, and the account it adds to.
struct Searching<'a, P: ?Sized> {
    control: &'a Control,
    counter: &'a Counter<'a, P>,
    spent: &'a (dyn Fn() -> bool + Sync),
    scratch: &'a mut Scratch,
    report: &'a mut ScanReport,
    /// Threads the region's headers are searched across.
    workers: usize,
}

/// Searches `items` across `workers` threads, one result per item in the order
/// the items were given.
///
/// A region is memory by the time this runs, so a worker needs nothing from the
/// medium: it takes its own view of the held bytes and its own working buffers,
/// and what the threads share is the region, the stop flag and the counter.
/// Results come back in item order whatever order they were computed in, so the
/// stage does not depend on how many threads ran it.
///
/// `None` for an item the search never reached, which is how cancelling and a
/// spent budget arrive here.
fn in_parallel<T, R, P>(
    items: &[T],
    region: &search::Region,
    ctx: &Searching<'_, P>,
    search: impl Fn(&mut search::RegionView<'_>, &mut Scratch, &T) -> R + Sync,
) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    P: ProgressSink + ?Sized,
{
    let mut out = Vec::with_capacity(items.len());
    out.resize_with(items.len(), || None);
    if items.is_empty() {
        return out;
    }
    let next = AtomicUsize::new(0);
    let (control, counter, spent) = (ctx.control, ctx.counter, ctx.spent);
    let batches = thread::scope(|scope| {
        let mut workers = Vec::with_capacity(ctx.workers.max(1));
        for _ in 0..ctx.workers.max(1) {
            let (next, search) = (&next, &search);
            workers.push(scope.spawn(move || {
                let mut scratch = Scratch::new();
                let mut mine = Vec::new();
                loop {
                    if control.is_cancelled() || spent() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    mine.push((index, search(&mut region.view(), &mut scratch, item)));
                    counter.step();
                }
                mine
            }));
        }
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Vec<_>>()
    });
    for batch in batches {
        for (index, result) in batch {
            out[index] = Some(result);
        }
    }
    out
}

/// Searches one held region for the images its headers were split into.
///
/// The gap search first, because two fragments with a gap is the dominant real
/// pattern and it is the cheaper question; then the graph walk over the
/// region's classified blocks, for whatever the gap search could not complete.
///
/// `spoken_for` is every extent the run has already accounted for, across the
/// whole medium. It is narrowed to this region once, here: a hypothesis cannot
/// reach past what was held, so a claim that lies outside can never match one,
/// and a search handed all of them would rule out the whole surface once per
/// candidate (`crates/argos_carve/tests/reassemble_scale.rs`).
fn search_region<P: ProgressSink + ?Sized>(
    region: &search::Region,
    headers: &[Broken],
    grid: (u64, u64),
    spoken_for: &[ByteRange],
    ctx: &mut Searching<'_, P>,
) -> Vec<Finding> {
    let range = region.range();
    let limits = reassemble::Limits {
        search_floor: range.start.get(),
        block_bytes: grid.0,
        block_origin: grid.1,
        ..reassemble::Limits::default()
    };
    // Every hypothesis is bounded by what was held, so none can ask for a byte
    // this region does not have.
    let searchable = range.end_saturating().get();
    let mut claimed_here: Vec<ByteRange> = spoken_for
        .iter()
        .copied()
        .filter(|extent| overlaps(*extent, range))
        .collect();

    let mut found = Vec::new();

    // The gap search consults nothing the headers share, so which thread takes
    // which header cannot change an answer: this is the sequential search, run
    // on as many cores as the run was given.
    let gaps = in_parallel(headers, region, ctx, |view, scratch, &candidate| {
        reassemble::bifragment(view, candidate, searchable, limits, scratch)
            .map(|attempt| attempt.reassembly)
    });

    let mut unresolved = Vec::new();
    for (&candidate, outcome) in headers.iter().zip(&gaps) {
        // A header the search never reached is not a header it failed on.
        let Some(outcome) = outcome else { continue };
        ctx.report.reassembly_attempted = ctx.report.reassembly_attempted.saturating_add(1);
        match outcome {
            Ok(Some(reassembly)) => {
                ctx.report.reassembled = ctx.report.reassembled.saturating_add(1);
                claimed_here.extend_from_slice(&reassembly.extents);
                found.push(finding_from_reassembly(candidate, reassembly));
            }
            Ok(None) => unresolved.push(candidate),
            // A candidate that cannot be read is one that cannot be
            // reassembled, and it is already counted as attempted.
            Err(_) => {}
        }
    }

    // The walk does consult the claimed set, so every thread is given the same
    // one — what was claimed when this phase began — and a result is taken only
    // if it still holds once the results before it are in. A walk that a
    // neighbour's recovery invalidated is run again here against everything
    // claimed by then, which is what makes the region's outcome the sequential
    // one for any number of threads. On measured media it is rare enough to
    // cost nothing: five recoveries in three hundred and ninety-nine attempts.
    let claimed_before = claimed_here.clone();
    let walks = in_parallel(&unresolved, region, ctx, |view, scratch, &candidate| {
        reassemble::parallel_unique_path(
            view,
            std::slice::from_ref(&candidate),
            region.blocks(),
            &claimed_before,
            searchable,
            limits,
            scratch,
        )
        .map(|walk| walk.assembled)
    });

    for (&candidate, outcome) in unresolved.iter().zip(&walks) {
        let Some(Ok(assembled)) = outcome else {
            continue;
        };
        let stale = assembled.iter().any(|(_, reassembly)| {
            reassembly
                .extents
                .iter()
                .any(|extent| claimed_here.iter().any(|claim| overlaps(*extent, *claim)))
        });
        let assembled = if stale {
            reassemble::parallel_unique_path(
                &mut region.view(),
                std::slice::from_ref(&candidate),
                region.blocks(),
                &claimed_here,
                searchable,
                limits,
                ctx.scratch,
            )
            .map(|walk| walk.assembled)
            .unwrap_or_default()
        } else {
            assembled.clone()
        };
        for (header, reassembly) in assembled {
            ctx.report.reassembled = ctx.report.reassembled.saturating_add(1);
            claimed_here.extend_from_slice(&reassembly.extents);
            found.push(finding_from_reassembly(header, &reassembly));
        }
    }
    found
}

/// The allocation grid a region's fragments fall on: unit size, and where it
/// is counted from.
///
/// A filesystem allocates in clusters counted from its own start, so those are
/// the only offsets a fragment can begin at. Stepping on them instead of on
/// every 4 KiB is exact — no real boundary is skipped, because every one of
/// them is a cluster boundary — and on a volume of 32 KiB clusters it is eight
/// times fewer hypotheses for the same reach, which the budget spends
/// elsewhere.
///
/// Falls back to the smallest cluster any filesystem here uses when no located
/// volume contains the region, or when two do and disagree: a finer grid tries
/// more than it needs to, while a wrong coarser one would step over the
/// boundary it was looking for.
fn allocation_grid(volumes: &[Volume], region: ByteRange) -> (u64, u64) {
    let default = (reassemble::BLOCK_BYTES, 0);
    let mut found: Option<(u64, u64)> = None;
    for volume in volumes {
        let (start, end) = (
            volume.range.start.get(),
            volume.range.end_saturating().get(),
        );
        if volume.allocation_bytes < reassemble::BLOCK_BYTES
            || !volume
                .allocation_bytes
                .is_multiple_of(reassemble::BLOCK_BYTES)
            || region.start.get() < start
            || region.end_saturating().get() > end
        {
            continue;
        }
        let grid = (volume.allocation_bytes, start);
        match found {
            Some(earlier) if earlier != grid => return default,
            _ => found = Some(grid),
        }
    }
    found.unwrap_or(default)
}

/// Which candidates the search takes, and the order it takes their regions in.
///
/// Two decisions, both made before a byte is read.
///
/// A frame declares its size before its data, so what a candidate claims to be
/// is known for the cost of a header. Searching only photograph-sized frames is
/// what stops a used disk's thumbnail cache — which outnumbers its photographs
/// by two orders of magnitude — from spending the budget: the entries are whole
/// small files, and no reassembly of one could produce anything but the small
/// file it already is (`docs/defects/02-thumbnail-provenance.md`).
///
/// Then regions go in the order of how many units the candidates inside each
/// decoded — MCUs for JPEG, counted absolutely rather than as a share of the
/// frame. A frame the decoder walked thousands of MCUs into is a photograph
/// whose first fragment survived; one it walked three into is a signature that
/// happened to land on plausible bytes. A share does not separate those,
/// because it measures the frame and not the evidence: a cache entry three
/// quarters decoded has walked fewer units than a photograph a tenth decoded,
/// and on a medium whose photographs are the large frames a share orders the
/// budget against them. A format that counts no units contributes zero, so its
/// regions sort last. When the clock runs out, it runs out having spent itself
/// on the first kind.
fn plan_search(
    broken: &[Broken],
    min_long_side: u32,
    medium_len: u64,
    report: &mut ScanReport,
) -> (Vec<Broken>, Vec<search::Plan>) {
    let taken: Vec<Broken> = broken
        .iter()
        .copied()
        .filter(|candidate| candidate.clears(min_long_side))
        .collect();
    report.reassembly_skipped_small = broken
        .len()
        .saturating_sub(taken.len())
        .try_into()
        .unwrap_or(u64::MAX);

    let mut plans: Vec<_> = search::plan_regions(
        &taken,
        medium_len,
        search::REGION_BYTES,
        reassemble::MAX_GAP_BYTES,
    )
    .into_iter()
    .map(|plan| {
        let best = taken[plan.headers.clone()]
            .iter()
            .map(|candidate| candidate.decoded)
            .max()
            .unwrap_or(0);
        (plan, best)
    })
    .collect();
    // Ties keep medium order, so two runs over one medium agree.
    plans.sort_by_key(|(plan, best)| (std::cmp::Reverse(*best), plan.range.start));
    (taken, plans.into_iter().map(|(plan, _)| plan).collect())
}

/// Smallest share of a frame that has to decode before its prefix is reported.
///
/// Below this there is not enough picture to be worth a file: a frame the
/// decoder walked a twentieth of is a few rows at the top and grey beneath. The
/// bytes are still accounted for — the candidate is in the manifest either way
/// — so this decides whether a file is written, not whether the evidence is
/// recorded (`M-DOCUMENTED-MAGIC`).
const MIN_PARTIAL_PROGRESS: f64 = 0.05;

/// Whether the part of `candidate` that decoded is worth reporting as a file.
///
/// The two formats do not measure the same thing, and only one of them can
/// answer this at all.
///
/// A JPEG's entropy decoder accounts for the frame one MCU at a time, so every
/// byte up to [`Broken::decoded_end`] is *proven* to be this image and the
/// share of the frame they draw is known. That is a prefix worth a file.
///
/// A PNG is verified per chunk: a chunk's CRC32 confirms all of it or says
/// nothing about any of it. A file whose tail is gone has a truncated `IDAT`,
/// which cannot verify, so the confirmed prefix stops at the `IHDR` — thirty
/// three bytes, a description of a picture with none of the picture in it.
/// Writing that as a recovery would be writing a header and calling it a
/// photograph.
///
/// This is a property of the format rather than a gap here, and the way past
/// it is not a lower threshold: it is an incremental inflate of the `IDAT`
/// stream, which is self-describing and would confirm payload the way
/// [`crate::pipeline`]'s JPEG counterpart confirms MCUs. Until that exists,
/// a broken PNG is reported as a fragmentation point and nothing more
/// (`A-CONFIDENCE-HONEST`).
fn worth_reporting(candidate: &Broken) -> bool {
    match candidate.format {
        Format::Jpeg => candidate.progress() >= MIN_PARTIAL_PROGRESS,
        Format::Png => false,
    }
}

/// Reports what decoded of the images reassembly could not complete.
///
/// A photograph whose remainder was overwritten is not recoverable, but its
/// beginning is *on the medium* and decodes: a 3072x2304 frame the decoder
/// walked 58% of is the top thirteen hundred rows of the picture, which is the
/// difference between recognising a photograph and having nothing. Before this,
/// such a candidate produced no file at all — only the EXIF thumbnail its
/// header happened to carry, at a size too small to make out.
///
/// The bytes reported are the medium's own, from the header to where the
/// decoder stopped; no `EOI` is appended and nothing is padded, so the digest
/// stays the digest of what was there (`A-PROVENANCE`). What the file is
/// missing is stated rather than hidden: the tier is the weakest there is, and
/// the record carries how much of the frame decoded.
fn partial_prefixes(
    broken: &[Broken],
    whole_again: &HashSet<ByteOffset>,
    min_long_side: u32,
) -> Vec<Finding> {
    broken
        .iter()
        .filter(|candidate| !whole_again.contains(&candidate.header))
        .filter(|candidate| candidate.clears(min_long_side))
        .filter(|candidate| worth_reporting(candidate))
        .filter_map(|candidate| {
            // To the last whole unit, not to where the stream stopped being
            // this file: between the two are the bytes the decoder read on its
            // way to finding out, and they belong to whatever followed on the
            // medium rather than to this picture.
            let length = candidate
                .decoded_end
                .get()
                .checked_sub(candidate.header.get())
                .filter(|length| *length > 0)?;
            Some(Finding {
                format: candidate.format,
                stage: Stage::Carve,
                confidence: Confidence::PartialOrThumbnail,
                extents: Box::from([ByteRange::new(candidate.header, length)]),
                declared_size: None,
                timestamps: argos_core::Timestamps::default(),
                deleted: None,
                name: None,
                source_object: None,
                parent: None,
            })
        })
        .collect()
}

/// Turns a confirmed reassembly into a finding.
///
/// The tier is [`Confidence::Reassembled`], below a contiguous carve: the
/// bytes are the image — the entropy decoder settled that — but which bytes
/// belong together is a reconstruction, and the extent list is what makes it
/// reproducible (A-PROVENANCE).
fn finding_from_reassembly(broken: Broken, reassembly: &reassemble::Reassembly) -> Finding {
    Finding {
        format: broken.format,
        stage: Stage::Reassembly,
        confidence: Confidence::Reassembled,
        extents: reassembly.extents.clone().into_boxed_slice(),
        declared_size: None,
        timestamps: argos_core::Timestamps::default(),
        deleted: None,
        name: None,
        source_object: None,
        parent: None,
    }
}

/// Measures an artifact's picture before the run decides whether to write it.
///
/// The dimensions are the only property of a recovered image that is both
/// cheap to establish and impossible to argue with, and they are what
/// separates a photograph from the thumbnail caches that dominate a used disk.
/// The decode also feeds the manifest, so a reader can tell the two apart
/// without opening a single file.
///
/// A triage label decided this once, and could not: measured against real
/// media its rules call 4128x3096 camera frames ambiguous and 258x258 cache
/// entries photographs. A label that unreliable must not choose what reaches
/// the output directory (`A-TRIAGE-NOT-VERDICT`).
pub(crate) struct Measure {
    /// Scratch for reading an artifact back, reused across findings.
    buf: Vec<u8>,
}

/// What reading an artifact back established about the picture.
#[derive(Clone, Debug, Default)]
pub(crate) struct Measured {
    /// Pixel dimensions, or `None` when the artifact does not decode.
    ///
    /// `None` is not a verdict: an artifact whose picture cannot be measured
    /// is written, because a decoder that gave up is not evidence that the
    /// bytes are worthless.
    pub pixels: Option<(u32, u32)>,
    /// What the picture records about itself and its camera.
    pub capture: argos_core::artifact::Capture,
}

impl Measure {
    /// Reads the artifact back and measures its picture.
    ///
    /// One read serves both answers, and both come from bytes whose digest was
    /// checked against the one recorded at recovery — a description read from
    /// a medium that changed underneath the scan would describe something else.
    fn measure<V: Read + Seek>(
        &mut self,
        view: &mut V,
        finding: &Finding,
        sha256: Digest,
    ) -> Measured {
        let nothing = Measured::default;
        let Ok(length) = usize::try_from(finding.length()) else {
            return nothing();
        };
        if length > argos_carve::decode::MAX_DECODE_BYTES {
            return nothing();
        }
        self.buf.clear();
        self.buf.reserve(length);
        let mut bytes = ExtentReader::new(view, &finding.extents);
        if bytes.read_to_end(&mut self.buf).is_err() || self.buf.len() != length {
            return nothing();
        }
        if Digest::new(Sha256::digest(&*self.buf).into()) != sha256 {
            return nothing();
        }
        Measured {
            pixels: argos_carve::decode::decode_rgba(finding.format, &self.buf)
                .map(|image| (image.width(), image.height())),
            capture: argos_carve::metadata(&self.buf),
        }
    }
}

/// Whether `dimensions` clear the floor this run was given.
///
/// An artifact that could not be measured clears it: the floor exists to keep
/// caches of small pictures out of the directory, not to punish a decoder.
fn clears_floor(dimensions: Option<(u32, u32)>, floor: u32) -> bool {
    dimensions.is_none_or(|(width, height)| width.max(height) >= floor)
}

/// Hashes every finding in medium order and hands the survivors to the sink.
///
/// Sequential on purpose: the order artifacts reach the sink is the order they
/// appear in the manifest, and that order must not depend on how many workers
/// the machine had. Returns what was actually persisted, which is what the
/// triage stage is allowed to see.
fn emit<V, S, P>(
    control: &Control,
    views: &mut [V],
    findings: &[Finding],
    sink: &mut S,
    min_long_side: u32,
    progress: &P,
    report: &mut ScanReport,
) -> Result<Vec<crate::annotate::Emitted>, ScanError>
where
    V: Read + Seek,
    S: ArtifactSink,
    P: ProgressSink + ?Sized,
{
    let mut emitted = Vec::new();
    let Some(view) = views.first_mut() else {
        return Ok(emitted);
    };
    // Cancelling means "stop searching and write what you have". The findings
    // below are everything the earlier stages established, and on a large
    // medium they are hours of reading; a stop aimed at the search must not
    // take them with it. Stopping the writing as well takes a second request,
    // made while the writing is what is running — which is why this is a count
    // read on entry rather than a flag.
    let asked_before = control.stops_requested();
    // The work this stage has to get through: every finding costs a read of
    // its extents whatever becomes of it. This is the denominator of the
    // progress figure, and the numerator counts findings *disposed of* rather
    // than findings stored — a duplicate, an unreadable run and an artifact a
    // caller asked not to write all cost this stage the same read, so counting
    // only the stored ones is what leaves a bar resting short of the end on a
    // run that did everything it could. What was actually stored is a separate
    // figure, reported by `ArtifactStored` and totalled in the manifest.
    let total_work = findings
        .iter()
        .fold(0_u64, |sum, finding| sum.saturating_add(finding.length()));
    progress.emit(ScanEvent::StageStarted {
        stage: Stage::Report,
        unit: Unit::Bytes,
        total: total_work,
    });

    let mut writer = Writing {
        sink,
        measure: Measure { buf: Vec::new() },
        min_long_side,
        seen: HashSet::with_capacity(findings.len()),
        buf: vec![0_u8; STREAM_CHUNK_BYTES],
        stored_bytes: 0,
    };
    let mut done_work = 0_u64;
    for (index, finding) in findings.iter().enumerate() {
        // Between two artifacts, which is the only place this stage can stop
        // without leaving a half-written file: the manifest has to describe
        // files that are whole. A stage that reads no request at all is a stage
        // where the button does nothing, and on a system disk this is the stage
        // a run spends its time in.
        if control.stops_requested() > asked_before {
            break;
        }
        let disposition = writer.dispose(view, finding, index, progress, report)?;

        done_work = done_work.saturating_add(finding.length());
        progress.emit(ScanEvent::StageProgress {
            stage: Stage::Report,
            unit: Unit::Bytes,
            done: done_work,
            total: total_work,
        });
        if let Some(item) = disposition {
            emitted.push(item);
        }
    }

    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Report,
        findings: report.artifacts,
    });
    Ok(emitted)
}

/// What the writing stage carries from one finding to the next.
struct Writing<'a, S> {
    sink: &'a mut S,
    /// What measures each artifact's picture, and the floor it must clear.
    measure: Measure,
    min_long_side: u32,
    /// Digests already stored, so one file recovered twice is stored once.
    seen: HashSet<Digest>,
    /// Working memory for the hashing read, reused by every finding.
    buf: Vec<u8>,
    /// Bytes handed to the sink so far.
    stored_bytes: u64,
}

impl<S: ArtifactSink> Writing<'_, S> {
    /// Settles one finding: stores it, records it unwritten, or drops it.
    ///
    /// `Ok(None)` means nothing was stored, and the counter in `report` says
    /// which of the reasons it was. The error cases are a sink that refused
    /// and a medium that changed underneath the run; both end the scan.
    fn dispose<V, P>(
        &mut self,
        view: &mut V,
        finding: &Finding,
        index: usize,
        progress: &P,
        report: &mut ScanReport,
    ) -> Result<Option<crate::annotate::Emitted>, ScanError>
    where
        V: Read + Seek,
        P: ProgressSink + ?Sized,
    {
        let expected = finding.length();
        let Some((sha256, read)) = hash_extents(view, &finding.extents, &mut self.buf) else {
            // The bytes this finding claims cannot be read back. It is not
            // reported, and nothing is invented in its place.
            report.unrecoverable = report.unrecoverable.saturating_add(1);
            return Ok(None);
        };
        if read != expected {
            report.unrecoverable = report.unrecoverable.saturating_add(1);
            return Ok(None);
        }
        if !self.seen.insert(sha256) {
            report.duplicates = report.duplicates.saturating_add(1);
            return Ok(None);
        }

        let measured = self.measure.measure(view, finding, sha256);
        let artifact = Artifact {
            format: finding.format,
            stage: finding.stage,
            confidence: finding.confidence,
            extents: &finding.extents,
            length: expected,
            expected_length: finding.declared_size,
            sha256,
            timestamps: finding.timestamps,
            deleted: finding.deleted,
            recovered_name: finding.name.as_deref(),
            source_object: finding.source_object,
            parent: finding.parent,
            // The picture is measured before anything is written, because that
            // is the decision below: an image too small to be a photograph
            // stays out of the directory. It is recorded either way, with its
            // dimensions, so the manifest describes the medium whole even when
            // the directory does not, and the extents locate the bytes exactly
            // for a rerun with a lower floor.
            pixels: measured.pixels,
            capture: &measured.capture,
        };
        let dimensions = artifact.pixels;
        if !clears_floor(dimensions, self.min_long_side) {
            self.sink
                .omit(&artifact, "below-size-floor")
                .map_err(ScanError::sink)?;
            report.omitted_assets = report.omitted_assets.saturating_add(1);
            return Ok(None);
        }

        // The sink reads through a hasher, so the digest in the manifest is
        // checked against the bytes the sink actually received rather than
        // against an earlier, separate read of the same extents.
        let mut bytes = Hashing::new(ExtentReader::new(view, &finding.extents));
        self.sink
            .accept(&artifact, &mut bytes)
            .map_err(ScanError::sink)?;
        if bytes.finish() != sha256 {
            // The medium answered differently between the two reads. Nothing
            // recovered from it can be trusted, so the run stops rather than
            // record a hash that does not describe the stored bytes.
            return Err(ScanError::unstable_medium(finding.start()));
        }
        report.artifacts = report.artifacts.saturating_add(1);
        self.stored_bytes = self.stored_bytes.saturating_add(expected);
        progress.emit(ScanEvent::ArtifactStored {
            artifacts: report.artifacts,
            bytes: self.stored_bytes,
        });
        Ok(Some(crate::annotate::Emitted {
            finding: index,
            sha256,
            offset: finding.start().get(),
            pixels: dimensions,
            // Carried from the measure the floor decision already made, so the
            // ordering below costs no second decode and no second read of the
            // metadata.
            evidence: argos_classify::rank::Evidence::measured(dimensions, &measured.capture),
        }))
    }
}

/// Wraps a reader and digests everything that passes through it.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Hashing<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// The digest of everything read so far.
    fn finish(self) -> Digest {
        Digest::new(self.hasher.finalize().into())
    }
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}

/// SHA-256 of the concatenated extents, with the byte count actually read.
/// `None` when the medium refuses part of the range.
fn hash_extents<V: Read + Seek>(
    view: &mut V,
    extents: &[ByteRange],
    buf: &mut [u8],
) -> Option<(Digest, u64)> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut bytes = ExtentReader::new(view, extents);
    loop {
        let read = bytes.read(buf).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        total = total.saturating_add(read as u64);
    }
    Some((Digest::new(hasher.finalize().into()), total))
}

/// Reads a finding's extents back to back as one byte stream.
pub(crate) struct ExtentReader<'a, V> {
    view: &'a mut V,
    extents: &'a [ByteRange],
    /// Extent currently being read.
    index: usize,
    /// Bytes already produced from the current extent.
    consumed: u64,
    /// Whether the view is positioned for the current extent.
    positioned: bool,
}

impl<'a, V: Read + Seek> ExtentReader<'a, V> {
    pub(crate) fn new(view: &'a mut V, extents: &'a [ByteRange]) -> Self {
        Self {
            view,
            extents,
            index: 0,
            consumed: 0,
            positioned: false,
        }
    }
}

impl<V: Read + Seek> Read for ExtentReader<'_, V> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let Some(extent) = self.extents.get(self.index) else {
                return Ok(0);
            };
            let remaining = extent.len.saturating_sub(self.consumed);
            if remaining == 0 {
                self.index += 1;
                self.consumed = 0;
                self.positioned = false;
                continue;
            }
            if !self.positioned {
                let at = extent
                    .start
                    .checked_add(self.consumed)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "extent position overflows the medium's address space",
                        )
                    })?
                    .get();
                self.view.seek(SeekFrom::Start(at))?;
                self.positioned = true;
            }
            let want = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            if want == 0 {
                return Ok(0);
            }
            let read = self.view.read(&mut buf[..want])?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the medium ends inside an extent the metadata claims",
                ));
            }
            self.consumed = self.consumed.saturating_add(read as u64);
            return Ok(read);
        }
    }
}

impl<V: Read + Seek> ExtentReader<'_, V> {
    /// Position in the assembled stream: bytes produced so far.
    fn assembled_position(&self) -> u64 {
        self.extents
            .iter()
            .take(self.index)
            .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
            .saturating_add(self.consumed)
    }

    /// Total length of the assembled stream.
    fn assembled_len(&self) -> u64 {
        self.extents
            .iter()
            .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
    }
}

/// Seeking addresses the **assembled** stream, not the medium: offset zero is
/// the file's first byte wherever that lives. This is what lets a format's
/// state machine, which works in file offsets, validate a fragmented recovery.
impl<V: Read + Seek> Seek for ExtentReader<'_, V> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.assembled_len().checked_add_signed(delta),
            SeekFrom::Current(delta) => self.assembled_position().checked_add_signed(delta),
        };
        let Some(target) = target else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek to a position before the start of the assembled file",
            ));
        };

        // Walk the extent list to the extent holding `target`. Extent counts
        // come from filesystem metadata and stay small; a file with more
        // extents than that fails its own parser long before reaching here.
        let mut remaining = target;
        self.index = self.extents.len();
        self.consumed = 0;
        self.positioned = false;
        for (index, extent) in self.extents.iter().enumerate() {
            if remaining < extent.len {
                self.index = index;
                self.consumed = remaining;
                break;
            }
            remaining = remaining.saturating_sub(extent.len);
        }
        Ok(target)
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        Ok(self.assembled_position())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broken(header: u64, declared: (u32, u32), decoded: u32, required: u32) -> Broken {
        Broken {
            header: ByteOffset::new(header),
            break_at: ByteOffset::new(header + 4096),
            format: Format::Jpeg,
            declared: Some(declared),
            decoded,
            required,
            decoded_end: ByteOffset::new(header + 2048),
        }
    }

    /// The floor decides what is searched at all; the order decides what a
    /// clock reaches. Both these frames clear a 300 px floor, so only the order
    /// separates them: 594 of 625 MCUs is most of a 400x400 derived image,
    /// 2,300 of 23,232 is a tenth of a 2816x2112 photograph. The share prefers
    /// the small frame, the units prefer the photograph, and the photograph is
    /// what a clock should buy.
    #[test]
    fn a_photograph_a_tenth_decoded_is_searched_before_a_small_frame_nearly_through() {
        let cache = broken(1 << 30, (400, 400), 594, 625);
        let photo = broken(8 << 30, (2816, 2112), 2_300, 23_232);
        assert!(
            cache.progress() > photo.progress(),
            "the premise: a share orders these the other way round"
        );

        let mut report = ScanReport::default();
        let (taken, plans) = plan_search(&[cache, photo], 300, 16 << 30, &mut report);

        assert_eq!(taken.len(), 2, "both frames clear the floor");
        assert_eq!(
            taken[plans[0].headers.clone()][0].header,
            photo.header,
            "the region holding the photograph is searched first"
        );
    }

    /// A format whose parser counts no units reports zero decoded, which must
    /// order its regions last rather than tie them with everything else.
    #[test]
    fn a_candidate_that_counts_no_units_sorts_behind_one_that_does() {
        let uncounted = Broken {
            format: Format::Png,
            declared: None,
            decoded: 0,
            required: 0,
            ..broken(1 << 30, (0, 0), 0, 0)
        };
        let counted = broken(8 << 30, (2816, 2112), 12, 23_232);

        let mut report = ScanReport::default();
        let (taken, plans) = plan_search(&[uncounted, counted], 300, 16 << 30, &mut report);

        assert_eq!(
            taken[plans[0].headers.clone()][0].header,
            counted.header,
            "twelve MCUs is still evidence; no count at all is not"
        );
    }
}
