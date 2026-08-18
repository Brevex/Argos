//! Argos CLI — forensic recovery of deleted images from block devices.
//!
//! Every capability the tool has is reachable here, headless, before any user
//! interface exposes it (`A-CLI-FIRST`). Sources are only ever opened
//! read-only, and the output directory is refused when it would contain the
//! source.
//!
//! Printing lives in [`console`]; this module defines the commands and
//! dispatches them.

mod acquire;
mod console;
mod destination;
mod export;
mod graft;
mod invoker;
mod progress;
mod scan;
mod scanlog;
mod serve;
mod source;
mod standing;

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
        /// How long fragment reassembly may search, in seconds. Defaults to
        /// two hours; 0 searches every candidate however long it takes.
        ///
        /// Reassembly is the stage that recovers images the medium stored in
        /// pieces, and it is a search: each candidate costs decode attempts
        /// until it finds its remainder or exhausts the region. The budget
        /// bounds the stage, not the scan, and when it runs out the report
        /// says so rather than implying the medium held nothing more.
        #[arg(long, value_name = "SECONDS")]
        reassembly_budget: Option<u64>,
        /// A photograph from the same batch as what is missing, whose header
        /// is lent to fragments that have none.
        ///
        /// After the scan, sweeps for entropy-coded fragments no header
        /// reaches and writes what decodes into a `grafted/` subdirectory.
        /// Those are pixels in a header this tool supplied, not files the
        /// medium held, which is why they land apart from the artifacts and in
        /// no manifest. See the `graft` command.
        #[arg(long, value_name = "IMAGE")]
        reference: Option<PathBuf>,
        /// Render a small preview of every artifact that decodes, into a
        /// `previews/` subdirectory. Previews are derived files, reproducible
        /// from the artifacts, and no part of the recovery depends on them.
        #[arg(long)]
        previews: bool,
        /// Scan only this byte range of the medium, as `START..END` or
        /// `START..` — decimal, or hexadecimal with a `0x` prefix.
        ///
        /// A whole-disk scan is hours; a neighbourhood is minutes. Files
        /// deleted together were usually written together, so the surroundings
        /// of a photograph that *did* come back are where the rest of its batch
        /// is — and a narrow range can afford settings a whole disk cannot,
        /// like `--min-long-side 0` and `--reassembly-budget 0`.
        ///
        /// The range bounds every stage: what lies outside it is not scanned,
        /// and the report counts only what was covered.
        #[arg(long, value_name = "START..END")]
        range: Option<String>,
    },
    /// Copy a medium into a raw image, then scan the image instead of the disk.
    ///
    /// A scan reads the whole surface, and every rerun reads it again. On a
    /// medium that is failing, each pass is one it may not survive, and the
    /// sectors lost are lost for good — so a disk worth recovering from is a
    /// disk worth reading exactly once.
    ///
    /// The sweep skips over failing regions to get the healthy majority off
    /// quickly, then revisits each one sector by sector. Whatever stays
    /// unreadable is zero-filled in the image and listed at the end: those
    /// zeroes are placeholders, never presented as data that was read.
    Acquire {
        /// Block device or image file to copy (opened read-only).
        source: PathBuf,
        /// Path of the raw image to create. Must not already exist, must be an
        /// ordinary file, and must not be on the source's own disk.
        #[arg(long)]
        to: PathBuf,
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
    ///
    /// Artifacts are listed strongest evidence first: the ones naming a camera,
    /// then the ones carrying a capture date, then photograph-sized frames,
    /// and last the entries found among same-sized neighbours — the layout a
    /// thumbnail cache has. A scan of a used disk records hundreds of
    /// thousands of artifacts, so only the head of that list is printed unless
    /// `--all` is given; the manifest holds every one of them either way.
    Report {
        /// Session directory a previous scan wrote.
        session: PathBuf,
        /// Print every artifact rather than the head of the list.
        #[arg(long)]
        all: bool,
    },
    /// Copy artifacts out of a session directory, verifying each hash.
    ///
    /// An artifact whose stored bytes no longer reproduce the digest the scan
    /// recorded is reported and *not* copied.
    /// Search a previous scan's fragmentation points again, without reading
    /// the whole medium a second time.
    ///
    /// A scan of a large disk spends its hours sweeping the surface and driving
    /// every signature hit through a state machine. Both establish the same
    /// fragmentation points every time, and the manifest records them — so
    /// trying a longer budget, a lower size floor or a newer search costs
    /// minutes rather than another overnight run.
    ///
    /// The medium is still read: every extent this reports is fetched back and
    /// hashed exactly as a scan's is.
    Reassemble {
        /// Session directory a previous scan wrote.
        #[arg(long)]
        from: PathBuf,
        /// The same medium that scan read.
        source: PathBuf,
        /// Directory to write the recovered images into.
        #[arg(long)]
        out: PathBuf,
        /// Worker threads. Defaults to the machine's available parallelism.
        #[arg(long, short)]
        jobs: Option<std::num::NonZeroUsize>,
        /// How long the search may run, in seconds. Defaults to two hours;
        /// 0 searches every candidate however long it takes.
        #[arg(long, value_name = "SECONDS")]
        reassembly_budget: Option<u64>,
        /// Smallest long side, in pixels, an image is written to disk for.
        #[arg(long, value_name = "PIXELS")]
        min_long_side: Option<u32>,
        /// Render a small preview of every artifact that decodes.
        #[arg(long)]
        previews: bool,
    },
    /// Recover pixels from JPEG fragments whose header is gone, by lending
    /// them the header of a photograph from the same batch.
    ///
    /// A fragment with no header decodes against nothing: the tables and the
    /// frame geometry live in the header. Point this at one of the photographs
    /// the medium already gave back — same camera, same batch — and it lends
    /// that header to fragments that have none, entering them where a restart
    /// marker resets the decoder.
    ///
    /// What comes out is pixels, not files. The frame size is the reference's,
    /// each strip's position inside it is unknown, and those bytes in that
    /// order never lay on the medium. That is why this is its own command and
    /// its own output directory: a scan's artifacts are files the medium held,
    /// and these are not.
    Graft {
        /// The medium to read.
        source: PathBuf,
        /// A whole baseline JPEG from the same batch as what is missing.
        #[arg(long)]
        reference: PathBuf,
        /// Directory to write the grafted pictures into.
        #[arg(long)]
        out: PathBuf,
        /// Byte range of the medium to sweep, as START..END.
        #[arg(long, value_name = "START..END")]
        range: Option<String>,
    },
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
        /// Export only pictures at least this many pixels on their long side.
        #[arg(long, value_name = "PIXELS")]
        min_long_side: Option<u32>,
        /// Export only pictures standing at least this strongly.
        ///
        /// In increasing order: `cache-neighbour`, `unremarkable`,
        /// `photograph-sized`, `dated`, `camera-named`. `--standing dated`
        /// is the narrow "pictures that say when they were taken" set;
        /// `--standing photograph-sized` is the wide net.
        #[arg(long, value_name = "STANDING")]
        standing: Option<String>,
        /// Export only pictures whose recorded camera make or model contains
        /// this text, matched without regard to case.
        #[arg(long, value_name = "TEXT")]
        camera: Option<String>,
        /// Export only pictures taken at or after this date, as EXIF stores it:
        /// `YYYY:MM:DD HH:MM:SS`, or any prefix — `2009` is that whole year.
        #[arg(long, value_name = "DATE")]
        taken_from: Option<String>,
        /// Export only pictures taken at or before this date, on the same
        /// terms as `--taken-from`.
        #[arg(long, value_name = "DATE")]
        taken_until: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    dispatch(Cli::parse().command)
}

/// Runs one command. Separate from `main` only because the arms outgrew it.
fn dispatch(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Scan {
            source,
            out,
            jobs,
            carve_only,
            metadata_only,
            no_reassemble,
            no_triage,
            min_long_side,
            reassembly_budget,
            previews,
            range,
            reference,
        } => {
            let options = scan::Options {
                reference,
                jobs,
                stages: Stages {
                    filesystem: !carve_only,
                    carving: !metadata_only,
                    reassembly: !metadata_only && !no_reassemble,
                },
                triage: !no_triage,
                min_long_side,
                reassembly_budget: reassembly_budget.map(std::time::Duration::from_secs),
                previews,
                range: range.as_deref().map(scan::parse_range).transpose()?,
                resume_from: None,
            };
            run_scan(&source, &out, &options)
        }
        Command::Acquire { source, to } => run_acquire(&source, &to),
        Command::Serve => {
            serve::run();
            Ok(())
        }
        Command::Devices => {
            console::devices();
            Ok(())
        }
        Command::Report { session, all } => run_report(&session, all),
        Command::Reassemble {
            from,
            source,
            out,
            jobs,
            reassembly_budget,
            min_long_side,
            previews,
        } => run_reassemble(
            &from,
            &source,
            &out,
            jobs,
            reassembly_budget,
            min_long_side,
            previews,
        ),
        Command::Graft {
            source,
            reference,
            out,
            range,
        } => run_graft(&source, &reference, &out, range.as_deref()),
        Command::Export {
            from,
            to,
            hashes,
            min_long_side,
            standing,
            camera,
            taken_from,
            taken_until,
        } => {
            run_export(
                &from,
                &to,
                &export::Filter {
                    hashes,
                    min_long_side,
                    standing: standing.as_deref().map(str::parse).transpose().map_err(
                        |_unknown| {
                            anyhow::anyhow!(
                                "not a standing: expected one of cache-neighbour, unremarkable, \
                             photograph-sized, dated, camera-named"
                            )
                        },
                    )?,
                    camera,
                    taken_from,
                    taken_until,
                },
            )
        }
    }
}

/// Copies a medium into a raw image, stoppable with `q`.
///
/// A pass over a terabyte is hours, and a person who starts one has to be able
/// to end it. What was copied before the stop stays copied, and the report says
/// how much of the medium was never reached.
fn run_acquire(source: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let controls = progress::spawn_stop_control(std::sync::Arc::clone(&stop));
    let outcome = acquire::run(source, to, &console::Console, &|| {
        stop.load(std::sync::atomic::Ordering::Acquire)
    });
    controls.stop();
    outcome
}

fn run_scan(
    source: &std::path::Path,
    out: &std::path::Path,
    options: &scan::Options,
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

/// Searches a previous session's fragmentation points again.
fn run_reassemble(
    from: &std::path::Path,
    source: &std::path::Path,
    out: &std::path::Path,
    jobs: Option<std::num::NonZeroUsize>,
    reassembly_budget: Option<u64>,
    min_long_side: Option<u32>,
    previews: bool,
) -> anyhow::Result<()> {
    let manifest = argos_report::Manifest::read(from)
        .with_context(|| format!("cannot read the session manifest in {}", from.display()))?;
    let broken = scan::fragmentation_points(&manifest);
    anyhow::ensure!(
        !broken.is_empty(),
        "{} records no fragmentation points; it was written by a scan that found none, or by a \
         version of this tool that did not record them",
        from.display()
    );
    println!("resuming  {} fragmentation points", broken.len());
    run_scan(
        source,
        out,
        &scan::Options {
            reference: None,
            jobs,
            // The sweep and the filesystem pass are what these points cost to
            // find; skipping them is the whole point.
            stages: Stages {
                filesystem: false,
                carving: true,
                reassembly: true,
            },
            triage: false,
            min_long_side,
            reassembly_budget: reassembly_budget.map(std::time::Duration::from_secs),
            previews,
            // The points carry their own offsets; a range would only cut them.
            range: None,
            resume_from: Some(broken),
        },
    )
}

/// Prints what a finished session recovered, read back from its manifest.
fn run_report(session: &std::path::Path, all: bool) -> anyhow::Result<()> {
    let manifest = argos_report::Manifest::read(session)
        .with_context(|| format!("cannot read the session manifest in {}", session.display()))?;
    console::manifest(&manifest, all);
    Ok(())
}

/// Lends a surviving photograph's header to fragments that have none.
fn run_graft(
    source: &std::path::Path,
    reference: &std::path::Path,
    out: &std::path::Path,
    range: Option<&str>,
) -> anyhow::Result<()> {
    let reference = graft::reference_from(reference)?;
    let span = match range {
        Some(text) => {
            let (start, end) = scan::parse_range(text)?;
            start..end.unwrap_or(u64::MAX)
        }
        None => 0..u64::MAX,
    };
    let (width, height) = reference.dimensions();
    println!("reference  {width}x{height}");
    let (entered, written) = graft::run(source, out, &reference, span)?;
    println!("entered {entered} orphaned runs, {written} decoded to a picture");
    println!(
        "these are pixels in a header this tool supplied, not files the medium held: the frame \
         size is the reference's and each strip's position inside it is unknown"
    );
    Ok(())
}

fn run_export(
    from: &std::path::Path,
    to: &std::path::Path,
    filter: &export::Filter,
) -> anyhow::Result<()> {
    let exported = export::run(from, to, filter)?;
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
