mod common;

use argos::bridge::runner::{run_test, run_test_with_device_class};
use argos::carve::DeviceClass;
use argos::error::ArgosError;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use tempfile::tempdir;

use common::{minimal_baseline_jpeg, sector_aligned_device, synthetic_device, valid_png, write_to};

fn try_recover(source: &Path, output: &Path) -> argos::bridge::runner::RecoveryReport {
    match run_test(source, output) {
        Ok(report) => report,
        Err(e) => panic!("recovery failed: {e:?}"),
    }
}

fn recover_as(
    source: &Path,
    output: &Path,
    device_class: DeviceClass,
) -> argos::bridge::runner::RecoveryReport {
    match run_test_with_device_class(source, output, device_class) {
        Ok(report) => report,
        Err(ArgosError::Io(ref e)) if e.raw_os_error() == Some(22) => {
            panic!("direct I/O rejected the synthetic fixture: {e:?}")
        }
        Err(e) => panic!("recovery failed: {e:?}"),
    }
}

fn output_file_names(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .expect("read output dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

fn assert_empty_bad_sector_map(dir: &Path) {
    let csv = std::fs::read_to_string(dir.join("bad_sectors.csv")).expect("bad sectors");
    assert!(
        csv.is_empty(),
        "synthetic fixtures must not record bad sectors"
    );
}

fn assert_final_progress_matches_report(report: &argos::bridge::runner::RecoveryReport) {
    let final_event = report.progress_events.last().expect("progress event");
    assert_eq!(final_event.bytes_scanned, report.bytes_scanned);
    assert_eq!(final_event.candidates_found, report.candidates_found);
    assert_eq!(final_event.artifacts_recovered, report.artifacts_recovered);
}

#[test]
fn pipeline_recovers_embedded_jpeg_and_png_from_synthetic_device() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");

    let device = synthetic_device(4096, 4096, 4096);
    write_to(&source_path, &device).expect("write device");

    let report = try_recover(&source_path, output_dir.path());

    assert!(report.bytes_scanned > 0, "scan must advance");
    assert!(
        report.artifacts_recovered >= 1,
        "pipeline must recover at least one artifact"
    );
    let names = output_file_names(output_dir.path());
    assert!(names.contains("audit.log"));
    assert!(names.contains("bad_sectors.csv"));
    let has_image = names
        .iter()
        .any(|n| n.ends_with(".jpg") || n.ends_with(".png"));
    assert!(has_image, "expected at least one .jpg or .png artifact");
    assert_final_progress_matches_report(&report);
    assert_empty_bad_sector_map(output_dir.path());
}

#[test]
fn forced_ssd_pipeline_recovers_known_jpeg_and_png_and_reports_final_counts() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("ssd-device.bin");
    let jpeg = minimal_baseline_jpeg();
    let png = valid_png();
    let device = sector_aligned_device(4096, &[(4096, &jpeg), (8192, &png)]);
    write_to(&source_path, &device).expect("write device");

    let report = recover_as(&source_path, output_dir.path(), DeviceClass::Ssd);

    assert_eq!(report.bytes_scanned, device.len() as u64);
    assert_eq!(report.candidates_found, 2);
    assert_eq!(report.artifacts_recovered, 2);
    assert_eq!(report.artifact_events.len(), 2);
    assert!(
        report
            .recovered_files
            .iter()
            .any(|name| name.starts_with("Jpeg@4096:"))
    );
    assert!(
        report
            .recovered_files
            .iter()
            .any(|name| name.starts_with("Png@8192:"))
    );
    assert_final_progress_matches_report(&report);
    assert_empty_bad_sector_map(output_dir.path());
}

#[test]
fn forced_hdd_pipeline_recovers_known_jpeg_and_png_and_reports_candidates() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("hdd-device.bin");
    let jpeg = minimal_baseline_jpeg();
    let png = valid_png();
    let device = sector_aligned_device(4096, &[(4096, &jpeg), (12288, &png)]);
    write_to(&source_path, &device).expect("write device");

    let report = recover_as(&source_path, output_dir.path(), DeviceClass::Hdd);

    assert_eq!(report.bytes_scanned, device.len() as u64);
    assert!(
        report
            .progress_events
            .iter()
            .any(|event| event.candidates_found > 0),
        "HDD progress must eventually report analyzed files"
    );
    assert_eq!(report.candidates_found, 2);
    assert_eq!(report.artifacts_recovered, 2);
    assert_eq!(report.artifact_events.len(), 2);
    assert_final_progress_matches_report(&report);
    assert_empty_bad_sector_map(output_dir.path());
}

#[test]
fn pipeline_emits_open_recover_close_audit_chain() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");

    let device = synthetic_device(4096, 4096, 4096);
    write_to(&source_path, &device).expect("write device");

    let _ = try_recover(&source_path, output_dir.path());

    let content =
        std::fs::read_to_string(output_dir.path().join("audit.log")).expect("read audit log");
    let entries: Vec<Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).expect("audit json"))
        .collect();

    assert!(!entries.is_empty(), "audit log must not be empty");
    assert_eq!(entries[0]["operation"], "open");
    assert!(entries[0]["previous_hash"].is_null());

    let last = entries.last().expect("non-empty");
    assert_eq!(last["operation"], "close");
    assert!(last["previous_hash"].is_string());
}

#[test]
fn pipeline_chains_every_audit_entry_after_the_first() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");

    let device = synthetic_device(4096, 0, 4096);
    write_to(&source_path, &device).expect("write device");

    let _ = try_recover(&source_path, output_dir.path());

    let content = std::fs::read_to_string(output_dir.path().join("audit.log")).expect("read");
    let entries: Vec<Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert!(entries[0]["previous_hash"].is_null());
    for entry in &entries[1..] {
        assert!(
            entry["previous_hash"].is_string(),
            "every entry after the first must chain"
        );
    }
}

#[test]
fn empty_device_produces_no_artifacts_but_writes_audit_skeleton() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("empty.bin");
    write_to(&source_path, &vec![0u8; 16 * 1024]).expect("write");

    let report = try_recover(&source_path, output_dir.path());

    assert_eq!(report.artifacts_recovered, 0);
    assert!(report.recovered_files.is_empty());
    assert!(output_dir.path().join("audit.log").exists());
    assert!(output_dir.path().join("bad_sectors.csv").exists());
}

#[test]
fn pure_garbage_device_yields_zero_validated_artifacts() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("garbage.bin");
    write_to(&source_path, &vec![0xC3u8; 64 * 1024]).expect("write");

    let report = try_recover(&source_path, output_dir.path());

    assert_eq!(report.artifacts_recovered, 0);
}

#[test]
fn recovered_artifact_filenames_carry_format_extension() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &synthetic_device(0, 0, 0)).expect("write");

    let report = try_recover(&source_path, output_dir.path());
    assert!(
        report.artifacts_recovered > 0,
        "fixture must recover at least one artifact"
    );

    let names = output_file_names(output_dir.path());
    for name in &names {
        if name == "audit.log" || name == "bad_sectors.csv" {
            continue;
        }
        assert!(
            name.ends_with(".jpg") || name.ends_with(".png"),
            "unexpected output filename: {name}"
        );
    }
}

#[test]
fn recovered_artifact_filenames_embed_hash_prefix() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &synthetic_device(0, 0, 0)).expect("write");

    let report = try_recover(&source_path, output_dir.path());
    assert!(
        report.artifacts_recovered > 0,
        "fixture must recover at least one artifact"
    );

    let names = output_file_names(output_dir.path());
    for name in &names {
        if name == "audit.log" || name == "bad_sectors.csv" {
            continue;
        }
        let prefix: String = name.chars().take(8).collect();
        assert!(
            prefix.chars().all(|c| c.is_ascii_hexdigit()),
            "filename {name} does not start with a hex hash prefix"
        );
    }
}

#[test]
fn pipeline_recovered_files_match_artifacts_recovered_counter() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &synthetic_device(8192, 4096, 8192)).expect("write");

    let report = try_recover(&source_path, output_dir.path());

    assert_eq!(
        report.artifacts_recovered as usize,
        report.recovered_files.len()
    );
}

#[test]
fn pipeline_recovers_isolated_jpeg_without_surrounding_garbage() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &minimal_baseline_jpeg()).expect("write");

    let report = try_recover(&source_path, output_dir.path());
    assert_eq!(report.artifacts_recovered, 1);
    let names = output_file_names(output_dir.path());
    assert!(names.iter().any(|n| n.ends_with(".jpg")));
}

#[test]
fn pipeline_recovers_isolated_png_without_surrounding_garbage() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &valid_png()).expect("write");

    let report = try_recover(&source_path, output_dir.path());
    assert_eq!(report.artifacts_recovered, 1);
    let names = output_file_names(output_dir.path());
    assert!(names.iter().any(|n| n.ends_with(".png")));
}

#[test]
fn pipeline_appends_to_existing_audit_log_across_sessions() {
    let source_dir = tempdir().expect("tempdir");
    let output_dir = tempdir().expect("tempdir");
    let source_path = source_dir.path().join("device.bin");
    write_to(&source_path, &synthetic_device(0, 0, 0)).expect("write");

    let _ = try_recover(&source_path, output_dir.path());
    let first_size = std::fs::metadata(output_dir.path().join("audit.log"))
        .expect("metadata")
        .len();

    let _ = try_recover(&source_path, output_dir.path());
    let second_size = std::fs::metadata(output_dir.path().join("audit.log"))
        .expect("metadata")
        .len();

    assert!(
        second_size > first_size,
        "audit log must grow across sessions"
    );
}
