use std::path::Path;
use std::sync::atomic::Ordering;

use memmap2::{Mmap, MmapOptions};
use rayon::prelude::*;
use tauri::{AppHandle, Emitter};

use crate::bridge::{
    ArtifactEvent, BridgeError, ProgressEvent, Session, SessionCompletedEvent, SessionStatus,
};
use crate::carve::ssd::Scanner;
use crate::carve::{Candidate, DeviceClass, ImageFormat};
use crate::custody::{AuditEntry, AuditLog, BadSectorMap, Operation, Status};
use crate::error::ArgosError;
use crate::io::OutputSink;
use crate::io::{AlignedBuf, BlockReader, SourceDevice};
use crate::reassemble::reassemble_ssd;
use crate::validate;

const MAX_EXTRACTION_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct RecoveryReport {
    pub bytes_scanned: u64,
    pub candidates_found: u64,
    pub artifacts_recovered: u64,
    pub recovered_files: Vec<String>,
    pub progress_events: Vec<ProgressEvent>,
    pub artifact_events: Vec<ArtifactEvent>,
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
        None,
        |event| {
            app.emit("progress", event).ok();
        },
        |event| {
            app.emit("artifact", event).ok();
        },
    )?;
    Ok(())
}

pub fn run_test(source_path: &Path, output_path: &Path) -> Result<RecoveryReport, ArgosError> {
    run_test_with_class(source_path, output_path, None)
}

pub fn run_test_with_device_class(
    source_path: &Path,
    output_path: &Path,
    device_class: DeviceClass,
) -> Result<RecoveryReport, ArgosError> {
    run_test_with_class(source_path, output_path, Some(device_class))
}

fn run_test_with_class(
    source_path: &Path,
    output_path: &Path,
    forced_device_class: Option<DeviceClass>,
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
        progress_events: Vec::new(),
        artifact_events: Vec::new(),
    };

    run_with_callbacks(
        source_path,
        output_path,
        &session,
        forced_device_class,
        |event| {
            report.bytes_scanned = event.bytes_scanned;
            report.candidates_found = event.candidates_found;
            report.artifacts_recovered = event.artifacts_recovered;
            report.progress_events.push(event);
        },
        |event| {
            report.recovered_files.push(format!(
                "{}@{}:{}:{:.2}",
                event.format, event.offset, event.length, event.score
            ));
            report.artifact_events.push(event);
        },
    )?;

    Ok(report)
}

fn open_extraction_mmap(source_path: &Path, size: u64) -> Result<Mmap, ArgosError> {
    let file = std::fs::File::open(source_path)?;
    let mmap = unsafe { MmapOptions::new().len(size as usize).map(&file)? };
    Ok(mmap)
}

fn read_artifact_bytes(
    file: &std::fs::File,
    source_size: u64,
    offset: u64,
    length: u64,
) -> Result<Option<Vec<u8>>, ArgosError> {
    if offset >= source_size {
        return Ok(None);
    }
    let available = source_size - offset;
    let bounded_length = length.min(available);
    let len = match usize::try_from(bounded_length) {
        Ok(n) if n > 0 && n <= MAX_EXTRACTION_BYTES => n,
        _ => return Ok(None),
    };
    let mut buf = vec![0u8; len];
    match rustix::io::pread(file, &mut buf, offset) {
        Ok(n) if n == len => Ok(Some(buf)),
        Ok(_) => Ok(None),
        Err(_) => Ok(None),
    }
}

fn extension_for(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
    }
}

fn run_with_callbacks(
    source_path: &Path,
    output_path: &Path,
    session: &Session,
    forced_device_class: Option<DeviceClass>,
    mut on_progress: impl FnMut(ProgressEvent),
    mut on_artifact: impl FnMut(ArtifactEvent),
) -> Result<(), ArgosError> {
    let device = SourceDevice::open(source_path)?;
    let size = device.size()?;
    let sector_size = device.sector_size();

    let sink = OutputSink::create(output_path)?;

    let audit_path = output_path.join("audit.log");
    let mut audit = AuditLog::open(&audit_path)?;
    audit.append(AuditEntry::new(
        Operation::Open,
        source_path.to_string_lossy().into_owned(),
        None,
        None,
        Status::Ok,
    ))?;

    let extraction_file = std::fs::File::open(source_path)?;
    let mut bad_map = BadSectorMap::new();

    let device_class =
        forced_device_class.unwrap_or_else(|| crate::io::detect_device_class(source_path));

    let (all_candidates, bytes_scanned) = match device_class {
        DeviceClass::Ssd => scan_ssd(
            &device,
            size,
            sector_size,
            session,
            &mut bad_map,
            &mut on_progress,
        )?,
        DeviceClass::Hdd => {
            let mmap = open_extraction_mmap(source_path, size)?;
            scan_hdd(&mmap, sector_size, session, size, &mut on_progress)?
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
            let bytes =
                read_artifact_bytes(&extraction_file, size, artifact.offset, artifact.length)
                    .ok()
                    .flatten()?;

            let score = match artifact.format {
                ImageFormat::Jpeg => validate::jpeg::validate(&bytes).ok()?,
                ImageFormat::Png => validate::png::validate(&bytes).ok()?,
            };

            if score > 0.0 {
                let hash = crate::custody::hash(&bytes);
                Some((artifact, score, bytes, hash))
            } else {
                None
            }
        })
        .collect();

    for (recovered, (artifact, score, bytes, hash)) in (1_u64..).zip(validated) {
        if session.cancel.load(Ordering::Relaxed) {
            break;
        }

        let name = format!(
            "{}_{}_{}_{:.2}.{}",
            hex::encode(&hash[..4]),
            artifact.offset,
            artifact.length,
            score,
            extension_for(artifact.format),
        );
        let mut writer = sink.create_file(&name)?;
        std::io::Write::write_all(&mut writer, &bytes)?;
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

fn scan_ssd(
    device: &SourceDevice,
    size: u64,
    sector_size: usize,
    session: &Session,
    bad_map: &mut BadSectorMap,
    on_progress: &mut impl FnMut(ProgressEvent),
) -> Result<(Vec<Candidate>, u64), ArgosError> {
    let buf = AlignedBuf::with_capacity(1024 * 1024, sector_size)?;
    let mut reader = BlockReader::new(device, buf, size);
    let mut scanner = Scanner::new()?;
    let mut bytes_scanned: u64 = 0;
    let mut candidates_found: u64 = 0;
    let mut all_candidates: Vec<Candidate> = Vec::new();

    while let Some(block) = reader.try_next()? {
        if session.cancel.load(Ordering::Relaxed) {
            break;
        }
        bytes_scanned += block.len() as u64;
        let found = scanner.scan_block(block)?;
        candidates_found += found.len() as u64;
        all_candidates.extend(found);
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

    Ok((all_candidates, bytes_scanned))
}

fn scan_hdd(
    data: &[u8],
    block_size: usize,
    session: &Session,
    size: u64,
    on_progress: &mut impl FnMut(ProgressEvent),
) -> Result<(Vec<Candidate>, u64), ArgosError> {
    let session_id = session.id;
    let candidates = crate::carve::hdd::scan(data, block_size, |bytes_scanned| {
        on_progress(ProgressEvent {
            session_id,
            bytes_scanned,
            candidates_found: 0,
            artifacts_recovered: 0,
        });
        !session.cancel.load(Ordering::Relaxed)
    })?;
    on_progress(ProgressEvent {
        session_id,
        bytes_scanned: size,
        candidates_found: candidates.len() as u64,
        artifacts_recovered: 0,
    });
    Ok((candidates, size))
}

pub fn emit_completed(
    app: &AppHandle,
    session_id: u64,
    status: SessionStatus,
    error: Option<BridgeError>,
) {
    let event = SessionCompletedEvent {
        session_id,
        status,
        error,
    };
    app.emit("session_completed", event).ok();
}
