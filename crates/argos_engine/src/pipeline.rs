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
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Reports how far a stage that counts items has got.
///
/// Shared by the workers of a parallel stage, so the count is one number for
/// the stage rather than one per thread.
pub(crate) struct Counter<'a, P: ?Sized> {
    progress: &'a P,
    stage: Stage,
    total: u64,
    /// Items between two events, never zero.
    stride: u64,
    done: AtomicU64,
}

impl<'a, P: ProgressSink + ?Sized> Counter<'a, P> {
    /// Announces the stage and prepares to count `total` items.
    pub(crate) fn start(progress: &'a P, stage: Stage, total: u64) -> Self {
        progress.emit(ScanEvent::StageStarted {
            stage,
            unit: Unit::Items,
            total,
        });
        Self {
            progress,
            stage,
            total,
            stride: total.div_ceil(PROGRESS_STEPS).max(1),
            done: AtomicU64::new(0),
        }
    }

    /// Records one item handled, reporting on stride boundaries and at the end.
    pub(crate) fn step(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if done.is_multiple_of(self.stride) || done == self.total {
            self.progress.emit(ScanEvent::StageProgress {
                stage: self.stage,
                unit: Unit::Items,
                done,
                total: self.total,
            });
        }
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
        let (carved, broken) = validate_candidates(
            control,
            &mut views,
            swept.candidates,
            range_end,
            progress,
            &mut report,
        );
        progress.emit(ScanEvent::StageFinished {
            stage: Stage::Validation,
            findings: carved.len() as u64,
        });
        findings.extend(carved);

        if config.stages().reassembly && !broken.is_empty() {
            let reassembled = reassemble_broken(
                control,
                &mut views,
                &broken,
                range_end,
                progress,
                &mut report,
            );
            progress.emit(ScanEvent::StageFinished {
                stage: Stage::Reassembly,
                findings: reassembled.len() as u64,
            });
            findings.extend(reassembled);
        }
    }

    merge::consolidate(&mut findings, &report.unreadable);
    // The size floor is applied inside the report stage or not at all: whether
    // to write an artifact has to be settled before it is written.
    let emitted = emit(
        control,
        &mut views,
        &findings,
        sink,
        config.min_long_side(),
        progress,
        &mut report,
    )?;

    report.cache_runs = same_size_runs(&emitted);

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
        crate::annotate::run(
            control,
            view,
            &findings,
            &emitted,
            work,
            progress,
            &mut report,
        );
    }
    Ok(report)
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
    let counter = Counter::start(progress, Stage::Filesystem, sweep.volumes.len() as u64);
    for volume in &sweep.volumes {
        if control.is_cancelled() {
            return found;
        }
        found.extend(recover_volume(view, *volume, &mut scratch));
        counter.step();
    }

    // Orphaned `FILE` records store volume-relative cluster numbers, so they
    // can only be resolved against the volume they belong to. A region no
    // located NTFS volume covers is counted, never resolved against a guess.
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
        let Some(volume) = ntfs_volumes
            .iter()
            .find(|volume| covers(volume.range, *region))
        else {
            report.unattributed_residue += 1;
            continue;
        };
        let Ok(Some(geometry)) = argos_fs::ntfs::Ntfs::open(view, volume.range.start) else {
            report.unattributed_residue += 1;
            continue;
        };
        if let Ok(files) =
            argos_fs::ntfs::orphan_scan(view, *region, volume.range.start, geometry.cluster_bytes)
        {
            found.extend(
                files
                    .into_iter()
                    .filter_map(|file| finding_from(view, file, &mut scratch)),
            );
        }
    }
    found
}

fn covers(outer: ByteRange, inner: ByteRange) -> bool {
    let outer_end = outer.end_saturating().get();
    let inner_end = inner.end_saturating().get();
    outer.start <= inner.start && inner_end <= outer_end
}

fn recover_volume<V: Read + Seek>(
    view: &mut V,
    volume: Volume,
    scratch: &mut Scratch,
) -> Vec<Finding> {
    let at = volume.range.start;
    let files = match volume.kind {
        FsKind::Ntfs => argos_fs::ntfs::Ntfs::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_deleted(view).ok()),
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
    let counter = Counter::start(progress, Stage::Validation, candidates.len() as u64);
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

/// Bytes either side of a fragmentation point whose blocks are offered to the
/// graph walk.
///
/// An allocator splits a file because it could not find one run long enough,
/// so the remainder lands in the same region — searching the whole medium for
/// every broken candidate would cost far more and find little. Classifying
/// only this window also keeps the block set small enough to hold
/// (A-BOUNDED-ALLOC).
const REASSEMBLY_WINDOW_BYTES: u64 = 16 * 1024 * 1024;

/// Blocks the graph walk may consider at once.
const MAX_REASSEMBLY_BLOCKS: usize = 1 << 16;

/// Bytes the graph walk may classify before it stops looking.
///
/// This bounds the *work*, which [`MAX_REASSEMBLY_BLOCKS`] does not: that caps
/// the blocks kept, and a medium whose windows hold almost nothing image-like
/// fills it never, so without this the pass reads until the windows run out.
/// Merged windows are read sequentially, so this is a bound in seconds on any
/// medium rather than a bound in blocks that depends on what is on it.
const MAX_CLASSIFY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Bytes read per request while classifying a window.
const CLASSIFY_READ_BYTES: usize = 1024 * 1024;

/// Recovers images the medium stored in pieces.
///
/// Two techniques in order of cost: the gap search first, because two
/// fragments with a gap is the dominant real pattern and it needs no block
/// classification; then the graph walk over classified blocks for whatever the
/// gap search could not complete.
///
/// The whole stage shares one decode budget. Reassembly runs by default, so a
/// medium carrying thousands of false signature hits must not be able to turn
/// a scan into an overnight job; when the budget runs out the report says so
/// rather than implying the medium held nothing more.
fn reassemble_broken<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    views: &mut [V],
    broken: &[Broken],
    medium_len: u64,
    progress: &P,
    report: &mut ScanReport,
) -> Vec<Finding> {
    let Some(view) = views.first_mut() else {
        return Vec::new();
    };
    // The stage's three phases are counted together, because they are one
    // stage on screen. The total is an upper bound — the walk only sees what
    // the gap search left — and `StageFinished` settles the bar at the end. A
    // total that is too large costs a bar that stops short; no total at all
    // costs a window that looks hung, which is what this stage did.
    let runs = merged_windows(broken, medium_len);
    let counter = Counter::start(
        progress,
        Stage::Reassembly,
        (broken.len() as u64)
            .saturating_add(runs.len() as u64)
            .saturating_add(broken.len() as u64),
    );
    let mut scratch = Scratch::new();
    let mut found = Vec::new();
    let mut spent = 0_u32;
    let mut unresolved = Vec::new();
    // Extents the gap search already recovered. The graph walk must not offer
    // them again: two artifacts over the same bytes are two reports of one
    // file, and the merge step cannot collapse them because their content
    // hashes differ (A-PROVENANCE).
    let mut spoken_for: Vec<ByteRange> = Vec::new();

    for &candidate in broken {
        if control.is_cancelled() {
            return found;
        }
        report.reassembly_attempted = report.reassembly_attempted.saturating_add(1);
        if spent >= crate::config::REASSEMBLY_BUDGET {
            report.ceilings.reassembly_decodes = true;
            return found;
        }
        let limits = reassemble::Limits {
            max_hypotheses: reassemble::MAX_HYPOTHESES
                .min(crate::config::REASSEMBLY_BUDGET - spent),
            ..reassemble::Limits::default()
        };
        match reassemble::bifragment(view, candidate, medium_len, limits, &mut scratch) {
            Ok(Some(reassembly)) => {
                spent = spent.saturating_add(reassembly.hypotheses);
                report.reassembled = report.reassembled.saturating_add(1);
                spoken_for.extend_from_slice(&reassembly.extents);
                found.push(finding_from_reassembly(candidate, &reassembly));
            }
            Ok(None) => {
                spent = spent.saturating_add(limits.max_hypotheses);
                unresolved.push(candidate);
            }
            // A candidate we cannot read is one we cannot reassemble.
            Err(_) => {}
        }
        counter.step();
    }

    if unresolved.is_empty() || spent >= crate::config::REASSEMBLY_BUDGET {
        report.ceilings.reassembly_decodes |= spent >= crate::config::REASSEMBLY_BUDGET;
        return found;
    }

    let blocks = classify_windows(control, view, &runs, &counter, report);

    // One header at a time, so the stage can say how far it has got and can be
    // stopped. The extents a completed path claims are carried into the next
    // header's call, which is what keeps two paths from reporting the same
    // bytes as two files (A-PROVENANCE) — the property the walk would
    // otherwise get from holding every header at once.
    for &candidate in &unresolved {
        if control.is_cancelled() {
            return found;
        }
        if spent >= crate::config::REASSEMBLY_BUDGET {
            report.ceilings.reassembly_decodes = true;
            return found;
        }
        let limits = reassemble::Limits {
            max_hypotheses: reassemble::MAX_HYPOTHESES
                .min(crate::config::REASSEMBLY_BUDGET - spent),
            ..reassemble::Limits::default()
        };
        match reassemble::parallel_unique_path(
            view,
            std::slice::from_ref(&candidate),
            &blocks,
            &spoken_for,
            medium_len,
            limits,
            &mut scratch,
        ) {
            Ok(assembled) if !assembled.is_empty() => {
                for (header, reassembly) in assembled {
                    spent = spent.saturating_add(reassembly.hypotheses);
                    report.reassembled = report.reassembled.saturating_add(1);
                    spoken_for.extend_from_slice(&reassembly.extents);
                    found.push(finding_from_reassembly(header, &reassembly));
                }
            }
            // Assembled nothing, or could not be read. The search happened
            // either way, and charging the whole of it is what stops a medium
            // of false headers from spending the budget as if nothing had been
            // tried.
            _ => spent = spent.saturating_add(limits.max_hypotheses),
        }
        counter.step();
    }
    report.ceilings.reassembly_decodes |= spent >= crate::config::REASSEMBLY_BUDGET;
    found
}

/// Classifies the blocks around each fragmentation point.
///
/// The window is read in large sequential pieces and sliced per block, the way
/// the rest of the pipeline reads: a block-at-a-time seek and read would issue
/// thousands of syscalls per candidate, which is the one thing that outweighs
/// every other cost in a scan (`M-THROUGHPUT`).
fn classify_windows<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    view: &mut V,
    runs: &[ByteRange],
    counter: &Counter<'_, P>,
    report: &mut ScanReport,
) -> Vec<reassemble::Candidate> {
    let block = argos_carve::classify::BLOCK_BYTES;
    let mut blocks: Vec<reassemble::Candidate> = Vec::new();
    let mut window = vec![0_u8; CLASSIFY_READ_BYTES];
    let mut budget = MAX_CLASSIFY_BYTES;

    for run in runs {
        if control.is_cancelled() {
            return blocks;
        }
        let to = run.end_saturating().get();
        let mut at = run.start.get();
        while at + block as u64 <= to {
            if blocks.len() >= MAX_REASSEMBLY_BLOCKS || budget == 0 {
                report.ceilings.reassembly_search = true;
                return blocks;
            }
            if control.is_cancelled() {
                return blocks;
            }
            let want = usize::try_from((to - at).min(CLASSIFY_READ_BYTES as u64).min(budget))
                .unwrap_or(CLASSIFY_READ_BYTES);
            let whole = want - (want % block);
            if whole == 0 {
                break;
            }
            budget = budget.saturating_sub(whole as u64);
            if read_exact_at(view, at, &mut window[..whole]).is_err() {
                // Unreadable here; skip the piece rather than the run.
                at = at.saturating_add(whole as u64);
                continue;
            }
            for (index, chunk) in window[..whole].chunks_exact(block).enumerate() {
                let profile = argos_carve::classify::classify(chunk);
                if profile.class.can_hold_image_data() {
                    if blocks.len() >= MAX_REASSEMBLY_BLOCKS {
                        report.ceilings.reassembly_search = true;
                        return blocks;
                    }
                    blocks.push(reassemble::Candidate {
                        start: ByteOffset::new(at.saturating_add((index * block) as u64)),
                        profile,
                    });
                }
            }
            at = at.saturating_add(whole as u64);
        }
        counter.step();
    }
    blocks
}

/// The medium regions the graph walk will look at, merged and in order.
///
/// Fragmentation points cluster: in one scan of a mechanical disk, 203
/// candidates fell inside eight megabytes of each other. Reading a window per
/// candidate therefore reads the same bytes once per candidate — hundreds of
/// gigabytes of seeking to classify a few, which is how this stage came to run
/// for hours and report nothing.
///
/// Merging first makes each block read exactly once, in medium order, which is
/// also what removes the need to remember which blocks have already been seen:
/// the walk cannot revisit one.
fn merged_windows(broken: &[Broken], medium_len: u64) -> Vec<ByteRange> {
    let block = argos_carve::classify::BLOCK_BYTES as u64;
    let mut spans: Vec<(u64, u64)> = broken
        .iter()
        .filter_map(|candidate| {
            let centre = candidate.header.get();
            let from = centre.saturating_sub(REASSEMBLY_WINDOW_BYTES);
            let from = from - (from % block);
            let to = centre
                .saturating_add(REASSEMBLY_WINDOW_BYTES)
                .min(medium_len);
            (to > from + block).then_some((from, to))
        })
        .collect();
    spans.sort_unstable();

    let mut merged: Vec<ByteRange> = Vec::with_capacity(spans.len());
    for (from, to) in spans {
        match merged.last_mut() {
            // Touching or overlapping the previous run: extend it rather than
            // starting another, so the read stays one sequential pass.
            Some(last) if from <= last.end_saturating().get() => {
                let end = last.end_saturating().get().max(to);
                *last = ByteRange::new(last.start, end - last.start.get());
            }
            _ => merged.push(ByteRange::new(ByteOffset::new(from), to - from)),
        }
    }
    merged
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

impl Measure {
    /// The artifact's pixel dimensions, or `None` when it does not decode.
    ///
    /// `None` is not a verdict: an artifact whose picture cannot be measured
    /// is written, because a decoder that gave up is not evidence that the
    /// bytes are worthless.
    fn dimensions<V: Read + Seek>(
        &mut self,
        view: &mut V,
        finding: &Finding,
        sha256: Digest,
    ) -> Option<(u32, u32)> {
        let length = usize::try_from(finding.length()).ok()?;
        if length > argos_carve::decode::MAX_DECODE_BYTES {
            return None;
        }
        self.buf.clear();
        self.buf.reserve(length);
        let mut bytes = ExtentReader::new(view, &finding.extents);
        bytes.read_to_end(&mut self.buf).ok()?;
        if self.buf.len() != length {
            return None;
        }
        if Digest::new(Sha256::digest(&*self.buf).into()) != sha256 {
            return None;
        }
        let image = argos_carve::decode::decode_rgba(finding.format, &self.buf)?;
        Some((image.width(), image.height()))
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
        // without leaving a half-written file: cancelling still writes the
        // manifest, and the manifest has to describe files that are whole.
        // A stage that reads no flag is a stage where the button does nothing,
        // and on a system disk this is the stage a run spends its time in.
        if control.is_cancelled() {
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

        let artifact = Artifact {
            format: finding.format,
            stage: finding.stage,
            confidence: finding.confidence,
            extents: &finding.extents,
            length: expected,
            expected_length: finding.declared_size,
            sha256,
            timestamps: finding.timestamps,
            recovered_name: finding.name.as_deref(),
            source_object: finding.source_object,
            parent: finding.parent,
            // The picture is measured before anything is written, because that
            // is the decision below: an image too small to be a photograph
            // stays out of the directory. It is recorded either way, with its
            // dimensions, so the manifest describes the medium whole even when
            // the directory does not, and the extents locate the bytes exactly
            // for a rerun with a lower floor.
            pixels: self.measure.dimensions(view, finding, sha256),
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
    use argos_core::Format;
    use argos_core::geometry::ByteOffset;

    use super::{Broken, REASSEMBLY_WINDOW_BYTES, merged_windows};

    fn broken_at(offset: u64) -> Broken {
        Broken {
            header: ByteOffset::new(offset),
            break_at: ByteOffset::new(offset + 4096),
            format: Format::Jpeg,
        }
    }

    #[test]
    fn clustered_headers_are_read_once_instead_of_once_each() {
        // The measured shape of a real medium: 203 fragmentation points inside
        // eight megabytes. A window each would read the same bytes 203 times,
        // which is what turned this stage into an overnight job.
        let medium = 1 << 40;
        let base = 100 * 1024 * 1024;
        let broken: Vec<_> = (0..203)
            .map(|index| broken_at(base + index * 40 * 1024))
            .collect();

        let runs = merged_windows(&broken, medium);

        assert_eq!(
            runs.len(),
            1,
            "one cluster is one sequential read: {runs:?}"
        );
        let read: u64 = runs.iter().map(|run| run.len).sum();
        let separate = broken.len() as u64 * 2 * REASSEMBLY_WINDOW_BYTES;
        assert!(
            read * 5 < separate,
            "merging must collapse the work, not restate it: {read} vs {separate}"
        );
    }

    #[test]
    fn headers_further_apart_than_a_window_stay_separate() {
        let medium = 1 << 40;
        let far = 8 * REASSEMBLY_WINDOW_BYTES;
        let runs = merged_windows(&[broken_at(far), broken_at(far * 4)], medium);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(
            runs[0].end_saturating().get() <= runs[1].start.get(),
            "merged runs come out in medium order and do not overlap: {runs:?}"
        );
    }

    #[test]
    fn a_window_never_runs_past_the_medium() {
        let medium = 32 * 1024 * 1024;
        let runs = merged_windows(&[broken_at(medium - 4096)], medium);
        for run in &runs {
            assert!(
                run.end_saturating().get() <= medium,
                "a read past the end is a read that fails: {run:?}"
            );
        }
    }

    #[test]
    fn every_block_appears_in_exactly_one_run() {
        // The property that lets the classification pass drop the set of blocks
        // it has already seen: runs cannot overlap, so it cannot revisit one.
        let medium = 1 << 40;
        let broken: Vec<_> = (0..50)
            .map(|index| broken_at(1_000_000 + index * 3 * 1024 * 1024))
            .collect();
        let runs = merged_windows(&broken, medium);
        for pair in runs.windows(2) {
            assert!(
                pair[0].end_saturating().get() < pair[1].start.get(),
                "overlapping runs would classify the same block twice: {pair:?}"
            );
        }
    }
}
