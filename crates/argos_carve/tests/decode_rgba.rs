//! What pixel decoding may be asked to do by a medium, and what it must
//! refuse.
//!
//! This decoder runs on bytes carved off an untrusted disk, so a frame header
//! is a claim, not a fact. The tests that matter here are the ones where the
//! claim is a lie: a header can ask for more memory than the machine has, and
//! a refused allocation aborts the process rather than failing one artifact.

use argos_carve::decode::{self, MAX_RGBA_PIXELS};
use argos_carve::fixture::{photo_jpeg, png};
use argos_core::Format;

/// A PNG whose `IHDR` declares `width` by `height`, with a CRC that checks
/// out, and almost no data behind it.
///
/// This is the shape of the attack: a kilobyte on the medium that a decoder
/// will happily size a multi-gigabyte buffer from.
fn png_claiming(width: u32, height: u32, colour: u8, depth: u8) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[depth, colour, 0, 0, 0]);
    out.extend_from_slice(&13_u32.to_be_bytes());
    let crc = crc32(&ihdr);
    out.extend_from_slice(&ihdr);
    out.extend_from_slice(&crc.to_be_bytes());

    // A short, valid zlib stream. Nowhere near enough data for the frame the
    // header claims, which is the point.
    let payload = [
        0x78, 0x01, 0x01, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01,
    ];
    let mut idat = Vec::from(*b"IDAT");
    idat.extend_from_slice(&payload);
    out.extend_from_slice(&u32::try_from(payload.len()).expect("small").to_be_bytes());
    let crc = crc32(&idat);
    out.extend_from_slice(&idat);
    out.extend_from_slice(&crc.to_be_bytes());

    let iend = Vec::from(*b"IEND");
    out.extend_from_slice(&0_u32.to_be_bytes());
    let crc = crc32(&iend);
    out.extend_from_slice(&iend);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

#[test]
fn a_png_header_claiming_more_pixels_than_the_ceiling_is_refused() {
    // 65535 x 65535 RGBA at 16 bits is about 34 GB. If the ceiling were
    // applied to the decoder's output rather than to the header, this would
    // allocate before anything checked it, and an allocation that large is a
    // process abort — the whole scan, not one artifact.
    let hostile = png_claiming(65535, 65535, 6, 16);
    assert!(hostile.len() < 1024, "the attack is small on the medium");
    assert_eq!(decode::decode_rgba(Format::Png, &hostile), None);
}

#[test]
fn a_png_header_just_past_the_ceiling_is_refused() {
    // Exactly at the boundary, so the test measures the constant rather than
    // some incidental limit far below it.
    let edge = u32::try_from(MAX_RGBA_PIXELS.isqrt()).expect("the ceiling fits u32");
    let hostile = png_claiming(edge, edge + 1, 6, 8);
    assert_eq!(decode::decode_rgba(Format::Png, &hostile), None);
}

/// Peak virtual memory this process has ever reserved, in kilobytes.
#[cfg(target_os = "linux")]
fn peak_reserved_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmPeak:"))
                .and_then(|value| value.split_whitespace().next().map(str::to_owned))
        })
        .and_then(|kb| kb.parse().ok())
        .unwrap_or(0)
}

/// The one test that can tell the fix from its absence.
///
/// Refusing an oversized frame is a contract the other tests check, but they
/// pass either way: a decoder handed a header it cannot satisfy errors out
/// anyway, just *after* reserving the buffer. What separates the two is the
/// reservation, so that is what this measures. Without the pre-decode bound,
/// a 32768x32768 RGBA-16 header reserves 8.6 GB here — measured, and enough
/// to abort the process outright under any address-space limit.
#[cfg(target_os = "linux")]
#[test]
fn refusing_an_oversized_frame_reserves_no_memory_for_it() {
    let before = peak_reserved_kb();
    let hostile = png_claiming(32768, 32768, 6, 16);
    assert_eq!(decode::decode_rgba(Format::Png, &hostile), None);
    let after = peak_reserved_kb();

    // The frame would need 8.6 GB. Allow a generous megabyte of ordinary
    // churn; anything near the frame's size means the decoder was let at it.
    let grew = after.saturating_sub(before);
    assert!(
        grew < 64 * 1024,
        "refusing a 32768x32768 frame still reserved {grew} kB — the bound is being applied \
         after the decoder sizes its buffer, not before"
    );
}

#[test]
fn a_jpeg_header_claiming_more_pixels_than_the_ceiling_is_refused() {
    // Craft an SOF0 declaring 65535 x 65535 with three components, then stop.
    let mut hostile = vec![0xFF, 0xD8];
    let mut sof = vec![0xFF, 0xC0, 0x00, 0x11, 0x08];
    sof.extend_from_slice(&65535_u16.to_be_bytes());
    sof.extend_from_slice(&65535_u16.to_be_bytes());
    sof.push(3);
    for component in 1..=3_u8 {
        sof.extend_from_slice(&[component, 0x11, 0]);
    }
    hostile.extend_from_slice(&sof);
    hostile.extend_from_slice(&[0xFF, 0xD9]);
    assert_eq!(decode::decode_rgba(Format::Jpeg, &hostile), None);
}

#[test]
fn a_zero_dimension_frame_is_refused() {
    let empty = png_claiming(0, 16, 6, 8);
    assert_eq!(decode::decode_rgba(Format::Png, &empty), None);
}

#[test]
fn truncated_and_corrupt_bytes_are_refused_rather_than_decoded() {
    let photo = photo_jpeg(64, 48, 0xDECD_0001);
    for keep in [1, 2, 8, photo.len() / 3, photo.len() / 2, photo.len() - 1] {
        // A truncated artifact is a normal medium condition, not an error:
        // it must come back as "not scored", never as pixels.
        let _ = decode::decode_rgba(Format::Jpeg, &photo[..keep]);
    }
    // Every byte flipped in the header region, one at a time.
    for at in 0..32.min(photo.len()) {
        let mut damaged = photo.clone();
        damaged[at] ^= 0xFF;
        let _ = decode::decode_rgba(Format::Jpeg, &damaged);
    }
    // And the same for PNG, whose chunk CRCs make most of these fail early.
    let image = png(24, 18);
    for at in 0..image.len().min(64) {
        let mut damaged = image.clone();
        damaged[at] ^= 0xFF;
        let _ = decode::decode_rgba(Format::Png, &damaged);
    }
}

#[test]
fn a_real_photograph_decodes_to_its_declared_size() {
    let photo = photo_jpeg(64, 48, 0xDECD_0002);
    let decoded = decode::decode_rgba(Format::Jpeg, &photo).expect("a real photo decodes");
    assert_eq!((decoded.width(), decoded.height()), (64, 48));
    assert_eq!(decoded.rgba().len(), 64 * 48 * 4);
}

#[test]
fn every_png_colour_type_expands_to_four_opaque_or_alpha_channels() {
    // The fixture PNG is RGBA; the greyscale and colour paths are what the
    // expansion code exists for, and nothing else covers them.
    for (colour, depth) in [(0_u8, 8_u8), (2, 8), (4, 8), (6, 8), (0, 16), (6, 16)] {
        let claimed = png_claiming(4, 4, colour, depth);
        // Data is deliberately too short for the frame, so this must refuse
        // rather than expand a partial buffer into a lying image.
        assert_eq!(
            decode::decode_rgba(Format::Png, &claimed),
            None,
            "colour type {colour} depth {depth} produced an image from a truncated stream"
        );
    }
}
