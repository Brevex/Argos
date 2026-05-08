use std::io::Write;

use argos::bridge::runner::run_test;
use tempfile::tempdir;

fn minimal_jpeg() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xFF, 0xD8]);

    let mut dht = Vec::new();
    dht.push(0x00);
    dht.extend_from_slice(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    dht.push(0x00);
    let dht_len = (dht.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC4);
    data.extend_from_slice(&dht_len.to_be_bytes());
    data.extend_from_slice(&dht);

    let mut dht_ac = Vec::new();
    dht_ac.push(0x10);
    dht_ac.extend_from_slice(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    dht_ac.push(0x00);
    let dht_ac_len = (dht_ac.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC4);
    data.extend_from_slice(&dht_ac_len.to_be_bytes());
    data.extend_from_slice(&dht_ac);

    let mut sof = Vec::new();
    sof.push(0x08);
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.push(0x01);
    sof.extend_from_slice(&[0x01, 0x11, 0x00]);
    let sof_len = (sof.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC0);
    data.extend_from_slice(&sof_len.to_be_bytes());
    data.extend_from_slice(&sof);

    let mut sos = Vec::new();
    sos.push(0x01);
    sos.extend_from_slice(&[0x01, 0x00]);
    sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
    let sos_len = (sos.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xDA);
    data.extend_from_slice(&sos_len.to_be_bytes());
    data.extend_from_slice(&sos);

    data.push(0x00);
    data.push(0x00);

    data.push(0xFF);
    data.push(0xD9);
    data
}

fn valid_png() -> Vec<u8> {
    let signature = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut data = Vec::new();
    data.extend_from_slice(&signature);

    let ihdr_len = 13u32;
    let ihdr_type = b"IHDR";
    let ihdr_data = [0x00; 13];
    let ihdr_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(ihdr_type);
        h.update(&ihdr_data);
        h.finalize()
    };
    data.extend_from_slice(&ihdr_len.to_be_bytes());
    data.extend_from_slice(ihdr_type);
    data.extend_from_slice(&ihdr_data);
    data.extend_from_slice(&ihdr_crc.to_be_bytes());

    let idat_len = 10u32;
    let idat_type = b"IDAT";
    let idat_data = [0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01];
    let idat_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(idat_type);
        h.update(&idat_data);
        h.finalize()
    };
    data.extend_from_slice(&idat_len.to_be_bytes());
    data.extend_from_slice(idat_type);
    data.extend_from_slice(&idat_data);
    data.extend_from_slice(&idat_crc.to_be_bytes());

    let iend_len = 0u32;
    let iend_type = b"IEND";
    let iend_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(iend_type);
        h.finalize()
    };
    data.extend_from_slice(&iend_len.to_be_bytes());
    data.extend_from_slice(iend_type);
    data.extend_from_slice(&iend_crc.to_be_bytes());

    data
}

fn create_synthetic_device(path: &std::path::Path) {
    let mut file = std::fs::File::create(path).unwrap();
    let garbage = vec![0xABu8; 4096];
    let jpeg = minimal_jpeg();
    let png = valid_png();

    file.write_all(&garbage).unwrap();
    file.write_all(&jpeg).unwrap();
    file.write_all(&garbage).unwrap();
    file.write_all(&png).unwrap();
    file.write_all(&garbage).unwrap();
    file.flush().unwrap();
}

#[test]
fn full_pipeline_recovers_embedded_images() {
    let source_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let source_path = source_dir.path().join("device.bin");
    create_synthetic_device(&source_path);

    let report = run_test(&source_path, output_dir.path()).unwrap();

    assert!(report.bytes_scanned > 0);

    let output_files: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();

    assert!(output_files.iter().any(|n| n.to_string_lossy().ends_with(".bin")));
    assert!(output_files.iter().any(|n| n.to_string_lossy() == "audit.log"));
    assert!(output_files.iter().any(|n| n.to_string_lossy() == "bad_sectors.csv"));
}

#[test]
fn audit_log_has_chained_entries() {
    let source_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let source_path = source_dir.path().join("device.bin");
    create_synthetic_device(&source_path);

    let _report = run_test(&source_path, output_dir.path()).unwrap();

    let audit_path = output_dir.path().join("audit.log");
    let content = std::fs::read_to_string(&audit_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert!(!lines.is_empty());

    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["operation"], "open");
    assert!(first["previous_hash"].is_null());

    if lines.len() > 1 {
        let last: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();
        assert_eq!(last["operation"], "close");
        assert!(last["previous_hash"].is_string());
    }
}

#[test]
fn empty_device_produces_no_artifacts() {
    let source_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let source_path = source_dir.path().join("empty.bin");
    let mut file = std::fs::File::create(&source_path).unwrap();
    file.write_all(&[0u8; 8192]).unwrap();

    let report = run_test(&source_path, output_dir.path()).unwrap();

    assert_eq!(report.artifacts_recovered, 0);
}
