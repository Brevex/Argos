use std::io::Cursor;

use argos_report::{Provenance, Store};

/// SHA-256 of the ASCII bytes `abc` — the FIPS 180-2 known-answer vector.
const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn saved_artifact_is_written_hashed_and_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");

    let record = store
        .save(
            "000000.jpg",
            Cursor::new(b"abc".to_vec()),
            Provenance {
                stage: "carve",
                format: "jpeg",
                source_offset: 1234,
                length: 3,
                confidence: "contiguous-carve",
                parent_offset: None,
            },
        )
        .expect("save artifact")
        .clone();

    assert_eq!(record.sha256, SHA256_ABC);
    assert_eq!(record.length, 3);
    let written = std::fs::read(dir.path().join("out/000000.jpg")).expect("artifact file");
    assert_eq!(written, b"abc");
}

#[test]
fn manifest_carries_every_record_and_the_rejection_count() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");
    store
        .save(
            "000000.png",
            Cursor::new(vec![1, 2, 3, 4]),
            Provenance {
                stage: "carve",
                format: "png",
                source_offset: 99,
                length: 4,
                confidence: "contiguous-carve",
                parent_offset: Some(42),
            },
        )
        .expect("save artifact");

    let manifest_path = store.finish("9.9.9", "fixture.img", 7).expect("manifest");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).expect("manifest file"))
            .expect("valid json");

    assert_eq!(json["tool_version"], "9.9.9");
    assert_eq!(json["source"], "fixture.img");
    assert_eq!(json["rejected_candidates"], 7);
    assert_eq!(json["artifacts"][0]["name"], "000000.png");
    assert_eq!(json["artifacts"][0]["source_offset"], 99);
    assert_eq!(json["artifacts"][0]["parent_offset"], 42);
    assert_eq!(json["artifacts"][0]["length"], 4);
}

#[test]
fn a_short_save_is_refused_not_misrecorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");

    let err = store
        .save(
            "000000.jpg",
            Cursor::new(b"ab".to_vec()), // two bytes where three were validated
            Provenance {
                stage: "carve",
                format: "jpeg",
                source_offset: 0,
                length: 3,
                confidence: "contiguous-carve",
                parent_offset: None,
            },
        )
        .expect_err("a truncated artifact must never be recorded as recovered");
    assert!(err.to_string().contains("expected 3 bytes"));
}
