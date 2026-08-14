//! Driving one scan, without deciding how any of it is shown.
//!
//! Two clients drive scans: the console, whose interface is stdout, and
//! `--serve`, where stdout *is* the JSON-RPC protocol and a stray byte on it
//! corrupts the stream. So nothing here prints. What a scan has to say about
//! itself arrives through [`Notice`] and the
//! [`ProgressSink`](argos_core::progress::ProgressSink); what it produced is
//! the returned [`Finished`].
//!
//! Keeping one driver rather than two is what makes `A-CLI-FIRST` checkable:
//! the UI cannot recover anything the CLI does not, because both reach the
//! engine through this function.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::Context;
use argos_core::progress::ProgressSink;
use argos_engine::{Medium, ScanConfig, ScanReport, ScanSession, Stages};

use crate::{destination, source};

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
    destination::refuse_writing_onto_source(source, out)?;

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
    let config = config.build().context("invalid scan settings")?;

    let opened = source::open(source, config.workers().get())
        .with_context(|| format!("cannot open {} read-only", source.display()))?;
    let description = opened.describe();
    let medium = Medium::new(opened.views, opened.len).context("cannot read the source")?;

    let owner = crate::invoker::owner();
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
    let log = crate::scanlog::ScanLog::create(out, owner)
        .with_context(|| format!("cannot open the scan log in {}", out.display()))?;
    log.line(&format!(
        "source         {description}, {} workers",
        config.workers()
    ));
    let progress = &crate::scanlog::Tee {
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

    // The manifest is written whatever happened. Artifacts already on disk
    // without one would be bytes nothing can attribute to a sector, which is
    // the situation provenance exists to prevent.
    let report = outcome.as_ref().ok();
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
    }
    let triage_record = report.map(|report| triage_record(report, triage_disabled.as_deref()));
    let fragmentation = report.map(fragmentation).unwrap_or_default();
    let manifest = store
        .finish(argos_report::Summary {
            tool_version: env!("CARGO_PKG_VERSION"),
            source: &description,
            state: &report.map_or_else(|| "failed".to_owned(), |report| report.state.to_string()),
            rejected_candidates: report.map_or(0, |report| report.rejected_candidates),
            unreadable: &unreadable,
            triage: triage_record.as_ref(),
            fragmentation: &fragmentation,
        })
        .context("cannot write manifest")?;
    match report {
        Some(report) => log.summary(report),
        None => log.line("scan failed before it could report"),
    }

    Ok(Finished {
        report: outcome.ok(),
        manifest,
    })
}

/// Reads a manifest's fragmentation points back into what the search takes.
///
/// A record whose format this tool does not recover, or whose break point does
/// not lie past its header, is skipped rather than guessed at: the search would
/// have nothing to start from either way.
///
#[must_use]
pub fn fragmentation_points(manifest: &argos_report::Manifest) -> Vec<argos_engine::Broken> {
    use argos_core::geometry::ByteOffset;
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
