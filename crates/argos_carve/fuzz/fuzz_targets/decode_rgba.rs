//! Totality of pixel decoding: arbitrary bytes must never crash it.
//!
//! This runs on every artifact a scan recovers, which means it runs on bytes
//! the medium chose. It is allowed to refuse anything; it is not allowed to
//! panic, and it is not allowed to size an allocation from a number a frame
//! header claimed (A-BOUNDED-ALLOC) — the libFuzzer `-rss_limit_mb` default
//! turns that second one into a reported failure rather than a slow death.

#![no_main]

use argos_carve::decode;
use argos_core::Format;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte picks the format, so one corpus exercises both decoders
    // rather than needing two.
    let Some((selector, bytes)) = data.split_first() else {
        return;
    };
    let format = if selector % 2 == 0 {
        Format::Jpeg
    } else {
        Format::Png
    };

    if let Some(image) = decode::decode_rgba(format, bytes) {
        // Anything it does return has to be internally consistent: the buffer
        // is exactly the frame it claims. A caller indexes by those
        // dimensions, so a mismatch here is an out-of-bounds read there.
        let pixels = u64::from(image.width()) * u64::from(image.height());
        assert!(pixels > 0, "an image was returned with no pixels");
        assert!(
            pixels <= decode::MAX_RGBA_PIXELS,
            "an image past the pixel ceiling was returned"
        );
        assert_eq!(
            image.rgba().len() as u64,
            pixels * 4,
            "the pixel buffer does not match the dimensions reported with it"
        );
    }
});
