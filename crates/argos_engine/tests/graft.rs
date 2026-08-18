//! Grafting a surviving header onto fragments that have none.
//!
//! The sweep's contract is narrow and its failure mode is bad: a graft that
//! decodes is not evidence a file was there, so what these assert is that it
//! enters only where a decoder's state is known and reports what it produces
//! as the floor tier it is.

use std::io::Cursor;

use argos_carve::fixture::{Disk, photo_jpeg, photo_jpeg_with_restarts};
use argos_core::Confidence;
use argos_engine::graft::{self, JpegReference};

/// Restart interval the fixtures use.
const INTERVAL: u16 = 2;

/// A photograph with its header and first half removed, planted in noise.
fn orphan_disk(photo: &[u8], reference: &JpegReference, at: usize, len: usize) -> Vec<u8> {
    let scan_start = reference.prefix().len();
    let orphan = &photo[scan_start + (photo.len() - scan_start) / 2..photo.len() - 2];
    Disk::noisy(len, 0x0A0F_0000_0000_0001)
        .with(at, orphan)
        .into_bytes()
}

#[test]
fn a_headerless_fragment_is_recovered_as_pixels_at_the_floor_tier() {
    // The capability: bytes from the middle of a photograph, with no header
    // anywhere on the medium, decoded by lending them a sibling's.
    let photo = photo_jpeg_with_restarts(1024, 768, 0x0A0F_0000_0000_0002, INTERVAL);
    let reference = JpegReference::read(&photo).expect("a fixture photo is a usable reference");
    let disk = orphan_disk(&photo, &reference, 1 << 20, 4 << 20);

    let mut found = Vec::new();
    let entered = graft::sweep(
        &mut Cursor::new(disk.clone()),
        0..disk.len() as u64,
        &reference,
        |grafted| found.push(grafted),
    );

    assert!(entered > 0, "the planted orphan must be entered");
    assert!(
        !found.is_empty(),
        "an orphan entered at a restart marker must decode to a picture"
    );
    for grafted in &found {
        assert_eq!(
            grafted.confidence,
            Confidence::Grafted,
            "the container is this tool's, so the tier is the floor and never rises"
        );
        assert_eq!(
            grafted.dimensions,
            (1024, 768),
            "the frame is the reference's, which is the fact that makes it not a file"
        );
    }
}

#[test]
fn a_medium_with_no_entropy_data_yields_nothing() {
    // Noise classifies as high entropy, not as a JPEG scan, and high entropy
    // is not offered: entering it would draw a frame the decoder happens to
    // accept out of bytes that were never a picture.
    let disk = Disk::noisy(2 << 20, 0x0A0F_0000_0000_0003).into_bytes();
    let photo = photo_jpeg_with_restarts(128, 96, 0x0A0F_0000_0000_0004, INTERVAL);
    let reference = JpegReference::read(&photo).expect("reference");

    let mut found = Vec::new();
    graft::sweep(
        &mut Cursor::new(disk.clone()),
        0..disk.len() as u64,
        &reference,
        |grafted| found.push(grafted),
    );
    assert!(
        found.is_empty(),
        "noise must not be grafted into a picture: {} produced",
        found.len()
    );
}

#[test]
fn a_fragment_with_no_restart_marker_is_not_entered() {
    // The bound on the technique. Without a restart marker there is no offset
    // at which the decoder's predictors are known, and none can be invented.
    let photo = photo_jpeg(1024, 768, 0x0A0F_0000_0000_0005);
    let reference = JpegReference::read(&photo).expect("reference");
    let disk = orphan_disk(&photo, &reference, 1 << 20, 4 << 20);

    let mut found = Vec::new();
    graft::sweep(
        &mut Cursor::new(disk.clone()),
        0..disk.len() as u64,
        &reference,
        |grafted| found.push(grafted),
    );
    assert!(
        found.is_empty(),
        "a photograph encoded without DRI offers no re-entry point"
    );
}

#[test]
fn the_grafted_tier_is_the_floor_of_the_ladder() {
    // Stated as a test because the ladder is the recovery model: a graft must
    // never sort above an artifact whose bytes lay on the medium as reported.
    assert!(Confidence::Grafted < Confidence::PartialOrThumbnail);
    assert!(Confidence::Grafted < Confidence::FsMetadata);
    assert_eq!(Confidence::Grafted.to_string(), "grafted");
}
