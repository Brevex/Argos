use argos::custody::{AuditEntry, AuditLog, BadSectorMap, Operation, Status, hash};
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn sha256_is_deterministic_per_input() {
    let a = hash(b"forensic-input");
    let b = hash(b"forensic-input");
    assert_eq!(a, b);
}

#[test]
fn sha256_differs_for_different_inputs() {
    assert_ne!(hash(b"a"), hash(b"b"));
}

#[test]
fn audit_log_first_entry_has_null_previous_hash() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("audit.log");
    let mut log = AuditLog::open(&path).expect("open");
    log.append(AuditEntry::new(
        Operation::Open,
        "source".into(),
        None,
        None,
        Status::Ok,
    ))
    .expect("append");
    drop(log);

    let content = std::fs::read_to_string(&path).expect("read");
    let entry: Value =
        serde_json::from_str(content.lines().next().expect("one line")).expect("json");
    assert_eq!(entry["operation"], "open");
    assert!(entry["previous_hash"].is_null());
}

#[test]
fn audit_log_subsequent_entries_chain_via_previous_hash() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("audit.log");
    let mut log = AuditLog::open(&path).expect("open");

    for i in 0..5 {
        log.append(AuditEntry::new(
            Operation::Read,
            format!("src_{i}"),
            None,
            Some((i * 4096, 4096)),
            Status::Ok,
        ))
        .expect("append");
    }
    drop(log);

    let content = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 5);

    let first: Value = serde_json::from_str(lines[0]).expect("json");
    assert!(first["previous_hash"].is_null());

    for line in &lines[1..] {
        let entry: Value = serde_json::from_str(line).expect("json");
        assert!(entry["previous_hash"].is_string());
        assert!(!entry["previous_hash"].as_str().unwrap().is_empty());
    }
}

#[test]
fn audit_log_entries_carry_structured_byte_range_when_provided() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("audit.log");
    let mut log = AuditLog::open(&path).expect("open");
    log.append(AuditEntry::new(
        Operation::Recover,
        "source".into(),
        Some("art.jpg".into()),
        Some((0x1000, 0x800)),
        Status::Ok,
    ))
    .expect("append");
    drop(log);

    let content = std::fs::read_to_string(&path).expect("read");
    let entry: Value =
        serde_json::from_str(content.lines().next().expect("one line")).expect("json");
    let range = entry["byte_range"].as_array().expect("byte_range array");
    assert_eq!(range[0].as_u64(), Some(0x1000));
    assert_eq!(range[1].as_u64(), Some(0x800));
}

#[test]
fn audit_log_append_is_idempotent_across_open_close() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("audit.log");

    {
        let mut log = AuditLog::open(&path).expect("open");
        log.append(AuditEntry::new(
            Operation::Open,
            "session_1".into(),
            None,
            None,
            Status::Ok,
        ))
        .expect("append");
    }
    {
        let mut log = AuditLog::open(&path).expect("reopen");
        log.append(AuditEntry::new(
            Operation::Close,
            "session_1".into(),
            None,
            None,
            Status::Ok,
        ))
        .expect("append");
    }

    let content = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);
}

#[test]
fn bad_sector_map_records_offset_and_length_pairs() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("bad_sectors.csv");
    let mut map = BadSectorMap::new();
    map.record(0, 4096);
    map.record(8192, 512);

    map.write_to(&path).expect("write");

    let csv = std::fs::read_to_string(&path).expect("read");
    assert!(csv.contains("0,4096"));
    assert!(csv.contains("8192,512"));
    assert_eq!(map.entries().len(), 2);
}

#[test]
fn bad_sector_map_truncates_previous_contents() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("bad_sectors.csv");

    let mut first = BadSectorMap::new();
    first.record(1, 2);
    first.write_to(&path).expect("write1");

    let mut second = BadSectorMap::new();
    second.record(99, 100);
    second.write_to(&path).expect("write2");

    let csv = std::fs::read_to_string(&path).expect("read");
    assert!(csv.contains("99,100"));
    assert!(!csv.contains("1,2"));
}
