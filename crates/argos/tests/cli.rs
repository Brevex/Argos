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
