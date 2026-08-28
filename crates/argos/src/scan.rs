//! Driving one scan, without deciding how any of it is shown.
//!
//! Two clients drive scans: the console, whose interface is stdout, and
//! `--serve`, where stdout *is* the JSON-RPC protocol and a stray byte on it
//! corrupts the stream. So nothing here prints. What a scan has to say about
//! itself arrives through [`Notice`] and the
//! [`ProgressSink`](argos_core::ports::ProgressSink); what it produced is
//! the returned [`Finished`].
//!
//! Keeping one driver rather than two is what makes `A-CLI-FIRST` checkable:
//! the UI cannot recover anything the CLI does not, because both reach the
//! engine through this function.
//!
//! Beside the artifacts a run also writes `scan.log`: a scan of a large medium
//! runs for hours in stages a person cannot see into, and a graphical launch
//! has no console to fall back on, so the run keeps its own plain-text account
//! in the session directory. It records the shape of the work and nothing about
//! its content — stages, times, counts and ceilings, never a recovered byte, a
//! recovered name or a path from the medium (`A-NO-CONTENT-IN-LOGS`).
//!
//! Reading a raw device needs administrator privileges, so a scan of one runs
//! as root and everything it writes is created by root. Every way of becoming
//! root leaves a trace of who asked; the run reads whichever is there and hands
//! the output back to that account. Absent all of them the process was root to
//! begin with and there is nobody to hand anything to.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Context;
use argos_core::ByteOffset;
use argos_core::ports::{ProgressSink, ScanEvent};
use argos_engine::graft::JpegReference as Reference;
use argos_engine::{Medium, ScanConfig, ScanReport, ScanSession, Stages};
use argos_report::Owner;

use crate::medium;

/// What the caller asked of one scan, beyond where to read and write.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Worker threads; the machine's available parallelism when absent.
    pub jobs: Option<NonZeroUsize>,
    /// Recovery stages to run.
    pub stages: Stages,
    /// Whether to label artifacts photograph vs synthetic asset.
    pub triage: bool,
    /// Smallest long side, in pixels, an artifact is written for. `None`
    /// takes the engine's default; zero writes everything. Whatever is not
    /// written is recorded either way (`A-TRIAGE-NOT-VERDICT`).
    pub min_long_side: Option<u32>,
    /// How long fragment reassembly may search. `None` takes the engine's
    /// default; a zero duration searches every candidate without a deadline.
    pub reassembly_budget: Option<std::time::Duration>,
    /// Whether to render a preview of every artifact that decodes.
    pub previews: bool,
    /// Byte range of the medium to cover, or `None` for all of it.
    ///
    /// An end of `None` means "to the end of the medium". The range bounds
    /// every stage, so a report of a ranged scan describes that range and not
    /// the medium (`A-CONFIDENCE-HONEST`).
    pub range: Option<(u64, Option<u64>)>,
    /// A photograph from the same batch as what is missing, whose header is
    /// lent to fragments that have none.
    ///
    /// When present, a graft sweep runs after the scan and writes into a
    /// `grafted/` subdirectory of the output. It is kept apart from the scan's
    /// artifacts and out of the manifest on purpose: what it produces is pixels
    /// in a container this tool built, not files the medium held
    /// (`A-CONFIDENCE-HONEST`).
    pub reference: Option<PathBuf>,
    /// Fragmentation points a previous run located, to search again without
    /// sweeping the medium.
    ///
    /// When present the sweep, the filesystem pass and the validation pass are
    /// all skipped: those are what a scan of a large medium spends its hours
    /// on, and they establish these same points every time.
    pub resume_from: Option<Vec<argos_engine::Broken>>,
}

/// What a scan said about itself, other than progress.
///
/// Every method is informational. None of them can stop or change a scan —
/// a client that ignores all of them still gets the same recovery.
pub trait Notice {
    /// The source was opened: how it is described, and with how many workers.
    fn opened(&self, description: &str, workers: usize);

    /// The medium reports solid-state storage with TRIM, so deleted content is
    /// often already gone from the host-visible surface before a scan begins.
    fn reduced_expectation(&self);

    /// Something about the medium the user should know before trusting the
    /// result — a mounted filesystem, or a path that is one partition.
    fn warning(&self, text: &str);
}

/// What a completed scan produced.
#[derive(Debug)]
pub struct Finished {
    /// What the engine found. Absent when the scan itself failed, in which
    /// case a manifest was still written for whatever reached the sink.
    pub report: Option<ScanReport>,
    /// Path of the manifest, which is written whatever happened.
    pub manifest: PathBuf,
}

/// Recovers images from `source` into `out`.
///
/// `on_session` is called once, with the session, before the scan starts
/// blocking; that is the only window in which a caller can arrange to pause or
/// cancel it.
///
/// # Errors
///
/// Fails when the output directory would contain the source, when the source
/// cannot be opened read-only, when the settings do not validate, when the
/// output directory cannot be prepared, or when the manifest cannot be
/// written. A scan that *ran* and failed partway still writes its manifest and
/// reports the failure through [`Finished::report`] being absent.
pub fn run<P, N>(
    source: &Path,
    out: &Path,
    options: &Options,
    progress: &P,
    notice: &N,
    on_session: impl FnOnce(&ScanSession),
) -> anyhow::Result<Finished>
where
    P: ProgressSink + ?Sized,
    N: Notice + ?Sized,
{
    medium::refuse_writing_onto_source(source, out)?;

    let mut config = ScanConfig::builder()
        .stages(options.stages)
        .min_long_side(
            options
                .min_long_side
                .unwrap_or(argos_engine::DEFAULT_MIN_LONG_SIDE),
        )
        .reassembly_budget(match options.reassembly_budget {
            // An explicit zero is "no deadline", which is the only way to ask
            // for a search that stops when it runs out of candidates.
            Some(budget) if budget.is_zero() => None,
            Some(budget) => Some(budget),
            None => Some(argos_engine::DEFAULT_REASSEMBLY_BUDGET),
        })
        .previews(options.previews);
    if let Some(jobs) = options.jobs {
        config = config.workers(jobs);
    }
    if let Some((start, end)) = options.range {
        let start = ByteOffset::new(start);
        config = match end {
            Some(end) => config.range(start..ByteOffset::new(end)),
            None => config.range(start..),
        };
    }
    let config = config.build().context("invalid scan settings")?;

    let opened = medium::open(source, config.workers().get())
        .with_context(|| format!("cannot open {} read-only", source.display()))?;
    let description = opened.describe();
    let medium = Medium::new(opened.views, opened.len).context("cannot read the source")?;

    let owner = owner();
    let mut store = argos_report::Store::create(out, owner)
        .with_context(|| format!("cannot prepare output directory {}", out.display()))?;
    // Said before the scan rather than after it: a person who learns at the
    // end of a four-hour run that the results belong to root learns it too
    // late to choose a different destination.
    if let argos_report::Handback::Refused(reason) = store.handback() {
        notice.warning(reason);
    }

    // The run's own account, next to what it recovers. A scan that has to be
    // killed leaves nothing else behind to say where it was.
    let log = ScanLog::create(out, owner)
        .with_context(|| format!("cannot open the scan log in {}", out.display()))?;
    log.line(&format!(
        "source         {description}, {} workers",
        config.workers()
    ));
    let progress = &Tee {
        inner: progress,
        log: &log,
    };

    notice.opened(&description, config.workers().get());
    if !opened.expects_content {
        notice.reduced_expectation();
    }
    for warning in &opened.warnings {
        notice.warning(warning);
    }

    // A model that fails verification disables triage; it never fails the
    // scan, and the reason reaches both the client and the manifest.
    let mut triage = options.triage.then(argos_classify::Triage::new);
    let triage_disabled = (!options.triage).then(|| "not requested".to_owned());

    let session = ScanSession::new(config);
    on_session(&session);
    let outcome = match (&options.resume_from, triage.as_mut()) {
        (Some(broken), _) => session.reassemble(medium, broken, &mut store, progress),
        (None, Some(classifier)) => {
            session.start_with_classifier(medium, &mut store, progress, classifier)
        }
        (None, None) => session.start(medium, &mut store, progress),
    };

    let report = outcome.as_ref().ok();
    let manifest = write_manifest(store, report, &description, triage_disabled.as_deref())?;
    match report {
        Some(report) => log.summary(report),
        None => log.line("scan failed before it could report"),
    }

    if let Some(reference) = options.reference.as_deref() {
        graft_after(source, out, reference, options.range, notice);
    }

    Ok(Finished {
        report: outcome.ok(),
        manifest,
    })
}

/// Sweeps for headerless fragments once the scan proper is done.
///
/// Its output goes to `grafted/` rather than beside the artifacts, and into no
/// manifest: a graft is pixels inside a header this tool supplied, and a
/// directory where those and recovered files look alike is a report that leads
/// to a wrong conclusion (`A-CONFIDENCE-HONEST`).
///
/// A failure here does not fail the scan. Everything the scan recovered is
/// already written, and losing it to a fault in an extra pass would be the
/// worse outcome.
fn graft_after<N: Notice + ?Sized>(
    source: &Path,
    out: &Path,
    reference: &Path,
    range: Option<(u64, Option<u64>)>,
    notice: &N,
) {
    let span = range.map_or(0..u64::MAX, |(start, end)| start..end.unwrap_or(u64::MAX));
    let outcome = reference_from(reference)
        .and_then(|reference| graft(source, &out.join("grafted"), &reference, span));
    match outcome {
        Ok((entered, written)) => notice.warning(&format!(
            "grafted {written} of {entered} headerless fragments into grafted/: pixels in a header this tool supplied, not files the medium held"
        )),
        Err(error) => notice.warning(&format!("the graft sweep did not run: {error}")),
    }
}

/// Annotates what was stored and writes the manifest describing it.
///
/// Written whatever happened, including for a run that failed partway:
/// artifacts already on disk without one would be bytes nothing can attribute
/// to a sector, which is the situation provenance exists to prevent
/// (`A-PROVENANCE`).
fn write_manifest(
    store: argos_report::Store,
    report: Option<&ScanReport>,
    description: &str,
    triage_disabled: Option<&str>,
) -> anyhow::Result<std::path::PathBuf> {
    let mut store = store;
    let unreadable: Vec<argos_report::ExtentRecord> = report
        .map(|report| {
            report
                .unreadable
                .iter()
                .map(|range| argos_report::ExtentRecord {
                    offset: range.start.get(),
                    length: range.len,
                })
                .collect()
        })
        .unwrap_or_default();
    // Triage annotations join the records by content hash. They add labels to
    // artifacts already written; nothing here can remove one
    // (A-TRIAGE-NOT-VERDICT).
    if let Some(report) = report {
        store.annotate_triage(&annotations(report));
        let runs: Vec<(String, u32)> = report
            .cache_runs
            .iter()
            .map(|run| (run.sha256.to_string(), run.neighbours))
            .collect();
        store.annotate_same_size_runs(&runs);
        // The sort key, so a reader of this directory can put the photographs
        // first without deriving anything itself.
        let standings: Vec<(String, String)> = report
            .standings
            .iter()
            .map(|(sha256, standing)| (sha256.to_string(), standing.to_string()))
            .collect();
        store.annotate_standings(&standings);
    }
    let triage_record = report.map(|report| triage_record(report, triage_disabled));
    let coverage = report.map(coverage);
    let volumes = report.map(volumes).unwrap_or_default();
    let fragmentation = report.map(fragmentation).unwrap_or_default();
    let lost = report.map(lost_files).unwrap_or_default();
    store
        .finish(argos_report::Summary {
            tool_version: env!("CARGO_PKG_VERSION"),
            source: description,
            state: &report.map_or_else(|| "failed".to_owned(), |report| report.state.to_string()),
            rejected_candidates: report.map_or(0, |report| report.rejected_candidates),
            unreadable: &unreadable,
            triage: triage_record.as_ref(),
            coverage: coverage.as_ref(),
            volumes: &volumes,
            fragmentation: &fragmentation,
            lost_files: &lost,
        })
        .context("cannot write manifest")
}

/// Parses a `START..END` byte range, decimal or `0x`-prefixed hexadecimal.
///
/// `START..` runs to the end of the medium. Underscores are allowed as digit
/// separators, because these are disk offsets and a person types them from a
/// report: `53_350_498_304` is the offset a photograph was found at.
///
/// # Errors
///
/// Fails when the text is not a range, when a bound is not a number, or when
/// the range covers nothing. A misparsed range would scan the wrong part of a
/// medium and report the result as the medium's, so it is refused rather than
/// interpreted generously.
pub fn parse_range(text: &str) -> anyhow::Result<(u64, Option<u64>)> {
    let (start, end) = text
        .split_once("..")
        .with_context(|| format!("{text} is not a range: write it as START..END or START.."))?;
    let number = |part: &str| -> anyhow::Result<u64> {
        let cleaned = part.trim().replace('_', "");
        let parsed = match cleaned
            .strip_prefix("0x")
            .or_else(|| cleaned.strip_prefix("0X"))
        {
            Some(hex) => u64::from_str_radix(hex, 16),
            None => cleaned.parse::<u64>(),
        };
        parsed.with_context(|| format!("{part} is not a byte offset"))
    };

    let start = if start.trim().is_empty() {
        0
    } else {
        number(start)?
    };
    let end = if end.trim().is_empty() {
        None
    } else {
        Some(number(end)?)
    };
    if let Some(end) = end {
        anyhow::ensure!(end > start, "the range {text} covers no bytes");
    }
    Ok((start, end))
}

/// Reads a manifest's fragmentation points back into what the search takes.
///
/// A record whose format this tool does not recover, or whose break point does
/// not lie past its header, is skipped rather than guessed at: the search would
/// have nothing to start from either way.
///
#[must_use]
pub fn fragmentation_points(manifest: &argos_report::Manifest) -> Vec<argos_engine::Broken> {
    manifest
        .fragmentation
        .iter()
        .filter_map(|record| {
            let format = record.format.parse().ok()?;
            (record.break_at > record.offset).then_some(argos_engine::Broken {
                header: ByteOffset::new(record.offset),
                break_at: ByteOffset::new(record.break_at),
                format,
                declared: record.declared_width.zip(record.declared_height),
                decoded: record.decoded,
                required: record.required,
                decoded_end: ByteOffset::new(record.decoded_end.max(record.offset)),
            })
        })
        .collect()
}

/// Turns the engine's fragmentation points into manifest records.
///
/// They are what a later `argos reassemble` starts from, so they are written
/// whether or not the search that follows them found anything: a point the
/// budget never reached is exactly the one worth trying again.
fn fragmentation(report: &argos_engine::ScanReport) -> Vec<argos_report::FragmentRecord> {
    report
        .fragmentation
        .iter()
        .map(|broken| argos_report::FragmentRecord {
            offset: broken.header.get(),
            break_at: broken.break_at.get(),
            decoded_end: broken.decoded_end.get(),
            format: broken.format.to_string(),
            declared_width: broken.declared.map(|(width, _)| width),
            declared_height: broken.declared.map(|(_, height)| height),
            decoded: broken.decoded,
            required: broken.required,
        })
        .collect()
}

/// Turns records of files the run could not place into manifest records.
///
/// Kept out of `artifacts` on purpose: nothing here was read from the medium.
/// A name, a size and two timestamps are what a `FILE` record states about
/// itself, and they survive the loss of the volume that would locate its
/// content — so they are the last evidence that a particular file existed
/// (`A-CONFIDENCE-HONEST`).
fn lost_files(report: &ScanReport) -> Vec<argos_report::LostFileRecord> {
    report
        .lost_files
        .iter()
        .map(|lost| argos_report::LostFileRecord {
            name: lost.name.clone(),
            size: lost.size,
            record_at: lost.record_at,
            created_unix: lost.timestamps.created.map(argos_report::unix_seconds),
            modified_unix: lost.timestamps.modified.map(argos_report::unix_seconds),
            record_number: lost.record_number,
            first_cluster: lost.first_lcn(),
            clusters: lost.clusters(),
            runs: lost
                .runs
                .iter()
                .map(|run| argos_report::RunRecord {
                    lcn: run.lcn,
                    clusters: run.clusters,
                })
                .collect(),
        })
        .collect()
}

/// Seconds in the window a batch of records is grouped into.
///
/// Files written — or removed — in one action share a moment; a minute is wide
/// enough to hold the action and narrow enough that ordinary use does not fill
/// it (`docs/OPEN-WORK.md` §1.4 records batches of a hundred frames).
const BATCH_WINDOW_SECS: i64 = 60;

/// Turns the census the passes over orphaned records kept into a manifest
/// record, adding the two figures only the collected records can give.
///
/// The instants come from the records the run could not place, which is the
/// population the question is about: whether a heap of files stopped existing
/// in one action. It is a count of records per window and never a timestamp of
/// any one file.
fn residue_census(report: &ScanReport) -> argos_report::ResidueCensusRecord {
    let census = report.orphan_census;
    let mut instants: Vec<i64> = report
        .lost_files
        .iter()
        .filter_map(|lost| lost.timestamps.created)
        .map(argos_report::unix_seconds)
        .collect();
    instants.sort_unstable();

    let distinct = instants
        .windows(2)
        .filter(|pair| pair[0] != pair[1])
        .count()
        + usize::from(!instants.is_empty());
    // The widest run of instants inside one window, by sliding the window's
    // start over the sorted instants.
    let mut largest = 0_usize;
    let mut low = 0_usize;
    for high in 0..instants.len() {
        while instants[high].saturating_sub(instants[low]) >= BATCH_WINDOW_SECS {
            low += 1;
        }
        largest = largest.max(high - low + 1);
    }

    argos_report::ResidueCensusRecord {
        seen: census.seen,
        valid: census.valid,
        in_use: census.in_use,
        resident: census.resident,
        non_resident: census.non_resident,
        image_named: census.image_named,
        runs: census.runs.to_vec(),
        distinct_instants: distinct as u64,
        largest_minute: largest as u64,
    }
}

/// Turns the engine's own account of the run into a manifest record.
///
/// These are the figures that separate "the medium held nothing more" from
/// "the run did not look there": what was recognised and deliberately not
/// written, what the search skipped, what damage cost, and which ceilings were
/// reached. The console prints them and the scan log keeps them, but neither
/// survives into a session directory a later run — or a person — has to read
/// (A-CONFIDENCE-HONEST).
fn coverage(report: &ScanReport) -> argos_report::CoverageRecord {
    argos_report::CoverageRecord {
        bytes_swept: report.bytes_swept,
        duplicates: report.duplicates,
        unrecoverable: report.unrecoverable,
        dropped_unreadable: report.dropped_unreadable,
        omitted_assets: report.omitted_assets,
        partial_prefixes: report.partial_prefixes,
        reassembly_attempted: report.reassembly_attempted,
        reassembled: report.reassembled,
        reassembly_skipped_small: report.reassembly_skipped_small,
        journal_deletions: report.journal_deletions,
        unattributed_residue: report.unattributed_residue,
        attributed_residue: report.attributed_residue,
        metadata_unconfirmed: report.metadata_unconfirmed,
        residue_census: residue_census(report),
        ceilings: report.ceilings.reached().map(str::to_owned).collect(),
    }
}

/// Turns the volumes the sweep located into manifest records.
///
/// Residual volumes are the point: a medium re-formatted more than once
/// carries the anchors of what came before, and which of them were found is
/// what decides whether the metadata of the filesystem that held a deleted
/// file could be read at all.
fn volumes(report: &ScanReport) -> Vec<argos_report::VolumeRecord> {
    report
        .volumes
        .iter()
        .map(|volume| argos_report::VolumeRecord {
            kind: volume.kind.to_string(),
            origin: volume.origin.to_string(),
            offset: volume.range.start.get(),
            length: volume.range.len,
            allocation_bytes: volume.allocation_bytes,
        })
        .collect()
}

/// Turns the engine's triage outcomes into manifest annotations.
fn annotations(report: &ScanReport) -> Vec<argos_report::TriageAnnotation> {
    report
        .triage
        .iter()
        .map(|outcome| argos_report::TriageAnnotation {
            sha256: outcome.sha256.to_string(),
            perceptual_hash: outcome.perceptual_hash.map(|hash| format!("{hash:016x}")),
            near_duplicate_of: outcome.near_duplicate_of.map(|of| of.to_string()),
            label: outcome.score.map(|score| score.label.to_string()),
            decided_by: outcome.score.map(|score| score.decided_by.to_string()),
        })
        .collect()
}

/// States how triage ran, including when it did not.
fn triage_record(report: &ScanReport, disabled: Option<&str>) -> argos_report::TriageRecord {
    argos_report::TriageRecord {
        status: if report.triage_model.is_some() {
            "scored".to_owned()
        } else {
            "disabled".to_owned()
        },
        disabled_reason: disabled.map(str::to_owned),
        model_version: report.triage_model.map(|model| model.version.to_owned()),
        scored: report.triage_scored,
        unscored: report.triage_unscored,
        degraded: report.triage_degraded,
    }
}

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
            "unreadable     {} regions, {} findings dropped for overlapping one",
            report.unreadable.len(),
            report.dropped_unreadable
        ));
        self.line(&format!(
            "not written    {} under the size floor, {} partial prefixes reported, {} \
             fragmented candidates skipped as too small",
            report.omitted_assets, report.partial_prefixes, report.reassembly_skipped_small
        ));
        // The two figures that say whether the metadata of an earlier
        // filesystem could be read at all. A residual anchor is the trace of a
        // format that came before; an unattributed region is a run list that
        // survived one and could not be resolved for want of the geometry it is
        // counted against.
        let residual = report
            .volumes
            .iter()
            .filter(|volume| volume.origin == argos_engine::Origin::Residual)
            .count();
        self.line(&format!(
            "volumes        {} located, {residual} left by earlier formats",
            report.volumes.len()
        ));
        self.line(&format!(
            "residue        {} orphaned metadata regions could not be tied to a volume, \
             {} were",
            report.unattributed_residue, report.attributed_residue
        ));
        // The denominator: what those regions held, so the recovery count above
        // can be read against something (A-CONFIDENCE-HONEST).
        let census = report.orphan_census;
        self.line(&format!(
            "records        {} read, {} intact, {} in use, {} resident, {} with a run list, \
             {} named an image",
            census.seen,
            census.valid,
            census.in_use,
            census.resident,
            census.non_resident,
            census.image_named
        ));
        self.line(&format!(
            "runs           per run list, bucketed 1/2/3/4/5-8/9-16/17-64/65+: {:?}",
            census.runs
        ));
        self.line(&format!(
            "unconfirmed    {} metadata claims pointed at bytes that are not the file \
             they name",
            report.metadata_unconfirmed
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

/// Set by the Argos shell when it starts an engine on someone's behalf.
const SHELL_UID: &str = "ARGOS_INVOKER_UID";
/// Group counterpart of [`SHELL_UID`].
const SHELL_GID: &str = "ARGOS_INVOKER_GID";

/// The account to give recovered files to, if this process is acting for one.
#[must_use]
pub fn owner() -> Option<Owner> {
    // In order of how much they know. The shell passes both parts; `sudo`
    // publishes both; `pkexec` publishes only the user, and leaving the group
    // alone is better than inventing one.
    for (uid, gid) in [(SHELL_UID, Some(SHELL_GID)), ("SUDO_UID", Some("SUDO_GID"))] {
        if let Some(uid) = read(uid) {
            return Some(Owner::new(uid, gid.and_then(read)));
        }
    }
    read("PKEXEC_UID").map(|uid| Owner::new(uid, None))
}

/// One environment variable as a user or group id.
fn read(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

/// Reads `path` as the header its lost siblings were written with.
pub fn reference_from(path: &Path) -> anyhow::Result<Reference> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read the reference {}", path.display()))?;
    Reference::read(&bytes)
        .map_err(|error| anyhow::anyhow!("{} {error}", path.display()))
        .context(
            "a reference must be a whole baseline or extended sequential JPEG — one of the \
             photographs this medium already gave back, from the same camera as what is missing",
        )
}

/// Sweeps `range` of `src`, writing every graft that decoded into `out`.
pub fn graft(
    src: &Path,
    out: &Path,
    reference: &Reference,
    range: std::ops::Range<u64>,
) -> anyhow::Result<(usize, usize)> {
    crate::medium::refuse_writing_onto_source(src, out)?;
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;

    let mut opened = medium::open(src, 1)?;
    for warning in &opened.warnings {
        eprintln!("warning: {warning}");
    }
    let end = range.end.min(opened.len);
    let mut view = opened
        .views
        .pop()
        .context("opening the medium yielded no view")?;

    let mut written = 0_usize;
    let mut failed = Ok(());
    let entered = argos_engine::graft::sweep(&mut view, range.start..end, reference, |grafted| {
        if failed.is_err() {
            return;
        }
        let name = format!("grafted-{:016x}.jpg", grafted.at.get());
        match std::fs::write(out.join(&name), &grafted.bytes) {
            Ok(()) => {
                written += 1;
                println!(
                    "  {name}  {}x{} entered at medium offset {}",
                    grafted.dimensions.0,
                    grafted.dimensions.1,
                    grafted.at.get()
                );
            }
            Err(error) => failed = Err(anyhow::anyhow!("cannot write {name}: {error}")),
        }
    });
    failed?;
    Ok((entered, written))
}
