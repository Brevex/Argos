//! Totality fuzz: the EXIF/TIFF thumbnail walker never panics or hangs on
//! arbitrary bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_carve::exif::thumbnail(data);
});
