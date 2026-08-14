use std::io::Cursor;

use argos_core::artifact::{Artifact, ArtifactSink, Digest};
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::{Confidence, Format, Stage};
use argos_report::{ExtentRecord, Handback, Owner, Store, Summary};

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

/// Nothing recorded about the camera: these tests are about the store, and a
/// borrowed default keeps every one of them from naming a field it ignores.
static NO_CAPTURE: std::sync::LazyLock<argos_core::artifact::Capture> =
    std::sync::LazyLock::new(argos_core::artifact::Capture::default);

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
        deleted: None,
        recovered_name: None,
        source_object: None,
        parent: None,
        pixels: None,
        capture: &NO_CAPTURE,
    }
}

#[test]
fn saved_artifact_is_written_hashed_and_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out"), None).expect("create store");
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
    assert_eq!(record.name.as_deref(), Some("000000.jpg"));
    let written = std::fs::read(dir.path().join("out/000000.jpg")).expect("artifact file");
    assert_eq!(written, b"abc");
}

#[test]
fn manifest_carries_every_record_the_rejection_count_and_the_damage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::create(dir.path().join("out"), None).expect("create store");
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
    // The run's own account of its reach, which is what separates a medium
    // that held nothing more from a run that stopped short of looking.
    let coverage = argos_report::CoverageRecord {
        bytes_swept: 1 << 20,
        dropped_unreadable: 2,
        omitted_assets: 11,
        unattributed_residue: 5,
        ceilings: vec!["reassembly decode budget".to_owned()],
        ..argos_report::CoverageRecord::default()
    };
    let volumes = [argos_report::VolumeRecord {
        kind: "ntfs".to_owned(),
        origin: "residual".to_owned(),
        offset: 1_048_576,
        length: 1 << 30,
        allocation_bytes: 4096,
    }];
    let manifest_path = store
        .finish(Summary {
            tool_version: "9.9.9",
            source: "fixture.img",
            state: "finished",
            rejected_candidates: 7,
            unreadable: &unreadable,
            triage: None,
            coverage: Some(&coverage),
            volumes: &volumes,
            fragmentation: &[],
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
    // The run's reach, so a reader can tell a medium that held nothing from a
    // run that stopped short of looking. Each of these was previously visible
    // only on a console the window discards.
    assert_eq!(json["coverage"]["omitted_assets"], 11);
    assert_eq!(json["coverage"]["dropped_unreadable"], 2);
    assert_eq!(json["coverage"]["unattributed_residue"], 5);
    assert_eq!(json["coverage"]["ceilings"][0], "reassembly decode budget");
    // And the volumes it found them among: a residual anchor is the trace of a
    // filesystem an earlier format left behind.
    assert_eq!(json["volumes"][0]["kind"], "ntfs");
    assert_eq!(json["volumes"][0]["origin"], "residual");
    assert_eq!(json["volumes"][0]["offset"], 1_048_576);
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
    let mut store = Store::create(dir.path().join("out"), None).expect("create store");
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
    let mut store = Store::create(dir.path().join("out"), None).expect("create store");
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

#[cfg(unix)]
#[test]
fn recovered_files_are_given_to_the_account_that_asked() {
    use std::os::unix::fs::MetadataExt;

    // A scan of a raw device runs elevated, so everything it writes is created
    // by the administrator. Nothing here is elevated, so the account that asks
    // is this one — which still exercises the whole path, because handing a
    // file to the account that already owns it is a real `chown` that a
    // filesystem without ownership still refuses.
    let dir = tempfile::tempdir().expect("temp dir");
    let me = std::fs::metadata(dir.path()).expect("stat the temporary directory");
    let owner = Owner::new(me.uid(), Some(me.gid()));

    let out = dir.path().join("out");
    let mut store = Store::create(&out, Some(owner)).expect("create store");
    assert_eq!(
        store.handback(),
        &Handback::Done,
        "a temporary directory belongs to this account already"
    );

    let extents = [ByteRange::new(ByteOffset::new(0), 3)];
    store
        .accept(
            &artifact(&extents, 3, SHA256_ABC),
            &mut Cursor::new(b"abc".to_vec()),
        )
        .expect("save the artifact");
    let manifest = store
        .finish(Summary {
            tool_version: "test",
            source: "fixture",
            state: "completed",
            rejected_candidates: 0,
            unreadable: &[],
            triage: None,
            coverage: None,
            volumes: &[],
            fragmentation: &[],
        })
        .expect("write the manifest");

    for path in [out.join("000000.jpg"), manifest] {
        let meta = std::fs::metadata(&path).expect("stat what was written");
        assert_eq!(
            (meta.uid(), meta.gid()),
            (me.uid(), me.gid()),
            "{} was left belonging to someone else",
            path.display()
        );
    }
}
