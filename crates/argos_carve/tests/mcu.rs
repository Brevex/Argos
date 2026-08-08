//! The entropy decoder is the oracle every reassembly answer rests on, so what
//! matters is that it says "complete" for exactly the real image and for
//! nothing else.

use std::io::Cursor;

use argos_carve::Scratch;
use argos_carve::fixture::{Disk, Jpeg, photo_jpeg, truncated};
use argos_carve::mcu::{self, ScanStop};
use argos_core::geometry::ByteOffset;

fn scan_of(bytes: &[u8]) -> mcu::ScanOutcome {
    let mut src = Cursor::new(bytes);
    let mut scratch = Scratch::new();
    mcu::scan(
        &mut src,
        ByteOffset::new(0),
        bytes.len() as u64,
        &mut scratch,
    )
    .expect("in-memory read")
}

#[test]
fn a_real_photograph_decodes_every_mcu_it_declares() {
    let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);

    let outcome = scan_of(&image);

    assert!(outcome.is_complete(), "got {outcome:?}");
    assert_eq!(outcome.stop, ScanStop::Complete);
    assert_eq!(outcome.mcus_decoded, outcome.mcus_required);
    // 320x240 of one component: 40 by 30 blocks.
    assert_eq!(outcome.mcus_across, 40);
    assert_eq!(outcome.mcus_required, 40 * 30);
    assert_eq!(outcome.end.get(), image.len() as u64);
}

#[test]
fn a_colour_photograph_with_subsampling_decodes_completely() {
    let image = argos_carve::fixture::photo_jpeg_rgb(160, 128, 0x0F1E_2D3C_4B5A_6978);

    let outcome = scan_of(&image);

    assert!(outcome.is_complete(), "got {outcome:?}");
    assert!(outcome.mcus_required > 0);
}

#[test]
fn a_truncated_photograph_is_never_complete_and_reports_how_far_it_got() {
    let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
    let cut = truncated(&image, image.len() / 2);

    let outcome = scan_of(&cut);

    assert!(!outcome.is_complete());
    assert_eq!(outcome.stop, ScanStop::Broke);
    assert!(
        outcome.mcus_decoded > 0 && outcome.mcus_decoded < outcome.mcus_required,
        "half an image should decode part of its MCUs, got {outcome:?}"
    );
}

#[test]
fn a_photograph_spliced_onto_unrelated_data_is_never_complete() {
    // This is the case the whole oracle exists for: a header followed by bytes
    // that are not its continuation. The marker grammar accepts such splices
    // routinely; the entropy decoder must not.
    let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
    let noise = Disk::noisy(64 * 1024, 0x5DEE_CE66_D000_0001).into_bytes();
    let mut spliced = image[..image.len() / 2].to_vec();
    spliced.extend_from_slice(&noise);

    let outcome = scan_of(&spliced);

    assert!(
        !outcome.is_complete(),
        "a splice onto noise must never read as a whole image, got {outcome:?}"
    );
    // And the noise must not inflate the progress the search ranks by: the
    // decoder gives up within a fraction of an MCU row of the splice.
    let truth = scan_of(&image);
    assert!(
        outcome.mcus_decoded < truth.mcus_required,
        "spliced progress {} must stay below the real MCU count {}",
        outcome.mcus_decoded,
        truth.mcus_required
    );
}

#[test]
fn noise_alone_decodes_almost_nothing() {
    // Progress has to be ungameable, because the graph walk ranks candidates
    // by it. A block of unrelated data must not look like a long stretch of
    // successfully decoded image.
    let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
    let noise = Disk::noisy(256 * 1024, 0x1357_9BDF_2468_ACE0).into_bytes();
    // Keep the frame headers so the decoder has tables and geometry, then feed
    // it nothing but noise where the scan should be.
    let scan_start = image
        .windows(2)
        .position(|pair| pair == [0xFF, 0xDA])
        .expect("the fixture has a scan");
    let mut headers_then_noise = image[..scan_start].to_vec();
    // Re-emit the SOS segment itself, then noise instead of entropy data.
    let sos_len = usize::from(u16::from_be_bytes([
        image[scan_start + 2],
        image[scan_start + 3],
    ]));
    headers_then_noise.extend_from_slice(&image[scan_start..scan_start + 2 + sos_len]);
    headers_then_noise.extend_from_slice(&noise);

    let outcome = scan_of(&headers_then_noise);

    assert!(!outcome.is_complete());
    // A whole 256 KiB of noise must not decode more MCUs than the real,
    // far smaller, scan does.
    let truth = scan_of(&image);
    assert!(
        outcome.mcus_decoded < truth.mcus_required,
        "noise decoded {} MCUs against a real {}",
        outcome.mcus_decoded,
        truth.mcus_required
    );
}

#[test]
fn restart_markers_out_of_order_break_the_scan() {
    let image = photo_jpeg(256, 128, 0x2468_ACE0_1357_9BDF);
    let outcome = scan_of(&image);
    assert!(outcome.is_complete(), "baseline fixture must decode");

    // The synthetic builder emits cyclic RSTn; scrambling one must be caught.
    let with_restarts = Jpeg::new().with_restart_interval(4).build();
    let broken = argos_carve::fixture::with_flipped_byte(
        &with_restarts,
        with_restarts
            .windows(2)
            .position(|pair| pair[0] == 0xFF && (0xD0..=0xD7).contains(&pair[1]))
            .map(|at| at + 1)
            .expect("the fixture has restart markers"),
    );
    assert!(!scan_of(&broken).is_complete());
}

#[test]
fn a_progressive_frame_is_reported_unsupported_not_guessed_at() {
    // SOF2 is a progressive frame. The decoder must say so rather than fail it
    // as corrupt: "we cannot check this" and "this is not an image" are
    // different answers, and only one of them is honest here.
    let image = photo_jpeg(64, 64, 0xDEAD_BEEF_CAFE_F00D);
    let sof = image
        .windows(2)
        .position(|pair| pair == [0xFF, 0xC0])
        .expect("the fixture has a baseline frame header");
    let progressive = argos_carve::fixture::with_flipped_byte(&image, sof + 1);
    // Flipping produces some other marker; force it to SOF2 explicitly.
    let mut progressive = progressive;
    progressive[sof + 1] = 0xC2;

    assert_eq!(scan_of(&progressive).stop, ScanStop::Unsupported);
}

#[test]
fn an_mcu_index_maps_to_the_pixel_row_it_starts() {
    let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
    let outcome = scan_of(&image);

    assert_eq!(outcome.pixel_row_of(0), 0);
    // One full MCU row in: 40 MCUs across, 8 pixel rows tall.
    assert_eq!(outcome.pixel_row_of(outcome.mcus_across), 8);
    assert_eq!(outcome.pixel_row_of(outcome.mcus_across * 3), 24);
}

#[test]
fn arbitrary_bytes_never_panic_and_never_claim_completeness() {
    let mut disk = Disk::noisy(32 * 1024, 0xFEED_FACE_BEEF_0001).into_bytes();
    // Plant a bare SOI so the parser actually enters the header loop.
    disk[0] = 0xFF;
    disk[1] = 0xD8;

    let outcome = scan_of(&disk);

    assert!(!outcome.is_complete());
}
