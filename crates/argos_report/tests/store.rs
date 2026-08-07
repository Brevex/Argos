use std::io::Cursor;

use argos_core::artifact::{Artifact, ArtifactSink, Digest};
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::{Confidence, Format, Stage};
use argos_report::{ExtentRecord, Store, Summary};

/// SHA-256 of the ASCII bytes `abc` — the FIPS 180-2 known-answer vector.
const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// SHA-256 of the bytes `[1, 2, 3, 4]`.
const SHA256_1234: &str = "9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a";

fn digest(hex: &str) -> Digest {
    let mut bytes = [0_u8; Digest::LEN];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let text = std::str::from_utf8(pair).expect("ascii hex");
        *byte = u8::from_str_radix(text, 16).expect("hex byte");
    }
    Digest::new(bytes)
}

fn artifact<'a>(extents: &'a [ByteRange], length: u64, sha256: &str) -> Artifact<'a> {
    Artifact {
        format: Format::Jpeg,
        stage: Stage::Carve,
        confidence: Confidence::ContiguousCarve,
        extents,
        length,
        expected_length: None,
        sha256: digest(sha256),
        timestamps: argos_core::Timestamps::default(),
        recovered_name: None,
        source_object: None,
        parent: None,
    }
}

#[test]
fn saved_artifact_is_written_hashed_and_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");
    let extents = [ByteRange::new(ByteOffset::new(1234), 3)];

    let record = store
        .save(
            &artifact(&extents, 3, SHA256_ABC),
            &mut Cursor::new(b"abc".to_vec()),
        )
        .expect("save artifact")
        .clone();

    assert_eq!(record.sha256, SHA256_ABC);
    assert_eq!(record.length, 3);
    assert_eq!(record.source_offset, 1234);
    assert_eq!(record.name, "000000.jpg");
    let written = std::fs::read(dir.path().join("out/000000.jpg")).expect("artifact file");
    assert_eq!(written, b"abc");
}

#[test]
fn manifest_carries_every_record_the_rejection_count_and_the_damage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");
    let extents = [
        ByteRange::new(ByteOffset::new(99), 2),
        ByteRange::new(ByteOffset::new(512), 2),
    ];
    let mut png = artifact(&extents, 4, SHA256_1234);
    png.format = Format::Png;
    png.parent = Some(ByteOffset::new(42));
    png.recovered_name = Some("holiday.png");
    png.source_object = Some(77);
    png.expected_length = Some(6);
    png.timestamps = argos_core::Timestamps {
        created: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000)),
        modified: None,
    };
    store
        .save(&png, &mut Cursor::new(vec![1, 2, 3, 4]))
        .expect("save artifact");

    let unreadable = [ExtentRecord {
        offset: 4096,
        length: 512,
    }];
    let manifest_path = store
        .finish(Summary {
            tool_version: "9.9.9",
            source: "fixture.img",
            state: "finished",
            rejected_candidates: 7,
            unreadable: &unreadable,
        })
        .expect("manifest");
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).expect("manifest file"))
            .expect("valid json");

    assert_eq!(json["tool_version"], "9.9.9");
    assert_eq!(json["source"], "fixture.img");
    assert_eq!(json["rejected_candidates"], 7);
    assert_eq!(json["unreadable"][0]["offset"], 4096);
    assert_eq!(json["artifacts"][0]["name"], "000000.png");
    assert_eq!(json["artifacts"][0]["source_offset"], 99);
    assert_eq!(json["artifacts"][0]["parent_offset"], 42);
    assert_eq!(json["artifacts"][0]["length"], 4);
    assert_eq!(json["artifacts"][0]["recovered_name"], "holiday.png");
    assert_eq!(json["artifacts"][0]["source_object"], 77);
    assert_eq!(json["scan_state"], "finished");
    // A short recovery states what is missing rather than reading as whole.
    assert_eq!(json["artifacts"][0]["expected_length"], 6);
    assert_eq!(json["artifacts"][0]["missing_bytes"], 2);
    assert_eq!(json["artifacts"][0]["created_unix"], 1_600_000_000);
    // Every extent is recorded, so the claim can be replayed against the medium.
    assert_eq!(json["artifacts"][0]["extents"][0]["offset"], 99);
    assert_eq!(json["artifacts"][0]["extents"][1]["offset"], 512);
    assert_eq!(json["artifacts"][0]["extents"][1]["length"], 2);
}

#[test]
fn a_short_save_is_refused_not_misrecorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");
    let extents = [ByteRange::new(ByteOffset::new(0), 3)];

    let err = store
        .save(
            &artifact(&extents, 3, SHA256_ABC),
            // Two bytes where three were validated.
            &mut Cursor::new(b"ab".to_vec()),
        )
        .expect_err("a truncated artifact must never be recorded as recovered");
    assert!(err.to_string().contains("expected 3 bytes"));
}

#[test]
fn bytes_that_do_not_reproduce_the_recovery_digest_are_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out")).expect("create store");
    let extents = [ByteRange::new(ByteOffset::new(0), 3)];

    // Right length, wrong bytes: the medium changed under us, or the extents
    // were resolved against the wrong offset. Either way it is not evidence.
    let err = store
        .accept(
            &artifact(&extents, 3, SHA256_ABC),
            &mut Cursor::new(b"abd".to_vec()),
        )
        .expect_err("an artifact whose hash moved must never be recorded");
    assert!(err.to_string().contains("hash changed"));
    assert!(store.records().is_empty());
    // The refused bytes are not left behind for someone to mistake for output.
    assert!(!dir.path().join("out/000000.jpg").exists());
}
