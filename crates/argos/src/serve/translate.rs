//! Engine and report vocabulary to wire vocabulary.
//!
//! This is the only place the two meet. It exists so that the DTO crate can
//! depend on nothing: every conversion from something the engine knows into
//! something a client is told is written out here, by hand, where a reviewer
//! can see exactly what crosses the boundary and what does not.
//!
//! Nothing is computed here. A field is copied, or a value is rendered with
//! the `Display` the engine already defines. A calculation in this file would
//! be a second implementation of something the engine already decided
//! (`A-SHELL-NO-DOMAIN`).

use argos_core::ports::ScanEvent;
use argos_ipc::dto;
use argos_report::Manifest;

/// Everything this machine exposes.
pub fn inventory() -> dto::Inventory {
    dto::Inventory {
        devices: argos_device::inventory::list()
            .iter()
            .map(|device| dto::Device {
                path: device.path.display().to_string(),
                kind: match device.kind {
                    argos_device::naming::NodeKind::WholeDisk => "disk".to_owned(),
                    argos_device::naming::NodeKind::Partition => "partition".to_owned(),
                },
                capacity_bytes: device.capacity_bytes,
                class: device.class.to_string(),
                trim: device.trim.to_string(),
                model: device.model.clone(),
                mounts: device
                    .mounts
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                writable_mount: device.has_writable_mount(),
            })
            .collect(),
        shadow_copies: argos_device::inventory::shadow_copies()
            .iter()
            .map(|shadow| dto::ShadowCopy {
                path: shadow.path.display().to_string(),
                index: shadow.index,
            })
            .collect(),
    }
}

/// One artifact record as a client sees it.
fn artifact(record: &argos_report::ArtifactRecord) -> dto::Artifact {
    dto::Artifact {
        name: record.name.clone(),
        sha256: record.sha256.clone(),
        stage: record.stage.clone(),
        format: record.format.clone(),
        confidence: record.confidence.clone(),
        length: record.length,
        expected_length: record.expected_length,
        missing_bytes: record.missing_bytes,
        extents: record.extents.iter().map(extent).collect(),
        created_unix: record.created_unix,
        modified_unix: record.modified_unix,
        recovered_name: record.recovered_name.clone(),
        standing: Some(crate::results::standing_of(record).to_string()),
        width: record.width,
        height: record.height,
        // Joined here rather than in the window: two fields that are one fact
        // to a reader, and a client that assembled them would be deciding how
        // a camera is named.
        camera: camera(record),
        taken: record.taken.clone(),
        same_size_neighbours: record.same_size_neighbours,
        triage_label: record.triage_label.clone(),
        triage_decided_by: record.triage_decided_by.clone(),
        near_duplicate_of: record.near_duplicate_of.clone(),
        preview: record.preview.clone(),
    }
}

/// The camera a record names, make and model joined, when it names one.
fn camera(record: &argos_report::ArtifactRecord) -> Option<String> {
    match (&record.camera_make, &record.camera_model) {
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (None, None) => None,
    }
}

/// What a session directory holds, as a client sees it.
pub fn results(manifest: &Manifest) -> dto::Results {
    dto::Results {
        tool_version: manifest.tool_version.clone(),
        source: manifest.source.clone(),
        state: manifest.scan_state.clone(),
        rejected_candidates: manifest.rejected_candidates,
        unreadable: manifest.unreadable.iter().map(extent).collect(),
        triage: manifest.triage.as_ref().map(|triage| dto::Triage {
            status: triage.status.clone(),
            disabled_reason: triage.disabled_reason.clone(),
            model_version: triage.model_version.clone(),
            scored: triage.scored,
            unscored: triage.unscored,
            degraded: triage.degraded,
        }),
        artifacts: manifest.artifacts.iter().map(artifact).collect(),
    }
}

/// The figures of a finished run, counted here rather than by the client.
///
/// A client shows a count and a total; handing it every record so it can
/// derive them is megabytes of JSON for three numbers, and the work of
/// materializing them lands on whatever thread draws the window.
///
/// The manifest is the account of the medium, not of the output directory: a
/// run asked to leave synthetic assets unwritten records every one of them
/// with `written: false`. Those are counted apart from what was recovered, so
/// no figure here describes as recovered a file that is not on disk
/// (`A-CONFIDENCE-HONEST`).
pub fn summary(manifest: &Manifest, session: &std::path::Path) -> dto::Summary {
    let written = manifest.artifacts.iter().filter(|record| record.written);
    dto::Summary {
        state: manifest.scan_state.clone(),
        source: manifest.source.clone(),
        artifacts: written.clone().count() as u64,
        bytes: written.fold(0_u64, |sum, record| sum.saturating_add(record.length)),
        omitted: manifest
            .artifacts
            .iter()
            .filter(|record| !record.written)
            .count() as u64,
        rejected_candidates: manifest.rejected_candidates,
        unreadable_regions: manifest.unreadable.len() as u64,
        unreadable_bytes: manifest
            .unreadable
            .iter()
            .fold(0_u64, |sum, record| sum.saturating_add(record.length)),
        session: session.display().to_string(),
        triage: manifest.triage.as_ref().map(|triage| dto::Triage {
            status: triage.status.clone(),
            disabled_reason: triage.disabled_reason.clone(),
            model_version: triage.model_version.clone(),
            scored: triage.scored,
            unscored: triage.unscored,
            degraded: triage.degraded,
        }),
    }
}

/// One recorded byte range.
fn extent(record: &argos_report::ExtentRecord) -> dto::Extent {
    dto::Extent {
        offset: record.offset,
        length: record.length,
    }
}

/// One progress event as the notification it becomes.
///
/// `None` for an event a client has nothing to do with.
pub fn event(event: ScanEvent) -> Option<argos_ipc::wire::Notification> {
    use argos_ipc::wire::Notification;
    match event {
        ScanEvent::StageStarted { stage, unit, total } => {
            Some(Notification::StageBegan(dto::StageBegan {
                stage: stage.to_string(),
                unit: unit.to_string(),
                total,
            }))
        }
        ScanEvent::StageProgress {
            stage,
            unit,
            done,
            total,
        } => Some(Notification::Progress(dto::Progress {
            stage: stage.to_string(),
            unit: unit.to_string(),
            done,
            total,
        })),
        ScanEvent::StageFinished { stage, findings } => {
            Some(Notification::StageDone(dto::StageDone {
                stage: stage.to_string(),
                findings,
            }))
        }
        ScanEvent::ArtifactStored { artifacts, bytes } => {
            Some(Notification::Stored(dto::Stored { artifacts, bytes }))
        }
        ScanEvent::StateChanged { state } => Some(Notification::State(dto::State {
            state: state.to_string(),
        })),
        // Unreadable regions reach a client as a running total the pacer
        // keeps, never one message per bad sector; the extents themselves
        // belong in the manifest rather than on a progress channel. The same
        // arm covers a variant added to the port after this was written: a
        // client that never knew about it loses nothing, and inventing a
        // message for it would put a name on the wire the schema does not
        // cover.
        _ => None,
    }
}

/// The scan options a request asks for.
pub fn options(request: &dto::ScanRequest) -> crate::scan::Options {
    crate::scan::Options {
        reference: request.reference.as_deref().map(std::path::PathBuf::from),
        jobs: request
            .jobs
            .and_then(|jobs| usize::try_from(jobs).ok())
            .and_then(std::num::NonZeroUsize::new),
        stages: argos_engine::Stages {
            filesystem: request.filesystem,
            carving: request.carving,
            reassembly: request.reassembly,
        },
        triage: request.triage,
        min_long_side: request.min_long_side,
        reassembly_budget: request
            .reassembly_budget_seconds
            .map(std::time::Duration::from_secs),
        previews: request.previews,
        // The wire carries no range: a client that could scan part of a medium
        // and report it as the medium is a client that can mislead, and adding
        // it is a schema change made on purpose (`A-DTO-VERSIONED`).
        range: None,
        // Not on the wire: resuming a search names a session directory, which
        // is a path, and the protocol deliberately carries none.
        resume_from: None,
    }
}
