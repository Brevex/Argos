//! Totality fuzz: for any input the JPEG validator returns a verdict in
//! bounded time and memory — no panic, no OOM, no hang (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::geometry::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut scratch = argos_carve::Scratch::new();
    let _ = argos_carve::jpeg::validate(
        &mut Cursor::new(data),
        ByteOffset::new(0),
        data.len() as u64,
        &mut scratch,
    );
});
