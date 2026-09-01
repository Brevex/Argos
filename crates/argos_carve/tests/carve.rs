//! Phase 2 exit criterion: on a synthetic disk with no filesystem referencing
//! them, every contiguous image is recovered bit-identical, and corruption
//! never breaks the scan of what follows.

use std::io::Cursor;

use argos_carve::fixture::{
    Disk, Jpeg, exif_payload, png, truncated, with_flipped_byte, with_u16_be, with_u32_be,
    zero_filled,
};
use argos_carve::{Carver, Finding, Format, Scratch, Verdict, jpeg};
use argos_core::{ByteOffset, Confidence};

/// Recovers the byte range a finding claims, from the source itself.
fn extract(disk: &[u8], finding: &Finding) -> Vec<u8> {
    let start = usize::try_from(finding.offset.get()).expect("test offsets fit usize");
    let len = usize::try_from(finding.length).expect("test lengths fit usize");
    disk[start..start + len].to_vec()
}

#[test]
fn every_contiguous_image_is_recovered_bit_identical() {
    let plain = Jpeg::new().build();
    let restarts = Jpeg::new().with_restart_interval(4).build();
    let picture = png(64, 48);
    let disk = Disk::filled(2 * 1024 * 1024)
        .with(1_003, &plain) // deliberately unaligned
        .with(300_000, &picture)
        .with(1_500_000, &restarts)
        .into_bytes();

    let scan = Carver::new()
        .scan(&mut Cursor::new(&disk))
        .expect("in-memory scan");

    assert_eq!(scan.rejected, 0);
    assert_eq!(scan.findings.len(), 3);
    let expected: [(u64, &[u8], Format); 3] = [
        (1_003, &plain, Format::Jpeg),
        (300_000, &picture, Format::Png),
        (1_500_000, &restarts, Format::Jpeg),
    ];
    for (finding, (offset, original, format)) in scan.findings.iter().zip(expected) {
        assert_eq!(finding.offset, ByteOffset::new(offset));
        assert_eq!(finding.format, format);
        assert_eq!(finding.confidence, Confidence::ContiguousCarve);
        assert_eq!(
            extract(&disk, finding),
            original,
            "recovered bytes must match the placed image bit for bit"
        );
    }
}

#[test]
fn corrupt_candidates_are_rejected_without_breaking_the_scan() {
    // Corrupt the APP0 length field: the segment walk derails deterministically.
    let broken_jpeg = with_flipped_byte(&Jpeg::new().build(), 4);
    let broken_png = with_flipped_byte(&png(32, 32), 40); // breaks the IHDR crc
    let survivor = png(16, 16);
    let disk = Disk::filled(512 * 1024)
        .with(10_000, &broken_jpeg)
        .with(200_000, &broken_png)
        .with(400_000, &survivor)
        .into_bytes();

    let scan = Carver::new()
        .scan(&mut Cursor::new(&disk))
        .expect("in-memory scan");

    assert_eq!(scan.rejected, 2);
    assert_eq!(scan.findings.len(), 1);
    assert_eq!(scan.findings[0].offset, ByteOffset::new(400_000));
    assert_eq!(extract(&disk, &scan.findings[0]), survivor);
}

#[test]
fn exif_thumbnail_is_a_separate_lower_tier_finding() {
    let thumb = Jpeg::new().with_entropy_bytes(256).build();
    let parent = Jpeg::new().with_exif_thumbnail(thumb.clone()).build();
    let disk = Disk::filled(256 * 1024).with(50_000, &parent).into_bytes();

    let scan = Carver::new()
        .scan(&mut Cursor::new(&disk))
        .expect("in-memory scan");

    let full: Vec<_> = scan
        .findings
        .iter()
        .filter(|f| f.parent.is_none())
        .collect();
    let thumbs: Vec<_> = scan
        .findings
        .iter()
        .filter(|f| f.parent.is_some())
        .collect();
    assert_eq!(full.len(), 1);
    assert_eq!(thumbs.len(), 1);
    assert_eq!(thumbs[0].confidence, Confidence::PartialOrThumbnail);
    assert_eq!(thumbs[0].parent, Some(ByteOffset::new(50_000)));
    assert_eq!(
        extract(&disk, thumbs[0]),
        thumb,
        "the embedded thumbnail must be recovered bit-identical"
    );
}

#[test]
fn thumbnail_survives_a_parent_that_breaks_after_the_app1_segment() {
    let thumb = Jpeg::new().with_entropy_bytes(256).build();
    let parent = Jpeg::new().with_exif_thumbnail(thumb.clone()).build();
    // Truncate the parent late in its entropy stream: APP1 stays intact.
    let broken_parent = truncated(&parent, parent.len() - 100);
    let disk = Disk::filled(256 * 1024)
        .with(50_000, &broken_parent)
        .into_bytes();

    let scan = Carver::new()
        .scan(&mut Cursor::new(&disk))
        .expect("in-memory scan");

    assert_eq!(scan.rejected, 1, "the parent itself must be rejected");
    assert_eq!(scan.findings.len(), 1);
    // The same extent is reached through the parent's EXIF and by direct
    // carving; the merged finding keeps the stronger tier (it validated as a
    // standalone contiguous image) plus the parent provenance.
    assert_eq!(scan.findings[0].confidence, Confidence::ContiguousCarve);
    assert_eq!(scan.findings[0].parent, Some(ByteOffset::new(50_000)));
    assert_eq!(extract(&disk, &scan.findings[0]), thumb);
}

#[test]
fn validator_reports_the_exact_length_despite_trailing_garbage() {
    let image = Jpeg::new().build();
    let mut buffer = image.clone();
    buffer.extend_from_slice(&[0x11; 4096]);

    let verdict = jpeg::validate(
        &mut Cursor::new(&buffer),
        ByteOffset::new(0),
        buffer.len() as u64,
        &mut Scratch::new(),
    )
    .expect("in-memory validation");

    assert_eq!(
        verdict,
        Verdict::Complete {
            length: image.len() as u64,
            thumbnail: None,
        }
    );
}

#[test]
fn truncation_at_every_segment_boundary_is_corrupt_never_a_panic() {
    let jpeg_bytes = Jpeg::new().with_entropy_bytes(64).build();
    let png_bytes = png(8, 8);
    let mut scratch = Scratch::new();

    for (bytes, is_png) in [(&jpeg_bytes, false), (&png_bytes, true)] {
        for keep in 0..bytes.len() {
            let cut = truncated(bytes, keep);
            let verdict = if is_png {
                argos_carve::png::validate(
                    &mut Cursor::new(&cut),
                    ByteOffset::new(0),
                    cut.len() as u64,
                    &mut scratch,
                )
            } else {
                jpeg::validate(
                    &mut Cursor::new(&cut),
                    ByteOffset::new(0),
                    cut.len() as u64,
                    &mut scratch,
                )
            }
            .expect("in-memory validation never raises I/O errors");
            assert!(
                matches!(verdict, Verdict::Corrupt { .. }),
                "a truncation at byte {keep} must be corrupt, not complete"
            );
        }
    }
}

#[test]
fn signature_straddling_a_window_boundary_is_still_found() {
    // Place a PNG so its 8-byte signature crosses the 4 MiB window edge.
    let picture = png(16, 16);
    let window = 4 * 1024 * 1024;
    let offset = window - 3;
    let disk = Disk::filled(window + 128 * 1024)
        .with(offset, &picture)
        .into_bytes();

    let scan = Carver::new()
        .scan(&mut Cursor::new(&disk))
        .expect("in-memory scan");

    assert_eq!(scan.findings.len(), 1);
    assert_eq!(scan.findings[0].offset, ByteOffset::new(offset as u64));
    assert_eq!(extract(&disk, &scan.findings[0]), picture);
}

#[test]
fn overflowed_length_fields_are_corrupt_never_a_hang_or_panic() {
    let mut scratch = Scratch::new();

    // JPEG: APP0 length field (bytes 4..6) claiming the u16 maximum.
    let jpeg_overflow = with_u16_be(&Jpeg::new().build(), 4, u16::MAX);
    let verdict = jpeg::validate(
        &mut Cursor::new(&jpeg_overflow),
        ByteOffset::new(0),
        jpeg_overflow.len() as u64,
        &mut scratch,
    )
    .expect("in-memory validation");
    assert!(matches!(verdict, Verdict::Corrupt { .. }));

    // PNG: IHDR chunk length field (bytes 8..12) claiming the spec maximum.
    let png_overflow = with_u32_be(&png(8, 8), 8, 0x7FFF_FFFF);
    let verdict = argos_carve::png::validate(
        &mut Cursor::new(&png_overflow),
        ByteOffset::new(0),
        png_overflow.len() as u64,
        &mut scratch,
    )
    .expect("in-memory validation");
    assert!(matches!(verdict, Verdict::Corrupt { .. }));
}

#[test]
fn zero_filled_regions_yield_no_findings() {
    let scan = Carver::new()
        .scan(&mut Cursor::new(zero_filled(256 * 1024)))
        .expect("in-memory scan");
    assert_eq!(scan.findings.len(), 0);
    assert_eq!(scan.rejected, 0, "zeros contain no signatures to reject");
}

#[test]
fn cyclic_exif_ifd_chain_terminates_without_a_thumbnail() {
    let thumb = Jpeg::new().with_entropy_bytes(64).build();
    let payload = exif_payload(&thumb, true);
    // The walker sees the TIFF bytes after the "Exif\0\0" prefix; a chain
    // whose IFD0 next-pointer cycles back to IFD0 must terminate with no
    // thumbnail, because IFD0 carries no thumbnail tags.
    assert_eq!(argos_carve::jpeg::exif::thumbnail(&payload[6..]), None);
}

// what a recovered image says about itself

#[test]
fn a_recovered_image_reports_the_camera_and_the_moment() {
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_capture(
            "NIKON CORPORATION",
            "NIKON D80",
            "2009:07:14 16:22:05",
            (3872, 2592),
        )
        .build();

    let found = argos_carve::metadata(&jpeg);

    assert_eq!(found.make.as_deref(), Some("NIKON CORPORATION"));
    assert_eq!(found.model.as_deref(), Some("NIKON D80"));
    assert_eq!(found.taken.as_deref(), Some("2009:07:14 16:22:05"));
    assert_eq!(found.pixels, Some((3872, 2592)));
}

#[test]
fn a_photograph_that_lost_its_picture_still_says_when_it_was_taken() {
    // The reason this is read from the artifact rather than from a whole file:
    // the description sits ahead of the picture data, so it survives a frame
    // whose scan was overwritten — and a partial recovery is exactly the case
    // where a person most needs to know which photograph it was.
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_capture(
            "Canon",
            "Canon PowerShot A590",
            "2011:12:25 09:03:41",
            (3264, 2448),
        )
        .with_entropy_bytes(8192)
        .build();
    let prefix = argos_carve::fixture::truncated(&jpeg, jpeg.len() / 3);

    let found = argos_carve::metadata(&prefix);

    assert_eq!(found.taken.as_deref(), Some("2011:12:25 09:03:41"));
    assert_eq!(found.model.as_deref(), Some("Canon PowerShot A590"));
}

#[test]
fn truncation_at_every_byte_of_a_described_image_never_panics() {
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_capture("Sony", "DSC-W180", "2010:01:01 00:00:00", (100, 200))
        .build();
    for keep in 0..jpeg.len().min(600) {
        let _ = argos_carve::metadata(&jpeg[..keep]);
    }
}

#[test]
fn a_description_whose_offsets_point_outside_it_yields_nothing() {
    // Every offset in a TIFF header comes from the medium. One that points
    // past the payload must read as absent, never as whatever happens to sit
    // there (A-UNTRUSTED-ONDISK).
    let payload =
        argos_carve::fixture::exif_capture("Make", "Model", "2009:01:01 00:00:00", (4, 4));
    let overflowing = argos_carve::fixture::with_u32_be(&payload, 6 + 4, u32::MAX);

    assert!(argos_carve::jpeg::exif::metadata(&overflowing[6..]).is_empty());
    for at in 0..payload.len() {
        let _ = argos_carve::jpeg::exif::metadata(&payload[..at]);
    }
}

#[test]
fn a_cyclic_ifd_chain_terminates_without_a_description() {
    // IFD0 pointing at itself through the Exif-IFD tag: the walk is bounded by
    // a count, so it stops whatever the pointers say.
    let payload =
        argos_carve::fixture::exif_capture("Make", "Model", "2009:01:01 00:00:00", (4, 4));
    let mut cyclic = payload.clone();
    // The Exif-IFD pointer is the third IFD0 entry's value: 6 (Exif header)
    // + 8 (TIFF header) + 2 (count) + 2 * 12 (entries) + 8 (into the entry).
    let at = 6 + 8 + 2 + 24 + 8;
    cyclic[at..at + 4].copy_from_slice(&8_u32.to_le_bytes());

    let found = argos_carve::jpeg::exif::metadata(&cyclic[6..]);

    // It terminates; whatever it read is bounded and no loop occurred.
    assert!(found.taken.is_none(), "a cycle cannot reach the Exif IFD");
}
