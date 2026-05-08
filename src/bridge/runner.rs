use std::path::Path;

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

pub fn run(
    source_path: &Path,
    output_path: &Path,
    session: &Session,
    app: &AppHandle,
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
            let mut reader = BlockReader::new(device, buf, size);
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
                emit_progress(app, session.id, bytes_scanned, candidates_found, 0);
            }

            for (offset, length) in reader.bad_sectors() {
                bad_map.record(*offset, *length);
            }

            (scanner.finish(), bytes_scanned)
        }
        DeviceClass::Hdd => {
            let candidates = crate::carve::hdd::scan(source_path, sector_size)?;
            let bytes_scanned = size;
            (candidates, bytes_scanned)
        }
    };

    let bad_path = output_path.join("bad_sectors.csv");
    bad_map.write_to(&bad_path)?;

    let artifacts = reassemble_ssd(all_candidates);
    let candidates_found = artifacts.len() as u64;

    let mut recovered: u64 = 0;
    for artifact in &artifacts {
        if session.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let mut extract_buf = vec![0u8; artifact.length as usize];
        let extract_device = SourceDevice::open(source_path)?;
        let _ = extract_device.read_range(&mut extract_buf, artifact.offset)?;

        let score = match artifact.format {
            ImageFormat::Jpeg => validate::jpeg::validate(&extract_buf)?,
            ImageFormat::Png => validate::png::validate(&extract_buf)?,
        };

        if score > 0.0 {
            let hash = crate::custody::hash(&extract_buf);
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

            recovered += 1;

            audit.append(AuditEntry::new(
                Operation::Recover,
                source_path.to_string_lossy().into_owned(),
                Some(name),
                Some((artifact.offset, artifact.length)),
                Status::Ok,
            ))?;

            emit_artifact(app, session.id, artifact.offset, artifact.length, artifact.format, score);
            emit_progress(app, session.id, bytes_scanned, candidates_found, recovered);
        }
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

fn emit_artifact(
    app: &AppHandle,
    session_id: u64,
    offset: u64,
    length: u64,
    format: ImageFormat,
    score: f32,
) {
    let event = ArtifactEvent {
        session_id,
        offset,
        length,
        format: format!("{format:?}"),
        score,
    };
    app.emit("artifact", event).ok();
}
