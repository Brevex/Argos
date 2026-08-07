//! Totality fuzz: the full signature sweep plus validation never panics,
//! OOMs or hangs on arbitrary media bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_carve::Carver::new().scan(&mut Cursor::new(data));
});
