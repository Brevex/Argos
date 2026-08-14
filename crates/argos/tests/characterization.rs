//! What the tool does today, recorded so a change to it is visible.
//!
//! This is not a test of correctness. Nothing here asserts that a recovery is
//! *right* — the suites that do that are `recovery_rate.rs` for fabrication and
//! `cli.rs` for end-to-end behaviour. This one records the output of a whole
//! scan over one planted medium and fails when it changes, whatever the change
//! is and whichever direction it goes in.
//!
//! It exists so that a refactor can be told apart from a behaviour change. A
//! cleanup that leaves this snapshot untouched moved code; one that alters it
//! moved recovery, and belongs in a different review.
//!
//! # Updating the snapshot
//!
//! ```text
//! ARGOS_UPDATE_SNAPSHOT=1 cargo test -p argos --test characterization
//! ```
//!
//! Do this only when the change to behaviour is the point. Read the diff first:
//! every line of it is a statement about what the tool now recovers.

use std::path::Path;
use std::process::Command;

use argos_carve::fixture::{Disk, Jpeg, icon_png, photo_jpeg, png, truncated};

/// Fragment grid the medium and the search share, so a planted split lands
/// where the search looks for one.
const BLOCK: usize = argos_carve::classify::BLOCK_BYTES;

/// Size of the planted medium. Large enough to hold every case on its own
/// block-aligned neighbourhood, small enough to scan in seconds.
const DISK_BYTES: usize = 8 * 1024 * 1024;

/// Where the snapshot lives, relative to this crate.
const SNAPSHOT: &str = "tests/characterization/manifest.snapshot.json";

/// Builds the medium every case is planted on.
///
/// Offsets are fixed and block-aligned. The filler carries neither `0xFF` nor
/// the PNG lead byte, so every signature hit in the result came from something
/// planted here.
fn planted_medium() -> Vec<u8> {
    // Contiguous: the simplest recovery there is.
    let plain = Jpeg::new().with_entropy_bytes(3072).build();
    // Carries the two properties that decide standing: a camera and a date.
    let dated = Jpeg::new()
        .with_entropy_bytes(3072)
        .with_capture("Argos", "Fixture One", "2016:07:04 11:22:33", (1600, 1200))
        .build();
    // A thumbnail inside a photograph: two images, one of them nested.
    let thumbnailed = Jpeg::new()
        .with_entropy_bytes(4096)
        .with_exif_thumbnail(Jpeg::new().with_entropy_bytes(512).build())
        .build();
    let flat_png = png(64, 48);
    // An asset of the kind a used disk holds most of.
    let icon = icon_png(32, 7);

    // Fragmented in two, split on the block grid. The photographs here are
    // full frames rather than the tiny fixtures above: a fragment has to span
    // whole blocks for a split to land on the grid the search steps on.
    let two = photo_jpeg(640, 480, 0x51);
    let two_split = split_at_block(two.len(), 2);
    // Fragmented in three.
    let three = photo_jpeg(800, 600, 0x77);
    let three_split = split_at_block(three.len(), 3);
    // A photograph whose remainder was overwritten: header and part of the
    // scan survive, the rest is filler.
    let overwritten = truncated(&photo_jpeg(640, 480, 0x93), 2 * BLOCK);

    let mut disk = Disk::filled(DISK_BYTES)
        .with(0x01_0000, &plain)
        .with(0x08_0000, &dated)
        .with(0x10_0000, &thumbnailed)
        .with(0x18_0000, &flat_png)
        .with(0x40_0000, &overwritten)
        .with(0x48_0000, &icon);

    // Two fragments, a gap between them.
    disk = disk
        .with(0x20_0000, &two[..two_split[0]])
        .with(0x28_0000, &two[two_split[0]..]);

    // Three fragments, out of order in the address space, which is what the
    // graph walk exists for.
    disk = disk
        .with(0x30_0000, &three[..three_split[0]])
        .with(0x38_0000, &three[three_split[0]..three_split[1]])
        .with(0x3A_0000, &three[three_split[1]..]);

    disk.into_bytes()
}

/// Block-aligned split points that cut `len` bytes into `pieces` parts.
///
/// # Panics
///
/// Panics when the image is too short to give every piece a whole block. A
/// split off the grid is a fixture that cannot exercise the search, and it
/// would record an absence of recovery as though it were a property of the
/// tool.
fn split_at_block(len: usize, pieces: usize) -> Vec<usize> {
    let step = (len / pieces / BLOCK) * BLOCK;
    assert!(
        step > 0,
        "a {len}-byte image cannot be cut into {pieces} block-aligned pieces of {BLOCK} bytes"
    );
    (1..pieces).map(|i| step * i).collect()
}

/// The manifest with everything that varies between machines removed.
///
/// `tool_version` moves with the crate version and `source` is an absolute
/// path in a temporary directory; neither says anything about what was
/// recovered. Everything else is kept, including counts that a change to the
/// pipeline would move.
fn normalized(manifest: &Path) -> serde_json::Value {
    let bytes = std::fs::read(manifest).expect("read the manifest the scan wrote");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("manifest is JSON");
    let object = value.as_object_mut().expect("manifest is an object");
    object.remove("tool_version");
    object.remove("source");
    value
}

#[test]
#[cfg_attr(miri, ignore = "spawns the compiled binary")]
fn a_scan_of_the_planted_medium_produces_what_it_produced_before() {
    let dir = tempfile::tempdir().expect("temp dir");
    let image = dir.path().join("planted.img");
    std::fs::write(&image, planted_medium()).expect("write the planted medium");
    let out = dir.path().join("session");

    let output = Command::new(env!("CARGO_BIN_EXE_argos"))
        .arg("scan")
        .arg(&image)
        .arg("--out")
        .arg(&out)
        // Every setting is pinned. A snapshot taken under the machine's own
        // parallelism or under a wall-clock budget would move on a busier
        // machine, and a snapshot that moves on its own records nothing.
        .args(["--jobs", "1"])
        .args(["--min-long-side", "0"])
        .args(["--reassembly-budget", "0"])
        .arg("--previews")
        .output()
        .expect("run argos scan");
    assert!(
        output.status.success(),
        "the scan failed, so there is nothing to characterize:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = normalized(&out.join("manifest.json"));
    let rendered = serde_json::to_string_pretty(&actual).expect("render the snapshot");
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT);

    if std::env::var_os("ARGOS_UPDATE_SNAPSHOT").is_some() || !snapshot.exists() {
        std::fs::create_dir_all(snapshot.parent().expect("snapshot has a parent"))
            .expect("create the snapshot directory");
        std::fs::write(&snapshot, format!("{rendered}\n")).expect("write the snapshot");
        assert!(
            std::env::var_os("ARGOS_UPDATE_SNAPSHOT").is_some(),
            "no snapshot existed, so one was recorded at {}. Read it, and commit it if it \
             describes what this build should do.",
            snapshot.display()
        );
        return;
    }

    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot).expect("read the snapshot"))
            .expect("the snapshot is JSON");
    assert_eq!(
        actual, expected,
        "the scan no longer produces what the snapshot records.\n\nThis is a behaviour change, \
         not a refactor. If it is intended, rerun with ARGOS_UPDATE_SNAPSHOT=1 and read the diff \
         line by line before committing it.\n\nproduced:\n{rendered}"
    );
}
