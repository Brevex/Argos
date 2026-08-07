//! Argos CLI — forensic recovery of deleted images from block devices.
//!
//! Stdout is the user interface of this binary; it is the only place in the
//! workspace where printing is allowed. Sources are only ever opened
//! read-only, and the output directory is refused when it would contain the
//! source.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use argos_core::geometry::ByteOffset;
use clap::{Parser, Subcommand};
use mimalloc::MiMalloc;

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
    /// Carve recoverable images out of a raw image file.
    Scan {
        /// Raw image file to scan (opened read-only).
        image: PathBuf,
        /// Directory receiving recovered files and the manifest.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Scan { image, out } => scan(&image, &out),
    }
}

fn scan(image: &Path, out: &Path) -> anyhow::Result<()> {
    let mut src = File::options()
        .read(true)
        .open(image)
        .with_context(|| format!("cannot open {} read-only", image.display()))?;
    let metadata = src
        .metadata()
        .with_context(|| format!("cannot inspect {}", image.display()))?;
    if !metadata.is_file() {
        bail!(
            "{} is not a regular image file; scanning block devices directly is not \
             supported yet — acquire the device to an image first",
            image.display()
        );
    }
    refuse_output_near_source(image, out)?;

    let mut store = argos_report::Store::create(out)
        .with_context(|| format!("cannot prepare output directory {}", out.display()))?;

    let scan = argos_carve::Carver::new()
        .scan(&mut src)
        .context("scan failed")?;

    let mut full = 0_u64;
    let mut thumbnails = 0_u64;
    for (index, finding) in scan.findings.iter().enumerate() {
        let name = format!("{index:06}.{}", finding.format.extension());
        (&src)
            .seek(SeekFrom::Start(finding.offset.get()))
            .with_context(|| format!("cannot seek to byte {}", finding.offset))?;
        let bytes = (&src).take(finding.length);
        store
            .save(
                &name,
                bytes,
                argos_report::Provenance {
                    stage: "carve",
                    format: &finding.format.to_string(),
                    source_offset: finding.offset.get(),
                    length: finding.length,
                    confidence: &finding.confidence.to_string(),
                    parent_offset: finding.parent.map(ByteOffset::get),
                },
            )
            .with_context(|| format!("cannot save artifact {name}"))?;
        if finding.parent.is_some() {
            thumbnails += 1;
        } else {
            full += 1;
        }
    }

    let manifest = store
        .finish(
            env!("CARGO_PKG_VERSION"),
            &image.display().to_string(),
            scan.rejected,
        )
        .context("cannot write manifest")?;

    println!("recovered {full} images and {thumbnails} embedded thumbnails");
    println!("rejected  {} corrupt candidates", scan.rejected);
    println!("manifest  {}", manifest.display());
    Ok(())
}

/// Refuses an output location that could write onto the source (A-READ-ONLY):
/// the output must not contain the source, and — checked before anything is
/// created — the source must not end up inside the output tree.
fn refuse_output_near_source(image: &Path, out: &Path) -> anyhow::Result<()> {
    let image = fs::canonicalize(image)
        .with_context(|| format!("cannot resolve source path {}", image.display()))?;
    // `out` may not exist yet; resolve its deepest existing ancestor.
    let out_resolved = deepest_existing_ancestor(out)?;
    if image.starts_with(&out_resolved) {
        bail!(
            "refusing to write output under {}: it would contain the source {}",
            out.display(),
            image.display()
        );
    }
    Ok(())
}

/// Canonicalizes the deepest ancestor of `path` that already exists, with the
/// missing suffix appended.
fn deepest_existing_ancestor(path: &Path) -> anyhow::Result<PathBuf> {
    let mut existing = path;
    let mut suffix = Vec::new();
    loop {
        if existing.exists() {
            let mut resolved = fs::canonicalize(existing)
                .with_context(|| format!("cannot resolve output path {}", existing.display()))?;
            for part in suffix.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                suffix.push(name.to_owned());
                existing = parent;
            }
            _ => bail!("output path {} has no existing ancestor", path.display()),
        }
    }
}
