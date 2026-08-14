//! Stdout: the console's user interface, and the only place printing happens.
//!
//! Everything that reaches a terminal is written here. The scan driver, the
//! engine and every library crate stay silent and hand back structured values
//! (`M-LOG-NOT-PRINT`); this module decides what a person sees.

use argos_engine::ScanReport;
use argos_report::Manifest;

use crate::scan::Notice;

/// Prints what a scan says about itself as it says it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Console;

impl Notice for Console {
    fn opened(&self, description: &str, workers: usize) {
        println!("source    {description}");
        println!("workers   {workers}");
    }

    fn reduced_expectation(&self) {
        println!(
            "note      this medium reports solid-state storage with TRIM; deleted content is \
             often already gone from the host-visible surface before a scan begins"
        );
    }

    fn warning(&self, text: &str) {
        println!("warning   {text}");
    }
}

impl crate::acquire::Notice for Console {
    fn progress(&self, progress: argos_device::acquire::Progress) {
        use argos_device::acquire::Progress::{Refined, Swept};
        match progress {
            Swept { done, total } => {
                println!("sweep     {done} of {total} sectors");
            }
            Refined { done, total } => {
                println!("refine    {done} of {total} sectors the sweep could not read");
            }
        }
    }

    fn finished(&self, report: &argos_device::acquire::Report) {
        println!(
            "acquired  {} of {} sectors",
            report.recovered_sectors(),
            report.sector_count()
        );
        if report.is_complete() {
            println!("image     every sector was read");
            return;
        }
        // Named individually, because these are the ranges where the image
        // holds zeroes that were never on the medium. A scan of the image
        // reports them as unreadable, and nothing is ever recovered from them
        // (A-CONFIDENCE-HONEST).
        println!(
            "damage    {} unreadable runs are zero-filled placeholders in the image, never \
             read data",
            report.unreadable().len()
        );
        for range in report.unreadable() {
            println!("          {range}");
        }
    }
}

/// Prints the media this machine exposes, and the shadow copies it holds.
pub fn devices() {
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

/// Prints what a finished scan found.
#[expect(
    clippy::too_many_lines,
    reason = "one branch per thing a scan can have to report; splitting it would \
              scatter the output format across functions without making any of it clearer"
)]
pub fn summarize(report: &ScanReport) {
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
    if report.ceilings.reassembly_decodes {
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
    if report.previews_written > 0 || report.previews_failed > 0 {
        println!(
            "previews  {} rendered into {}/",
            report.previews_written,
            argos_report::PREVIEW_DIR
        );
        if report.previews_failed > 0 {
            println!(
                "          {} could not be written; the artifacts themselves are stored \
                 and recorded",
                report.previews_failed
            );
        }
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
    if report.journal_deletions > 0 {
        println!(
            "journal   {} deletions read from change journals; artifacts sharing one moment \
             were removed in one action",
            report.journal_deletions
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
    if report.ceilings.detection {
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
    if report.dropped_unreadable > 0 {
        println!(
            "lost      {} findings overlapped a damaged region and were dropped; the \
             signature that started each was real",
            report.dropped_unreadable
        );
    }
    if report.omitted_assets > 0 {
        println!(
            "omitted   {} artifacts were under the size floor and not written; each is in \
             the manifest with its dimensions and extents",
            report.omitted_assets
        );
    }
    if report.partial_prefixes > 0 {
        println!(
            "partial   {} images were reported as the part of themselves that decodes",
            report.partial_prefixes
        );
    }
    if report.reassembly_skipped_small > 0 {
        println!(
            "skipped   {} fragmented candidates declare a picture under the size floor and \
             were not searched; a run with a lower floor searches them",
            report.reassembly_skipped_small
        );
    }
}

/// Prints what a run reached and what it stopped short of.
///
/// These are the figures that separate a medium that held nothing more from a
/// run that did not look: what was recognised and deliberately not written,
/// what the search skipped, what damage cost, and which volumes were there to
/// read metadata from at all (`A-CONFIDENCE-HONEST`).
///
/// Printed before the artifact list, because a scan of a used disk has
/// hundreds of thousands of those and these numbers are what a reader needs
/// first.
fn coverage(manifest: &Manifest) {
    let Some(coverage) = &manifest.coverage else {
        return;
    };
    println!("swept     {} bytes", coverage.bytes_swept);
    if !manifest.volumes.is_empty() {
        let residual = manifest
            .volumes
            .iter()
            .filter(|volume| volume.origin == "residual")
            .count();
        println!(
            "volumes   {} located ({residual} left by earlier formats)",
            manifest.volumes.len()
        );
        // Counted by family first. A residue sweep of a re-formatted disk can
        // report thousands of anchors, and which families were found is the
        // fact that matters; the individual offsets are in the manifest.
        let mut families: Vec<(&str, usize)> = Vec::new();
        for volume in &manifest.volumes {
            match families.iter_mut().find(|(kind, _)| *kind == volume.kind) {
                Some((_, count)) => *count += 1,
                None => families.push((&volume.kind, 1)),
            }
        }
        families.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (kind, count) in families {
            println!("          {count} {kind}");
        }
        for volume in manifest.volumes.iter().take(LISTED_BY_DEFAULT) {
            println!(
                "          {:<6} {:<8} at {}, {} bytes, {} per allocation unit",
                volume.kind, volume.origin, volume.offset, volume.length, volume.allocation_bytes
            );
        }
        if manifest.volumes.len() > LISTED_BY_DEFAULT {
            println!(
                "          … and {} more anchors, all in the manifest",
                manifest.volumes.len() - LISTED_BY_DEFAULT
            );
        }
    }
    // Each of these is a place a recovery could have been and was not, and each
    // says which of the reasons it was.
    for (count, text) in [
        (
            coverage.journal_deletions,
            "deletions were read from change journals, naming and dating what was removed",
        ),
        (
            coverage.unattributed_residue,
            "orphaned metadata regions could not be tied to a volume, so their extents were \
             never resolved",
        ),
        (
            coverage.omitted_assets,
            "artifacts were under the size floor and not written; each is below with its \
             dimensions",
        ),
        (
            coverage.partial_prefixes,
            "images were reported as the part of themselves that decodes",
        ),
        (
            coverage.reassembly_skipped_small,
            "fragmented candidates declare a picture under the size floor and were not searched",
        ),
        (
            coverage.dropped_unreadable,
            "findings overlapped a damaged region and were dropped",
        ),
        (
            coverage.unrecoverable,
            "findings claimed bytes that could not be read back",
        ),
        (
            coverage.duplicates,
            "artifacts were collapsed by content hash",
        ),
    ] {
        if count > 0 {
            println!("          {count} {text}");
        }
    }
    if coverage.reassembly_attempted > 0 {
        println!(
            "reassembly {} recovered of {} fragmented candidates searched",
            coverage.reassembled, coverage.reassembly_attempted
        );
    }
    for ceiling in &coverage.ceilings {
        println!("ceiling   {ceiling} reached; the run looked at less than it set out to");
    }
}

/// Prints what a session directory holds, read back from its manifest.
///
/// The headless counterpart of a results view: it answers "what did that scan
/// recover?" from the record the scan left, without re-reading the medium.
/// Artifacts listed by default, before `--all` is needed.
///
/// A scan of a used disk records hundreds of thousands, and a list that long
/// is a list nobody reads. The head of it, strongest evidence first, is what
/// answers "did my photographs come back"; the manifest answers everything
/// else (`M-DOCUMENTED-MAGIC`).
const LISTED_BY_DEFAULT: usize = 40;

pub fn manifest(manifest: &Manifest, all: bool) {
    println!("source    {}", manifest.source);
    println!("state     {}", manifest.scan_state);
    println!("tool      {}", manifest.tool_version);
    // Recorded and written are different numbers, and saying so is the whole
    // point of recording an artifact that was not written: the manifest is a
    // complete account of the medium, the directory is what the caller asked
    // for (`A-CONFIDENCE-HONEST`).
    let written = manifest
        .artifacts
        .iter()
        .filter(|record| record.written)
        .count();
    println!(
        "recorded  {} artifacts, {written} of them written to this directory",
        manifest.artifacts.len()
    );
    println!(
        "rejected  {} candidates that failed validation",
        manifest.rejected_candidates
    );
    if !manifest.unreadable.is_empty() {
        let bytes: u64 = manifest.unreadable.iter().map(|range| range.length).sum();
        println!(
            "damaged   {} regions ({bytes} bytes) could not be read",
            manifest.unreadable.len()
        );
    }
    if let Some(triage) = &manifest.triage {
        match triage.status.as_str() {
            "scored" => println!(
                "triage    {} scored, {} unscored, model {}",
                triage.scored,
                triage.unscored,
                triage.model_version.as_deref().unwrap_or("unknown")
            ),
            _ => println!(
                "triage    disabled: {}",
                triage
                    .disabled_reason
                    .as_deref()
                    .unwrap_or("no reason given")
            ),
        }
    }
    coverage(manifest);

    // Strongest evidence first, then the largest picture, so what a person is
    // looking for is at the top of the list rather than somewhere in it. The
    // order is presentation; the manifest's own order is untouched.
    let mut listed: Vec<&argos_report::ArtifactRecord> = manifest.artifacts.iter().collect();
    listed.sort_by(|left, right| {
        crate::standing::rank(right)
            .cmp(&crate::standing::rank(left))
            .then(crate::standing::long_side(right).cmp(&crate::standing::long_side(left)))
            .then(right.length.cmp(&left.length))
    });

    let shown = if all {
        listed.len()
    } else {
        listed.len().min(LISTED_BY_DEFAULT)
    };
    for record in &listed[..shown] {
        print!(
            "  {:<12} {:>12} bytes  {:<16} at {}",
            record.name.as_deref().unwrap_or("(not written)"),
            record.length,
            record.confidence,
            record.source_offset
        );
        print!("  {}", crate::standing::of(record));
        if let (Some(width), Some(height)) = (record.width, record.height) {
            print!("  {width}x{height}");
        }
        if let Some(taken) = &record.taken {
            print!("  taken {taken}");
        }
        if let Some(deleted) = record.deleted_unix {
            print!("  deleted {deleted}");
        }
        if let Some(label) = &record.triage_label {
            print!("  {label}");
        }
        if record.preview.is_some() {
            print!("  +preview");
        }
        println!();
    }
    if shown < listed.len() {
        println!(
            "  … and {} more, weaker evidence first onwards; `--all` lists them, and the \
             manifest records every one either way",
            listed.len() - shown
        );
    }
}
