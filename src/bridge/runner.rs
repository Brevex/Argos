use std::path::Path;
use std::sync::atomic::Ordering;

use rayon::prelude::*;
use tauri::{AppHandle, Emitter};

use crate::bridge::{ArtifactEvent, ProgressEvent, Session};
use crate::carve::ssd::Scanner;
use crate::carve::{DeviceClass, ImageFormat};
use crate::custody::{AuditEntry, AuditLog, BadSectorMap, Operation, Status};
use crate::io::OutputSink;
use crate::error::ArgosError;
use crate::io::{AlignedBuf, BlockReader, SourceDevice};
use crate::reassemble::reassemble_ssd;
use crate::validate;

#[derive(Debug)]
pub struct RecoveryReport {
    pub bytes_scanned: u64,
    pub candidates_found: u64,
    pub artifacts_recovered: u64,
    pub recovered_files: Vec<String>,
}

pub fn run(
    source_path: &Path,
    output_path: &Path,
    session: &Session,
    app: &AppHandle,
) -> Result<(), ArgosError> {
    run_with_callbacks(
        source_path,
        output_path,
        session,
        |event| {
            app.emit("progress", event).ok();
        },
        |event| {
            app.emit("artifact", event).ok();
        },
    )?;
    Ok(())
}

pub fn run_test(
    source_path: &Path,
    output_path: &Path,
) -> Result<RecoveryReport, ArgosError> {
    let session = crate::bridge::Session {
        id: 0,
        cancel: std::sync::atomic::AtomicBool::new(false),
    };
    let mut report = RecoveryReport {
        bytes_scanned: 0,
        candidates_found: 0,
        artifacts_recovered: 0,
        recovered_files: Vec::new(),
    };

    run_with_callbacks(
        source_path,
        output_path,
        &session,
        |event| {
            report.bytes_scanned = event.bytes_scanned;
            report.candidates_found = event.candidates_found;
            report.artifacts_recovered = event.artifacts_recovered;
        },
        |event| {
            report.recovered_files.push(format!(
                "{}@{}:{}:{:.2}",
                event.format, event.offset, event.length, event.score
            ));
        },
    )?;

    Ok(report)
}

fn run_with_callbacks(
    source_path: &Path,
    output_path: &Path,
    session: &Session,
    mut on_progress: impl FnMut(ProgressEvent),
    mut on_artifact: impl FnMut(ArtifactEvent),
) -> Result<(), ArgosError> {
    let device = SourceDevice::open(source_path)?;
    let size = device.size()?;
    let sector_size = device.sector_size();

    let audit_path = output_path.join("audit.log");
    let mut audit = AuditLog::open(&audit_path)?;
    audit.append(AuditEntry::new(
        Operation::Open,
        source_path.to_string_lossy().into_owned(),
        None,
        None,
        Status::Ok,
    ))?;

    let sink = OutputSink::create(output_path)?;
    let mut bad_map = BadSectorMap::new();

    let device_class = crate::io::detect_device_class(source_path);

    let (all_candidates, bytes_scanned) = match device_class {
        DeviceClass::Ssd => {
            let buf = AlignedBuf::with_capacity(1024 * 1024, sector_size)?;
            let mut reader = BlockReader::new(&device, buf, size);
            let mut scanner = Scanner::new()?;
            let mut bytes_scanned: u64 = 0;
            let mut candidates_found: u64 = 0;

            while let Some(block) = reader.try_next()? {
                if session.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                bytes_scanned += block.len() as u64;
                let found = scanner.scan_block(block)?;
                candidates_found += found.len() as u64;
                on_progress(ProgressEvent {
                    session_id: session.id,
                    bytes_scanned,
                    candidates_found,
                    artifacts_recovered: 0,
                });
            }

            for (offset, length) in reader.bad_sectors() {
                bad_map.record(*offset, *length);
            }

            (scanner.finish(), bytes_scanned)
        }
        DeviceClass::Hdd => {
            let candidates = crate::carve::hdd::scan(source_path, sector_size)?;
            let bytes_scanned = size;
            on_progress(ProgressEvent {
                session_id: session.id,
                bytes_scanned,
                candidates_found: candidates.len() as u64,
                artifacts_recovered: 0,
            });
            (candidates, bytes_scanned)
        }
    };

    let bad_path = output_path.join("bad_sectors.csv");
    bad_map.write_to(&bad_path)?;

    let artifacts = reassemble_ssd(all_candidates);
    let candidates_found = artifacts.len() as u64;

    let validated: Vec<_> = artifacts
        .par_iter()
        .filter_map(|artifact| {
            if session.cancel.load(Ordering::Relaxed) {
                return None;
            }

            let mut extract_buf = vec![0u8; artifact.length as usize];
            if device.read_range(&mut extract_buf, artifact.offset).is_err() {
                return None;
            }

            let score = match artifact.format {
                ImageFormat::Jpeg => validate::jpeg::validate(&extract_buf).ok()?,
                ImageFormat::Png => validate::png::validate(&extract_buf).ok()?,
            };

            if score > 0.0 {
                let hash = crate::custody::hash(&extract_buf);
                Some((artifact, score, extract_buf, hash))
            } else {
                None
            }
        })
        .collect();

    for (recovered, (artifact, score, extract_buf, hash)) in (1_u64..).zip(validated) {
        if session.cancel.load(Ordering::Relaxed) {
            break;
        }

        let name = format!(
            "{}_{}_{}_{:.2}.bin",
            hex::encode(&hash[..4]),
            artifact.offset,
            artifact.length,
            score
        );
        let mut writer = sink.create_file(&name)?;
        std::io::Write::write_all(&mut writer, &extract_buf)?;
        drop(writer);

        audit.append(AuditEntry::new(
            Operation::Recover,
            source_path.to_string_lossy().into_owned(),
            Some(name.clone()),
            Some((artifact.offset, artifact.length)),
            Status::Ok,
        ))?;

        on_artifact(ArtifactEvent {
            session_id: session.id,
            offset: artifact.offset,
            length: artifact.length,
            format: format!("{:?}", artifact.format),
            score,
        });
        on_progress(ProgressEvent {
            session_id: session.id,
            bytes_scanned,
            candidates_found,
            artifacts_recovered: recovered,
        });
    }

    audit.append(AuditEntry::new(
        Operation::Close,
        source_path.to_string_lossy().into_owned(),
        None,
        None,
        Status::Ok,
    ))?;

    Ok(())
}

pub fn emit_progress(
    app: &AppHandle,
    session_id: u64,
    bytes_scanned: u64,
    candidates_found: u64,
    artifacts_recovered: u64,
) {
    let event = ProgressEvent {
        session_id,
        bytes_scanned,
        candidates_found,
        artifacts_recovered,
    };
    app.emit("progress", event).ok();
}


