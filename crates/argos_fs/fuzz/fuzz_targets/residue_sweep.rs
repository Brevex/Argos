//! Totality fuzz: the whole-surface residue sweep never panics, OOMs or hangs
//! on arbitrary media bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let len = data.len() as u64;
    let _ = argos_fs::residue::sweep(&mut Cursor::new(data), len, &[]);
});
