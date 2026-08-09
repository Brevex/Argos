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
fn excluding_assets_leaves_them_unwritten_and_still_recorded() {
    // The one option that lets a label decide what reaches the output
    // directory, and the condition that keeps it defensible: what it omits, it
    // still accounts for. A record with `written: false` describes bytes that
    // are on the medium at the extents beside it, so the manifest remains a
    // complete statement of what the scan found (A-TRIAGE-NOT-VERDICT).
    // Flat blocks of a few colours with hard edges between them: an icon, and
    // the shape every asset rule was measured against. The generator's own
    // `png` helper draws a dithered gradient, which is genuinely unclear and
    // would prove nothing here.
    let icon = argos_carve::fixture::icon_png(96, 0);
    let disk = Disk::noisy(1024 * 1024, 0x5EED_0002)
        .with(20_000, &icon)
        .into_bytes();

    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");

    let run = |out: &std::path::Path, exclude: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_argos"));
        command.arg("scan").arg(&image).arg("--out").arg(out);
        if exclude {
            command.arg("--exclude-assets");
        }
        let output = command.output().expect("run argos scan");
        assert!(
            output.status.success(),
            "scan failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("manifest"))
                .expect("valid json");
        manifest
    };

    let kept = dir.path().join("kept");
    let trimmed = dir.path().join("trimmed");
    let all = run(&kept, false);
    let some = run(&trimmed, true);

    let all_records = all["artifacts"].as_array().expect("artifacts");
    let some_records = some["artifacts"].as_array().expect("artifacts");
    assert!(
        !all_records.is_empty(),
        "the fixture must recover something"
    );
    assert_eq!(
        all_records.len(),
        some_records.len(),
        "excluding assets must not change how many artifacts are accounted for"
    );

    // The flat PNG is an asset by every rule there is, so this run omitted it.
    let omitted: Vec<&serde_json::Value> = some_records
        .iter()
        .filter(|record| record["written"] == serde_json::json!(false))
        .collect();
    assert_eq!(omitted.len(), 1, "the planted icon should be the omission");
    let record = omitted[0];
    assert_eq!(record["triage_label"], "synthetic-asset");
    assert!(record["omitted_because"].is_string());
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

    // And the bytes really are absent from the directory while present in the
    // other run's.
    assert!(kept.join("000000.png").is_file());
    assert!(!trimmed.join("000000.png").exists());
}
