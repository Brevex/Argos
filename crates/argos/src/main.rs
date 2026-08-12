//! Argos CLI — forensic recovery of deleted images from block devices.
//!
//! Every capability the tool has is reachable here, headless, before any user
//! interface exposes it (`A-CLI-FIRST`). Sources are only ever opened
//! read-only, and the output directory is refused when it would contain the
//! source.
//!
//! Printing lives in [`console`]; this module defines the commands and
//! dispatches them.

mod console;
mod destination;
mod export;
mod invoker;
mod progress;
mod scan;
mod scanlog;
mod serve;
mod source;

use std::path::PathBuf;

use anyhow::Context;
use argos_engine::Stages;
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
        /// Skip triage. Triage labels recovered images photograph vs
        /// synthetic asset; on its own it never changes what is recovered, so
        /// disabling it only removes the labels.
        #[arg(long)]
        no_triage: bool,
        /// Smallest long side, in pixels, an image is written to disk for.
        /// Defaults to 300; 0 writes everything.
        ///
        /// A used disk holds far more derived images than photographs —
        /// icons, avatars and above all the thumbnail caches desktops keep —
        /// and they are small. Everything below the floor is still examined,
        /// hashed and recorded in the manifest with its extents and its
        /// dimensions, so the account of the medium stays complete and a rerun
        /// with a lower floor produces the files; what changes is what lands in
        /// the output directory.
        #[arg(long, value_name = "PIXELS")]
        min_long_side: Option<u32>,
        /// Render a small preview of every artifact that decodes, into a
        /// `previews/` subdirectory. Previews are derived files, reproducible
        /// from the artifacts, and no part of the recovery depends on them.
        #[arg(long)]
        previews: bool,
    },
    /// Run the engine as a JSON-RPC server on stdin/stdout.
    ///
    /// This is how a graphical client reaches the engine: it spawns this —
    /// elevated when the medium needs it — and speaks the wire format in
    /// `argos_ipc`. Every capability here is one this binary already has as a
    /// subcommand, and stdout carries the protocol rather than any output
    /// meant for a person.
    Serve,
    /// List the media on this machine that a scan can be pointed at.
    ///
    /// Needs no privileges: it reports what the operating system will say
    /// about a disk without opening it. Whole disks are listed before
    /// partitions, because a recovery scan almost always wants the disk —
    /// a partition cannot see the partition table, the space between
    /// partitions, or the residue an earlier filesystem left behind.
    Devices,
    /// Print what a finished scan recovered, read back from its manifest.
    Report {
        /// Session directory a previous scan wrote.
        session: PathBuf,
    },
    /// Copy artifacts out of a session directory, verifying each hash.
    ///
    /// An artifact whose stored bytes no longer reproduce the digest the scan
    /// recorded is reported and *not* copied.
    Export {
        /// Session directory a previous scan wrote.
        #[arg(long)]
        from: PathBuf,
        /// Directory to copy into. Created if it does not exist.
        #[arg(long)]
        to: PathBuf,
        /// SHA-256 of an artifact to export, or an unambiguous prefix of it.
        /// Repeatable. Exports everything when omitted.
        #[arg(long = "sha256")]
        hashes: Vec<String>,
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
            no_reassemble,
            no_triage,
            min_long_side,
            previews,
        } => run_scan(
            &source,
            &out,
            scan::Options {
                jobs,
                stages: Stages {
                    filesystem: !carve_only,
                    carving: !metadata_only,
                    reassembly: !metadata_only && !no_reassemble,
                },
                triage: !no_triage,
                min_long_side,
                previews,
            },
        ),
        Command::Serve => {
            serve::run();
            Ok(())
        }
        Command::Devices => {
            console::devices();
            Ok(())
        }
        Command::Report { session } => {
            let manifest = argos_report::Manifest::read(&session).with_context(|| {
                format!("cannot read the session manifest in {}", session.display())
            })?;
            console::manifest(&manifest);
            Ok(())
        }
        Command::Export { from, to, hashes } => run_export(&from, &to, &hashes),
    }
}

fn run_scan(
    source: &std::path::Path,
    out: &std::path::Path,
    options: scan::Options,
) -> anyhow::Result<()> {
    let renderer = Renderer::new();
    let mut controls = None;
    let finished = scan::run(
        source,
        out,
        options,
        &renderer,
        &console::Console,
        |session| controls = Some(progress::spawn_console_controls(session.clone())),
    );
    if let Some(controls) = controls {
        controls.stop();
    }
    renderer.finish();

    let finished = finished?;
    match &finished.report {
        Some(report) => console::summarize(report),
        None => println!("state     failed"),
    }
    println!("manifest  {}", finished.manifest.display());
    // A scan that ran and failed partway still wrote its manifest above; the
    // failure is reported after it, never instead of it.
    anyhow::ensure!(
        finished.report.is_some(),
        "the scan failed; the manifest describes what reached the output directory"
    );
    Ok(())
}

fn run_export(
    from: &std::path::Path,
    to: &std::path::Path,
    hashes: &[String],
) -> anyhow::Result<()> {
    let exported = export::run(from, to, hashes)?;
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
    anyhow::ensure!(
        exported.tampered.is_empty(),
        "{} artifacts changed since the scan and were not exported",
        exported.tampered.len()
    );
    Ok(())
}
