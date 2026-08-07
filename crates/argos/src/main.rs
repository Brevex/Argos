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
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Scan {
            source,
            out,
            jobs,
            carve_only,
            metadata_only,
        } => scan(
            &source,
            &out,
            Options {
                jobs,
                stages: Stages {
                    filesystem: !carve_only,
                    carving: !metadata_only,
                },
            },
        ),
    }
}

/// What the user asked of one scan, beyond where to read and write.
#[derive(Clone, Copy)]
struct Options {
    jobs: Option<std::num::NonZeroUsize>,
    stages: Stages,
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
            "note      this device may have been TRIMmed; deleted content is often \
             already gone from the host-visible surface"
        );
    }

    let session = ScanSession::new(config);
    let renderer = Renderer::new();
    let controls = progress::spawn_console_controls(session.clone());
    let outcome = session.start(medium, &mut store, &renderer);
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
    let manifest = store
        .finish(argos_report::Summary {
            tool_version: env!("CARGO_PKG_VERSION"),
            source: &description,
            state: &report.map_or_else(|| "failed".to_owned(), |report| report.state.to_string()),
            rejected_candidates: report.map_or(0, |report| report.rejected_candidates),
            unreadable: &unreadable,
        })
        .context("cannot write manifest")?;

    let report = outcome.context("scan failed")?;
    summarize(&report);
    println!("manifest  {}", manifest.display());
    Ok(())
}

fn summarize(report: &ScanReport) {
    println!("state     {}", report.state);
    println!("scanned   {} bytes", report.bytes_swept);
    println!("recovered {} artifacts", report.artifacts);
    println!(
        "rejected  {} candidates that failed validation",
        report.rejected_candidates
    );
    if report.duplicates > 0 {
        println!(
            "duplicate {} artifacts collapsed by content hash",
            report.duplicates
        );
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
