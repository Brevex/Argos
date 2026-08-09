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

/// Prints what a session directory holds, read back from its manifest.
///
/// The headless counterpart of a results view: it answers "what did that scan
/// recover?" from the record the scan left, without re-reading the medium.
pub fn manifest(manifest: &Manifest) {
    println!("source    {}", manifest.source);
    println!("state     {}", manifest.scan_state);
    println!("tool      {}", manifest.tool_version);
    println!("recovered {} artifacts", manifest.artifacts.len());
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

    for record in &manifest.artifacts {
        print!(
            "  {:<12} {:>12} bytes  {:<16} at {}",
            record.name.as_deref().unwrap_or("(not written)"),
            record.length,
            record.confidence,
            record.source_offset
        );
        if let Some(label) = &record.triage_label {
            print!("  {label}");
        }
        if record.preview.is_some() {
            print!("  +preview");
        }
        println!();
    }
}
