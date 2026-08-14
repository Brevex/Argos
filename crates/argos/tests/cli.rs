//! End-to-end: the binary scans a fixture disk and recovers every image.

use std::process::Command;

use argos_carve::fixture::{Disk, Jpeg, png};

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn scan_recovers_all_images_bit_identical_end_to_end() {
    let jpeg_bytes = Jpeg::new().build();
    let png_bytes = png(32, 24);
    let disk = Disk::filled(1024 * 1024)
        .with(10_000, &jpeg_bytes)
        .with(600_000, &png_bytes)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");
    let out = dir.path().join("recovered");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        // The fixtures are a few dozen pixels across on purpose, so the size
        // floor is turned off here: this test is about what is recovered, and
        // the floor has a test of its own.
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let recovered_jpeg = std::fs::read(out.join("000000.jpg")).expect("recovered jpeg");
    let recovered_png = std::fs::read(out.join("000001.png")).expect("recovered png");
    assert_eq!(recovered_jpeg, jpeg_bytes);
    assert_eq!(recovered_png, png_bytes);

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    assert_eq!(manifest["artifacts"].as_array().map(Vec::len), Some(2));
    assert_eq!(manifest["rejected_candidates"], 0);
    // Provenance a reader can replay against the source.
    assert_eq!(manifest["artifacts"][0]["extents"][0]["offset"], 10_000);
    assert_eq!(manifest["artifacts"][0]["confidence"], "contiguous-carve");
    assert_eq!(manifest["artifacts"][0]["stage"], "carve");
    assert!(manifest["unreadable"].as_array().is_some_and(Vec::is_empty));
    // The run's own account of its reach, so a later reader can tell a medium
    // that held nothing from a run that stopped short of looking.
    assert_eq!(manifest["coverage"]["omitted_assets"], 0);
    assert_eq!(manifest["coverage"]["dropped_unreadable"], 0);
    assert_eq!(manifest["coverage"]["unattributed_residue"], 0);
    assert!(manifest["coverage"]["bytes_swept"].as_u64().unwrap_or(0) > 0);
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_range_scans_its_neighbourhood_and_reports_only_what_it_covered() {
    // The anchor-directed mode: files deleted together were written together,
    // so the surroundings of a photograph that did come back are where the rest
    // of its batch is. A whole disk is hours; a neighbourhood is minutes, and
    // can afford settings the whole disk cannot.
    let inside = Jpeg::new().with_entropy_bytes(2048).build();
    let outside = Jpeg::new().with_entropy_bytes(4096).build();
    let disk = Disk::filled(4 * 1024 * 1024)
        .with(100_000, &outside)
        .with(2_000_000, &inside)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");
    let out = dir.path().join("recovered");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        .args(["--min-long-side", "0"])
        .args(["--range", "1_500_000..2_500_000"])
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "ranged scan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    let offsets: Vec<u64> = manifest["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .filter_map(|record| record["source_offset"].as_u64())
        .collect();
    assert_eq!(
        offsets,
        vec![2_000_000],
        "only the image inside the range may be reported: {offsets:?}"
    );
    // And the account describes the range, not the medium.
    assert_eq!(manifest["coverage"]["bytes_swept"], 1_000_000);
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_range_that_covers_nothing_is_refused_rather_than_scanned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, Disk::filled(64 * 1024).into_bytes()).expect("write fixture disk");

    for bad in ["2000..1000", "notarange", "10..abc"] {
        let output = Command::new(env!("CARGO_BIN_EXE_argos"))
            .arg("scan")
            .arg(&image)
            .arg("--out")
            .arg(dir.path().join(format!("out-{}", bad.len())))
            .args(["--range", bad])
            .output()
            .expect("run argos scan");
        assert!(
            !output.status.success(),
            "{bad} must be refused rather than interpreted: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn acquire_copies_a_medium_and_the_copy_scans_to_the_same_images() {
    // The point of acquiring: everything afterwards works from the image, so a
    // failing medium is read exactly once. The property that makes that safe is
    // that the copy is the medium — bit for bit — and recovers the same files.
    let jpeg_bytes = Jpeg::new().build();
    let disk = Disk::filled(512 * 1024)
        .with(20_000, &jpeg_bytes)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("source.img");
    std::fs::write(&source, &disk).expect("write fixture disk");
    let copy = dir.path().join("copy.img");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("acquire")
        .arg(&source)
        .arg("--to")
        .arg(&copy)
        .output()
        .expect("run argos acquire");
    assert!(
        output.status.success(),
        "acquire failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(&copy).expect("the acquired image"),
        disk,
        "an acquisition that is not bit-identical is not evidence"
    );

    let out = dir.path().join("recovered");
    let scan = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&copy)
        .arg("--out")
        .arg(&out)
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(scan.status.success());
    assert_eq!(
        std::fs::read(out.join("000000.jpg")).expect("recovered jpeg"),
        jpeg_bytes
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn acquire_refuses_to_overwrite_an_image_that_already_exists() {
    // An acquisition that silently overwrote an earlier one would destroy the
    // only copy of a medium that may no longer be readable.
    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("source.img");
    std::fs::write(&source, Disk::filled(64 * 1024).into_bytes()).expect("write fixture disk");
    let copy = dir.path().join("copy.img");
    std::fs::write(&copy, b"an earlier acquisition").expect("write the existing image");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("acquire")
        .arg(&source)
        .arg("--to")
        .arg(&copy)
        .output()
        .expect("run argos acquire");

    assert!(!output.status.success(), "an existing image must be kept");
    assert_eq!(
        std::fs::read(&copy).expect("the earlier image"),
        b"an earlier acquisition",
        "and it must be untouched"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_single_worker_scan_recovers_exactly_what_a_parallel_one_does() {
    let jpeg_bytes = Jpeg::new().build();
    let png_bytes = png(16, 16);
    let disk = Disk::filled(2 * 1024 * 1024)
        .with(70_000, &jpeg_bytes)
        .with(1_500_000, &png_bytes)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");

    let manifests: Vec<serde_json::Value> = ["1", "8"]
        .iter()
        .map(|jobs| {
            let out = dir.path().join(format!("recovered-{jobs}"));
            let output = Command::new(env!("CARGO_BIN_EXE_argos"))
                .args(["scan"])
                .arg(&image)
                .arg("--out")
                .arg(&out)
                .args(["--jobs", jobs])
                .output()
                .expect("run argos scan");
            assert!(
                output.status.success(),
                "scan with {jobs} jobs failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut manifest: serde_json::Value = serde_json::from_slice(
                &std::fs::read(out.join("manifest.json")).expect("manifest"),
            )
            .expect("valid json");
            // The source description embeds the output-specific path.
            manifest["source"] = serde_json::Value::Null;
            manifest
        })
        .collect();

    assert_eq!(
        manifests[0], manifests[1],
        "the manifest must not depend on how many workers ran"
    );
    let recovered = std::fs::read(dir.path().join("recovered-8/000000.jpg")).expect("jpeg");
    assert_eq!(recovered, jpeg_bytes);
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_scan_with_every_stage_disabled_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, Disk::filled(1024 * 1024).into_bytes()).expect("write fixture disk");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(dir.path().join("out"))
        .args(["--carve-only", "--metadata-only"])
        .output()
        .expect("run argos scan");

    assert!(!output.status.success());
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn scan_refuses_an_output_directory_containing_the_source() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, Disk::filled(4096).into_bytes()).expect("write fixture disk");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(dir.path())
        .output()
        .expect("run argos scan");
    assert!(
        !output.status.success(),
        "an output directory containing the source must be refused"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn the_manifest_carries_triage_labels_and_what_decided_them() {
    let photo = argos_carve::fixture::photo_jpeg(320, 240, 0xC0FF_EE01);
    let disk = Disk::noisy(1024 * 1024, 0x5EED_0001)
        .with(10_000, &photo)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");
    let out = dir.path().join("recovered");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        // The fixtures are a few dozen pixels across on purpose, so the size
        // floor is turned off here: this test is about what is recovered, and
        // the floor has a test of its own.
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
            .expect("valid json");

    // What labelled the scan is named, so a result can be reproduced against
    // that procedure and no other (A-MODEL-PINNED).
    assert_eq!(manifest["triage"]["status"], "scored");
    assert_eq!(
        manifest["triage"]["model_version"],
        argos_classify::RULES_VERSION
    );
    assert_eq!(manifest["triage"]["degraded"], false);

    // The annotation reaches the record, and says what decided it — which is
    // checkable against the image in a way a probability never was. *Which*
    // label a given image earns is gated by the eval harness in
    // `argos_classify`, against a corpus built for it; asserting a particular
    // one here would tie the plumbing test to one fixture's statistics.
    let artifact = &manifest["artifacts"][0];
    let label = artifact["triage_label"].as_str().expect("a label");
    let decided = artifact["triage_decided_by"].as_str().expect("a reason");
    assert!(
        ["photograph", "synthetic-asset", "ambiguous"].contains(&label),
        "unknown label {label}"
    );
    assert_eq!(
        label == "ambiguous",
        decided == "inconclusive",
        "an unclear label and an unclear reason go together: {label} / {decided}"
    );
    assert!(
        artifact["perceptual_hash"].as_str().is_some_and(|hash| {
            hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit())
        }),
        "the perceptual hash must be recorded as 16 hex digits"
    );
    // The recovered bytes are untouched by any of it.
    assert_eq!(
        std::fs::read(out.join("000000.jpg")).expect("artifact"),
        photo
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn disabling_triage_removes_the_labels_and_nothing_else() {
    let photo = argos_carve::fixture::photo_jpeg(320, 240, 0xC0FF_EE02);
    let disk = Disk::noisy(1024 * 1024, 0x5EED_0002)
        .with(10_000, &photo)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");

    let manifests: Vec<serde_json::Value> = [true, false]
        .into_iter()
        .map(|triage| {
            let out = dir.path().join(if triage { "with" } else { "without" });
            let mut command = Command::new(env!("CARGO_BIN_EXE_argos"));
            command.arg("scan").arg(&image).arg("--out").arg(&out);
            if !triage {
                command.arg("--no-triage");
            }
            let output = command.output().expect("run argos scan");
            assert!(output.status.success());
            assert_eq!(
                std::fs::read(out.join("000000.jpg")).expect("artifact"),
                photo,
                "the recovered bytes must not depend on whether triage ran"
            );
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
                .expect("valid json")
        })
        .collect();

    let (with, without) = (&manifests[0], &manifests[1]);
    assert_eq!(
        with["artifacts"].as_array().map(Vec::len),
        without["artifacts"].as_array().map(Vec::len),
        "triage must not change how many artifacts are reported"
    );
    assert_eq!(
        with["artifacts"][0]["sha256"], without["artifacts"][0]["sha256"],
        "triage must not change what was recovered"
    );
    assert!(with["artifacts"][0]["triage_label"].is_string());
    assert!(
        without["artifacts"][0]["triage_label"].is_null(),
        "a scan without triage carries no label"
    );
    assert_eq!(without["triage"]["status"], "disabled");
    assert_eq!(without["triage"]["disabled_reason"], "not requested");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn listing_devices_needs_no_privileges_and_never_fails() {
    // Enumeration is what a user runs before deciding what to scan, so it has
    // to work unprivileged on every platform. A machine that publishes no
    // device list says so and still exits successfully — the user can always
    // name a path.
    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("devices")
        .output()
        .expect("run argos devices");
    assert!(
        output.status.success(),
        "listing devices failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn scanning_an_image_file_warns_about_nothing() {
    // The mount and partition warnings exist for real media. An image file is
    // a snapshot nothing else is writing to, so a scan of one must stay
    // silent — a warning that fires on everything trains users to ignore it.
    let jpeg = Jpeg::new().build();
    let disk = Disk::filled(256 * 1024).with(4096, &jpeg).into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(dir.path().join("recovered"))
        .output()
        .expect("run argos scan");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("warning"),
        "an image-file scan warned about something: {stdout}"
    );
}

/// Fields of the manifest that describe *what was recovered from where*.
///
/// Deliberately not the whole manifest. The classifier score is a softmax over
/// a transcendental, and `exp` is a libm implementation detail — its last
/// digits differ between operating systems for no substantive reason. What
/// must not differ is the recovery: the same bytes, at the same offsets, at
/// the same confidence tier, in the same order.
fn provenance_digest(manifest: &serde_json::Value) -> String {
    use sha2::{Digest as _, Sha256};

    let mut hasher = Sha256::new();
    let artifacts = manifest["artifacts"]
        .as_array()
        .expect("the manifest lists artifacts");
    for artifact in artifacts {
        for field in ["name", "stage", "format", "confidence", "sha256", "length"] {
            hasher.update(artifact[field].to_string().as_bytes());
        }
        for extent in artifact["extents"]
            .as_array()
            .expect("every artifact records its extents")
        {
            hasher.update(extent["offset"].to_string().as_bytes());
            hasher.update(extent["length"].to_string().as_bytes());
        }
    }
    hasher.update(manifest["rejected_candidates"].to_string().as_bytes());
    hasher.update(manifest["scan_state"].to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn the_recovery_is_identical_on_every_platform() {
    // The exit criterion of the multi-OS phase, mechanised. This same constant
    // is checked by the Linux, Windows and macOS legs of the CI matrix, so a
    // HAL or path-handling difference that changed what a scan recovers fails
    // on the platform that changed it — rather than being noticed months later
    // by an examiner comparing two reports.
    const RECORDED: &str = "5adacb28911f9ceddbd717439fad208cd1e4f4e9c398ddb01562b53e034e55b4";

    let mut disk = argos_carve::fixture::Disk::noisy(512 * 1024, 0x9E7A_0001);
    disk = disk.with(4096, &Jpeg::new().build());
    disk = disk.with(120_000, &png(24, 18));
    disk = disk.with(
        300_000,
        &argos_carve::fixture::photo_jpeg(96, 72, 0x9E7A_0002),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, disk.into_bytes()).expect("write fixture disk");
    let out = dir.path().join("recovered");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        .args(["--jobs", "2"])
        // The floor is off: this fixture's images are deliberately tiny, and
        // what is being pinned here is the recovery, not the write policy.
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    assert!(
        !manifest["artifacts"]
            .as_array()
            .expect("artifacts")
            .is_empty(),
        "the fixture must recover something for this to mean anything"
    );

    let digest = provenance_digest(&manifest);
    assert_eq!(
        digest, RECORDED,
        "this platform recovered something different from the recorded result. The fixture is \
         deterministic and the recovery must be too: a difference here is a real difference in \
         what an examiner would be shown on one operating system versus another"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_image_below_the_size_floor_is_recorded_and_not_written() {
    // What a used disk is mostly made of: small derived images. The floor is
    // the only thing about them that is not a matter of opinion, so it is what
    // decides — and the decision costs no evidence, because the record stays.
    let small = argos_carve::fixture::photo_jpeg(96, 72, 0x5EED_0003);
    let large = argos_carve::fixture::photo_jpeg(640, 480, 0x5EED_0004);
    let disk = Disk::noisy(4 * 1024 * 1024, 0x5EED_0002)
        .with(20_000, &small)
        .with(600_000, &large)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");

    let run = |out: &std::path::Path, floor: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_argos"))
            .arg("scan")
            .arg(&image)
            .arg("--out")
            .arg(out)
            .args(["--min-long-side", floor])
            .output()
            .expect("run argos scan");
        assert!(
            output.status.success(),
            "scan failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(out.join("manifest.json")).expect("manifest"),
        )
        .expect("valid json")
    };

    let kept = dir.path().join("kept");
    let floored = dir.path().join("floored");
    let all = run(&kept, "0");
    let some = run(&floored, "300");

    let all_records = all["artifacts"].as_array().expect("artifacts");
    let some_records = some["artifacts"].as_array().expect("artifacts");
    assert!(
        all_records.len() >= 2,
        "the fixture must recover both images: {all_records:?}"
    );
    assert_eq!(
        all_records.len(),
        some_records.len(),
        "a floor must not change how many artifacts are accounted for"
    );

    // Every record carries the picture's size, which is what lets a reader
    // tell a photograph from a cache entry without opening a file.
    for record in all_records {
        assert!(
            record["width"].as_u64().is_some() && record["height"].as_u64().is_some(),
            "an artifact that decoded records its dimensions: {record}"
        );
    }

    let omitted: Vec<&serde_json::Value> = some_records
        .iter()
        .filter(|record| record["written"] == serde_json::json!(false))
        .collect();
    assert_eq!(
        omitted.len(),
        1,
        "only the image under the floor should be left unwritten: {omitted:?}"
    );
    let record = omitted[0];
    assert_eq!(record["width"], 96);
    assert_eq!(record["height"], 72);
    assert_eq!(record["omitted_because"], "below-size-floor");
    assert!(
        record["name"].is_null(),
        "an unwritten artifact names no file"
    );

    // Everything that makes the artifact findable again is still there.
    assert!(
        record["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64)
    );
    assert!(!record["extents"].as_array().expect("extents").is_empty());
    assert!(record["length"].as_u64().is_some_and(|length| length > 0));

    // The one above the floor was written in both runs.
    let written: Vec<&serde_json::Value> = some_records
        .iter()
        .filter(|record| record["written"] == serde_json::json!(true))
        .collect();
    assert!(
        written.iter().any(|record| record["width"] == 640),
        "the 640x480 image belongs in the directory: {written:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_batch_of_photographs_can_be_exported_by_camera_and_date() {
    // The question a person actually has after a scan of a used disk: not
    // "which artifact has hash 4f2c…" but "which of these hundreds of
    // thousands are the ones I lost". A camera and a year answer it; an offset
    // and a byte count do not.
    let lost_first = Jpeg::new()
        .with_capture(
            "NIKON CORPORATION",
            "NIKON D80",
            "2009:07:14 16:22:05",
            (3872, 2592),
        )
        .with_entropy_bytes(3000)
        .build();
    let lost_second = Jpeg::new()
        .with_capture(
            "NIKON CORPORATION",
            "NIKON D80",
            "2009:07:14 16:31:40",
            (3872, 2592),
        )
        .with_entropy_bytes(3100)
        .build();
    let unrelated = Jpeg::new()
        .with_capture("Apple", "iPhone 12", "2021:03:02 11:00:00", (4032, 3024))
        .with_entropy_bytes(3200)
        .build();

    let disk = Disk::filled(1024 * 1024)
        .with(10_000, &lost_first)
        .with(200_000, &lost_second)
        .with(600_000, &unrelated)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");
    let out = dir.path().join("recovered");

    let scan = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos scan");
    assert!(
        scan.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&scan.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    assert_eq!(manifest["artifacts"][0]["camera_model"], "NIKON D80");
    assert_eq!(manifest["artifacts"][0]["taken"], "2009:07:14 16:22:05");

    let exported = dir.path().join("the-batch");
    let export = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("export")
        .args(["--from".as_ref(), out.as_os_str()])
        .args(["--to".as_ref(), exported.as_os_str()])
        .args(["--camera", "nikon"])
        .args(["--taken-from", "2009"])
        .args(["--taken-until", "2009"])
        .output()
        .expect("run argos export");
    assert!(
        export.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&export.stderr)
    );

    let taken: serde_json::Value = serde_json::from_slice(
        &std::fs::read(exported.join("manifest.json")).expect("exported manifest"),
    )
    .expect("valid json");
    let models: Vec<&str> = taken["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .map(|record| record["camera_model"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        models,
        ["NIKON D80", "NIKON D80"],
        "the export must hold that camera's pictures from that year, and only those"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_search_can_be_run_again_from_a_session_without_reading_the_medium_twice() {
    // A scan of a large disk spends its hours sweeping the surface and driving
    // every signature hit through a state machine, and both establish the same
    // fragmentation points every time. Recording them is what lets a longer
    // budget or a lower floor be tried in minutes.
    //
    // The first run is given a floor no photograph on this disk clears, so its
    // search skips the fragmented image entirely. The second run lowers the
    // floor and finds it — from the manifest, with the sweep never repeated.
    let photo = argos_carve::fixture::photo_jpeg(320, 240, 0x5E55_1000_0000_0001);
    let block = argos_carve::classify::BLOCK_BYTES;
    let layout =
        argos_carve::fixture::fragmented(512 * block, &photo, &[8 * block, 40 * block], block);

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &layout.disk).expect("write fixture disk");
    let first = dir.path().join("first");

    let scan = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&first)
        .args(["--min-long-side", "4000"])
        .output()
        .expect("run argos scan");
    assert!(
        scan.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&scan.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(first.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    let points = manifest["fragmentation"]
        .as_array()
        .expect("the manifest must record where the search could be picked up");
    assert!(!points.is_empty());
    assert_eq!(points[0]["offset"], (8 * block) as u64);

    let second = dir.path().join("second");
    let again = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("reassemble")
        .args(["--from".as_ref(), first.as_os_str()])
        .arg(&image)
        .args(["--out".as_ref(), second.as_os_str()])
        .args(["--min-long-side", "0"])
        .output()
        .expect("run argos reassemble");
    assert!(
        again.status.success(),
        "reassemble failed: {}",
        String::from_utf8_lossy(&again.stderr)
    );

    let recovered = std::fs::read(second.join("000000.jpg")).expect("the reassembled photograph");
    assert_eq!(
        recovered, photo,
        "a resumed search must land on the planted bytes exactly, as a scan's would"
    );
    let resumed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(second.join("manifest.json")).expect("manifest"))
            .expect("valid json");
    assert_eq!(resumed["artifacts"][0]["confidence"], "reassembled");
    assert_eq!(
        resumed["artifacts"][0]["extents"].as_array().map(Vec::len),
        Some(2)
    );
}
