//! A surviving sibling as the header a lost fragment never kept.
//!
//! The published technique estimates a fragment's coding parameters from a
//! corpus of camera settings (Uzun & Sencar, IEEE TIFS 10(8), 2015). These
//! tests cover the case that corpus exists to approximate: a file from the same
//! batch is in hand, so the parameters are known exactly.

use argos_carve::fixture::{photo_jpeg, photo_jpeg_rgb, photo_jpeg_with_restarts};
use argos_carve::reassemble::restart_points;
use argos_carve::reference::{Fault, Reference};
use argos_core::ByteOffset;

/// Restart interval the fixtures use: often enough that a fragment of a small
/// photograph contains several re-entry points.
const INTERVAL: u16 = 2;

#[test]
fn a_reference_lends_a_header_that_reproduces_its_own_file() {
    // The identity case, and the one that proves the prefix is cut in the right
    // place: a file's own entropy data grafted onto its own header must be the
    // file again, byte for byte. Anything else means the cut moved bytes.
    let photo = photo_jpeg(160, 120, 0x5EED_0000_0000_0001);
    let reference = Reference::read(&photo).expect("a fixture photo is a usable reference");

    let entropy = &photo[reference.prefix().len()..photo.len() - 2];
    assert_eq!(
        reference.graft(entropy),
        photo,
        "header plus its own entropy plus EOI is the original file"
    );
}

#[test]
fn a_reference_decodes_a_siblings_entropy_data() {
    // The capability itself. Two photographs of one size from one encoder are
    // siblings in the sense that matters: the tables the first declares are the
    // tables the second was coded with, so the first's header decodes the
    // second's data.
    let first = photo_jpeg(160, 120, 0x5EED_0000_0000_0002);
    let second = photo_jpeg(160, 120, 0x5EED_0000_0000_0003);
    let reference = Reference::read(&first).expect("reference");

    let entropy = &second[Reference::read(&second).expect("sibling").prefix().len()..];
    let grafted = reference.graft(&entropy[..entropy.len() - 2]);

    let pixels = argos_carve::decode::decode_jpeg_luma(&grafted)
        .expect("a graft of a sibling's data must decode");
    assert_eq!(
        (pixels.width, pixels.height),
        (160, 120),
        "the frame is the reference's, which is what makes the strip placeable"
    );
}

#[test]
fn an_orphaned_fragment_entered_at_a_restart_marker_decodes_to_a_picture() {
    // The case the whole module exists for: bytes from the middle of a
    // photograph, with no header anywhere on the medium, and nothing before
    // them to decode through. Entered at a restart marker — where the DC
    // predictors reset and the stream is byte-aligned — a decoder can start.
    let photo = photo_jpeg_with_restarts(320, 240, 0x5EED_0000_0000_0004, INTERVAL);
    let reference = Reference::read(&photo).expect("reference");

    // Take the second half of the entropy data and throw the rest away: this
    // is an orphan, with the header and the first fragment gone.
    let scan_start = reference.prefix().len();
    let orphan = &photo[scan_start + (photo.len() - scan_start) / 2..photo.len() - 2];

    let points = restart_points(orphan, ByteOffset::new(0));
    assert!(
        !points.is_empty(),
        "a fixture written with restart markers must contain some"
    );

    let entry = usize::try_from(points[0].get()).expect("an offset inside an in-memory fixture");
    let grafted = reference.graft(&orphan[entry..]);
    let pixels = argos_carve::decode::decode_jpeg_luma(&grafted)
        .expect("an orphan entered at a restart marker must decode");

    assert_eq!((pixels.width, pixels.height), (320, 240));
    assert!(
        pixels.roughness() > 0.0,
        "a decoded orphan must carry picture, not one flat colour"
    );
}

#[test]
fn entering_an_orphan_anywhere_but_a_restart_marker_is_not_offered() {
    // The bound on the technique, stated as a test so it cannot quietly widen.
    // A fragment with no restart marker in it has no point at which a decoder's
    // predictors are known, and none can be invented for it.
    let photo = photo_jpeg(160, 120, 0x5EED_0000_0000_0005);
    let reference = Reference::read(&photo).expect("reference");
    let entropy = &photo[reference.prefix().len()..photo.len() - 2];

    assert!(
        restart_points(entropy, ByteOffset::new(0)).is_empty(),
        "a photograph encoded without DRI offers no re-entry point"
    );
}

#[test]
fn a_progressive_frame_is_refused_as_a_reference() {
    // Each scan of a progressive frame carries its own parameters, so a prefix
    // of one does not decode the data of another. Lending one would produce
    // confident nonsense, which is worse than lending nothing.
    let mut photo = photo_jpeg(64, 64, 0x5EED_0000_0000_0006);
    let sof0 = photo
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .expect("the fixture is baseline");
    photo[sof0 + 1] = 0xC2;

    let error = Reference::read(&photo).expect_err("progressive must be refused");
    assert_eq!(error.fault, Fault::NotSequential);
}

#[test]
fn a_reference_reports_what_is_wrong_with_it_rather_than_guessing() {
    assert_eq!(
        Reference::read(b"not a jpeg at all")
            .expect_err("not jpeg")
            .fault,
        Fault::NotJpeg
    );
    assert_eq!(
        Reference::read(&[0xFF, 0xD8]).expect_err("no scan").fault,
        Fault::NoScan
    );

    // A segment that claims more bytes than the file holds.
    let photo = photo_jpeg(64, 64, 0x5EED_0000_0000_0007);
    let truncated = &photo[..photo.len() / 4];
    let fault = Reference::read(truncated).expect_err("truncated").fault;
    assert!(
        matches!(fault, Fault::Truncated | Fault::NoScan),
        "a cut file is reported as cut, not as a usable header: {fault:?}"
    );
}

#[test]
fn a_colour_reference_lends_its_own_component_geometry() {
    // Three components with 2x2 luma sampling is what a camera writes, and the
    // interleaved-MCU geometry a single-component fixture never poses.
    let photo = photo_jpeg_rgb(96, 64, 0x5EED_0000_0000_0008);
    let reference = Reference::read(&photo).expect("a colour photo is a usable reference");
    assert_eq!(reference.dimensions(), (96, 64));

    let entropy = &photo[reference.prefix().len()..photo.len() - 2];
    assert_eq!(reference.graft(entropy), photo);
}
