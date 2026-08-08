//! Argos CLI — forensic recovery of deleted images from block devices.
//!
//! Stdout is the user interface of this binary; it is the only place in the
//! workspace where printing is allowed. Sources are only ever opened
//! read-only, and the output directory is refused when it would contain the
//! source.

mod destination;
mod progress;
mod source;

use std::path::{Path, PathBuf};

use anyhow::Context;
use argos_engine::{Medium, ScanConfig, ScanReport, ScanSession, Stages};
use clap::{Parser, Subcommand};
use mimalloc::MiMalloc;

use crate::progress::Renderer;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[command(version, about = "Forensic recovery of deleted images")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Recover images from a raw image file or a block device.
    ///
    /// While a scan runs, `p` pauses it, `r` resumes it and `q` stops it
    /// early, each followed by Enter; results found so far are still written.
    Scan {
        /// Raw image file or block device to scan (opened read-only).
        source: PathBuf,
        /// Directory receiving recovered files and the manifest.
        #[arg(long)]
        out: PathBuf,
        /// Worker threads. Defaults to the machine's available parallelism.
        #[arg(long)]
        jobs: Option<std::num::NonZeroUsize>,
        /// Skip filesystem metadata recovery and carve only.
        #[arg(long)]
        carve_only: bool,
        /// Skip carving and recover from filesystem metadata only.
        #[arg(long, conflicts_with = "carve_only")]
        metadata_only: bool,
        /// Skip fragment reassembly. Reassembly runs by default and recovers
        /// images the medium stored in pieces, at the cost of decoding every
        /// candidate that did not carve whole.
        #[arg(long)]
        no_reassemble: bool,
        /// Skip ML triage. Triage labels recovered images photograph vs
        /// synthetic asset after they are written; it never changes what is
        /// recovered, so disabling it only removes the labels.
        #[arg(long)]
        no_triage: bool,
    },
    /// List the media on this machine that a scan can be pointed at.
    ///
    /// Needs no privileges: it reports what the operating system will say
    /// about a disk without opening it. Whole disks are listed before
    /// partitions, because a recovery scan almost always wants the disk —
    /// a partition cannot see the partition table, the space between
    /// partitions, or the residue an earlier filesystem left behind.
    Devices,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Scan {
            source,
            out,
            jobs,
            carve_only,
            metadata_only,
            no_reassemble,
            no_triage,
        } => scan(
            &source,
            &out,
            Options {
                jobs,
                stages: Stages {
                    filesystem: !carve_only,
                    carving: !metadata_only,
                    reassembly: !metadata_only && !no_reassemble,
                },
                triage: !no_triage,
            },
        ),
        Command::Devices => {
            list_devices();
            Ok(())
        }
    }
}

/// Prints the media this machine exposes, and the shadow copies it holds.
fn list_devices() {
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

/// What the user asked of one scan, beyond where to read and write.
#[derive(Clone, Copy)]
struct Options {
    jobs: Option<std::num::NonZeroUsize>,
    stages: Stages,
    triage: bool,
}

fn scan(source: &Path, out: &Path, options: Options) -> anyhow::Result<()> {
    destination::refuse_writing_onto_source(source, out)?;

    let mut config = ScanConfig::builder().stages(options.stages);
    if let Some(jobs) = options.jobs {
        config = config.workers(jobs);
    }
    let config = config.build().context("invalid scan settings")?;

    let opened = source::open(source, config.workers().get())
        .with_context(|| format!("cannot open {} read-only", source.display()))?;
    let description = opened.describe();
    let medium = Medium::new(opened.views, opened.len).context("cannot read the source")?;

    let mut store = argos_report::Store::create(out)
        .with_context(|| format!("cannot prepare output directory {}", out.display()))?;

    println!("source    {description}");
    println!("workers   {}", config.workers());
    if !opened.expects_content {
        println!(
            "note      this medium reports solid-state storage with TRIM; deleted content is \
             often already gone from the host-visible surface before a scan begins"
        );
    }
    for warning in &opened.warnings {
        println!("warning   {warning}");
    }

    // A model that fails verification disables triage; it never fails the
    // scan, and the reason reaches both stdout and the manifest.
    let mut triage = None;
    let mut triage_disabled = (!options.triage).then(|| "not requested".to_owned());
    if options.triage {
        match argos_classify::Triage::new() {
            Ok(classifier) => triage = Some(classifier),
            Err(err) => {
                println!("triage    disabled: {err}");
                triage_disabled = Some(err.to_string());
            }
        }
    }

    let session = ScanSession::new(config);
    let renderer = Renderer::new();
    let controls = progress::spawn_console_controls(session.clone());
    let outcome = match triage.as_mut() {
        Some(classifier) => {
            session.start_with_classifier(medium, &mut store, &renderer, classifier)
        }
        None => session.start(medium, &mut store, &renderer),
    };
    controls.stop();
    renderer.finish();

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
    }
    let triage_record = report.map(|report| triage_record(report, triage_disabled.as_deref()));
    let manifest = store
        .finish(argos_report::Summary {
            tool_version: env!("CARGO_PKG_VERSION"),
            source: &description,
            state: &report.map_or_else(|| "failed".to_owned(), |report| report.state.to_string()),
            rejected_candidates: report.map_or(0, |report| report.rejected_candidates),
            unreadable: &unreadable,
            triage: triage_record.as_ref(),
        })
        .context("cannot write manifest")?;

    let report = outcome.context("scan failed")?;
    summarize(&report);
    println!("manifest  {}", manifest.display());
    Ok(())
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
            photograph: outcome.score.map(|score| score.photograph),
            scored_by: outcome.score.map(|score| score.scored_by.to_string()),
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
        model_sha256: report.triage_model.map(|model| model.sha256.to_string()),
        scored: report.triage_scored,
        unscored: report.triage_unscored,
        degraded: report.triage_degraded,
    }
}

fn summarize(report: &ScanReport) {
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
    if report.reassembly_budget_exhausted {
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
    if report.unattributed_residue > 0 {
        println!(
            "residue   {} orphaned metadata regions could not be tied to a volume, so \
             their extents were not resolved",
            report.unattributed_residue
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
    if report.detection_truncated {
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
}
