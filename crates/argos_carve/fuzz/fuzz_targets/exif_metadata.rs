//! Totality fuzz: reading a recovered image's own description never panics or
//! hangs on arbitrary bytes (A-FUZZ-EVERY-PARSER).
//!
//! Both entry points: the whole-image walk that has to find `APP1` among
//! marker segments whose lengths come from the medium, and the TIFF/IFD walk
//! whose every offset does.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_carve::metadata(data);
    let _ = argos_carve::jpeg::exif::metadata(data);
});
