//! Totality fuzz: MBR/GPT parsing returns in bounded time and memory on any
//! input — no panic, no OOM, no hang (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let len = data.len() as u64;
    let _ = argos_fs::part::scan(&mut Cursor::new(data), len);
});
