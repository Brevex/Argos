//! End-to-end over a session directory: previews, `report` and `export`.
//!
//! These three are what a results view is built on, and each of them exists in
//! the CLI first (`A-CLI-FIRST`). What is checked here is the property that
//! matters forensically: none of them can change, hide or invent a recovery.

use std::path::Path;
use std::process::Command;

use argos_carve::fixture::{Disk, photo_jpeg, png};

/// A fixture disk holding one photograph and one PNG, and its session
/// directory after a scan with the given extra flags.
fn scanned(dir: &Path, extra: &[&str]) -> std::path::PathBuf {
    let disk = Disk::filled(1024 * 1024)
        .with(10_000, &photo_jpeg(320, 240, 0x5EED_0001))
        .with(600_000, &png(32, 24))
        .into_bytes();
    let image = dir.join("fixture.img");
    std::fs::write(&image, &disk).expect("write fixture disk");
    let out = dir.join("session");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        .args(extra)
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

fn manifest_of(session: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(session.join("manifest.json")).expect("manifest"))
        .expect("valid json")
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn previews_are_written_only_when_asked_for() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &[]);

    assert!(
        !session.join("previews").exists(),
        "a forensic tool must not write derived files nobody asked for"
    );
    let manifest = manifest_of(&session);
    for artifact in manifest["artifacts"].as_array().expect("artifacts") {
        assert!(artifact.get("preview").is_none());
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_preview_is_written_per_artifact_and_named_by_its_hash() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &["--previews"]);
    let manifest = manifest_of(&session);

    let artifacts = manifest["artifacts"].as_array().expect("artifacts");
    assert_eq!(artifacts.len(), 2);
    for artifact in artifacts {
        let sha256 = artifact["sha256"].as_str().expect("a recorded hash");
        let preview = artifact["preview"]
            .as_str()
            .unwrap_or_else(|| panic!("{sha256} decoded, so it should have a preview"));
        // Keyed by content hash, exactly like the triage annotations: that is
        // what lets a manifest join a preview to a record without a second
        // index that could disagree with the first.
        assert_eq!(preview, format!("previews/{sha256}.jpg"));

        let bytes = std::fs::read(session.join(preview)).expect("the preview file");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a preview is a JPEG");
        assert!(
            bytes.len() < 64 * 1024,
            "a 256 px preview that is {} bytes is not a preview",
            bytes.len()
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_artifact_that_carves_but_does_not_decode_is_reported_without_a_preview() {
    // A structurally valid JPEG that no decoder can turn into pixels is a real
    // recovery: the marker grammar validated, the bytes are on the medium, and
    // the artifact belongs in the manifest. What it does not get is a
    // thumbnail. A missing preview must never read as a missing artifact.
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("fixture.img");
    let carvable = argos_carve::fixture::Jpeg::new().build();
    std::fs::write(
        &image,
        Disk::filled(512 * 1024)
            .with(10_000, &carvable)
            .into_bytes(),
    )
    .expect("write fixture disk");
    let out = dir.path().join("session");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        .arg("--previews")
        .output()
        .expect("run argos scan");
    assert!(output.status.success());

    let manifest = manifest_of(&out);
    let artifacts = manifest["artifacts"].as_array().expect("artifacts");
    assert_eq!(artifacts.len(), 1, "the artifact is recovered and recorded");
    assert!(
        artifacts[0].get("preview").is_none(),
        "an undecodable artifact has no preview, and says so by omission"
    );
    assert_eq!(
        std::fs::read(out.join("000000.jpg")).expect("recovered jpeg"),
        carvable,
        "the artifact itself is recovered bit-identical"
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn previews_do_not_depend_on_triage_and_change_nothing_about_the_recovery() {
    // Previews and triage share one decode pass. A scan that turns triage off
    // must still render previews, and must recover byte-for-byte what a scan
    // with neither recovers.
    let plain = tempfile::tempdir().expect("temp dir");
    let annotated = tempfile::tempdir().expect("temp dir");
    let bare = scanned(plain.path(), &[]);
    let with_previews = scanned(annotated.path(), &["--no-triage", "--previews"]);

    assert!(
        with_previews.join("previews").is_dir(),
        "previews must not be coupled to triage running"
    );
    for name in ["000000.jpg", "000001.png"] {
        assert_eq!(
            std::fs::read(bare.join(name)).expect("bare artifact"),
            std::fs::read(with_previews.join(name)).expect("annotated artifact"),
            "{name} differs between the two scans"
        );
    }

    // The manifests agree on everything except the annotations.
    let (bare, annotated) = (manifest_of(&bare), manifest_of(&with_previews));
    for (left, right) in bare["artifacts"]
        .as_array()
        .expect("artifacts")
        .iter()
        .zip(annotated["artifacts"].as_array().expect("artifacts"))
    {
        for field in ["sha256", "length", "confidence", "stage", "extents"] {
            assert_eq!(left[field], right[field], "{field} changed");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn report_reads_back_every_artifact_the_scan_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &["--previews"]);

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("report")
        .arg(&session)
        .output()
        .expect("run argos report");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("recovered 2 artifacts"), "{text}");
    assert!(text.contains("000000.jpg"), "{text}");
    assert!(text.contains("000001.png"), "{text}");
    assert!(text.contains("+preview"), "{text}");
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn export_copies_what_was_selected_and_nothing_else() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &["--previews"]);
    let manifest = manifest_of(&session);
    let wanted = manifest["artifacts"][0]["sha256"]
        .as_str()
        .expect("a hash")
        .to_owned();
    let to = dir.path().join("exported");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .args(["export", "--from"])
        .arg(&session)
        .arg("--to")
        .arg(&to)
        .arg("--sha256")
        // A prefix names an artifact, as long as it names only one.
        .arg(&wanted[..12])
        .output()
        .expect("run argos export");
    assert!(
        output.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(to.join("000000.jpg").is_file());
    assert!(
        !to.join("000001.png").exists(),
        "an export must carry only what was selected"
    );
    assert!(to.join("previews").join(format!("{wanted}.jpg")).is_file());

    // The destination's manifest describes exactly what landed in it.
    let exported = manifest_of(&to);
    assert_eq!(exported["artifacts"].as_array().map(Vec::len), Some(1));
    assert_eq!(exported["artifacts"][0]["sha256"], wanted.as_str());
    assert_eq!(exported["source"], manifest["source"]);
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn export_refuses_an_artifact_whose_bytes_changed_since_the_scan() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &[]);

    // Something edited a recovered file after the scan wrote it. Exporting it
    // would put a file in the destination that the manifest misdescribes.
    let tampered = session.join("000000.jpg");
    let mut bytes = std::fs::read(&tampered).expect("recovered jpeg");
    bytes[100] ^= 0xFF;
    std::fs::write(&tampered, &bytes).expect("tamper");

    let to = dir.path().join("exported");
    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .args(["export", "--from"])
        .arg(&session)
        .arg("--to")
        .arg(&to)
        .output()
        .expect("run argos export");

    assert!(
        !output.status.success(),
        "an export that silently carried altered evidence must not succeed"
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("refused   000000.jpg"), "{text}");
    assert!(
        !to.join("000000.jpg").exists(),
        "the altered artifact must not be left in the destination"
    );
    // The untouched artifact still exports: one bad file does not deny the
    // examiner the rest.
    assert!(to.join("000001.png").is_file());
    let exported = manifest_of(&to);
    assert_eq!(exported["artifacts"].as_array().map(Vec::len), Some(1));
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn an_ambiguous_selection_is_refused_rather_than_guessed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let session = scanned(dir.path(), &[]);
    let to = dir.path().join("exported");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .args(["export", "--from"])
        .arg(&session)
        .arg("--to")
        .arg(&to)
        .args(["--sha256", "abc"])
        .output()
        .expect("run argos export");

    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("too short"), "{text}");
}
