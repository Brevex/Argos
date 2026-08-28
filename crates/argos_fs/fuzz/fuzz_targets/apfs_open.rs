//! Totality fuzz: APFS container opening and checkpoint diffing never panic,
//! OOM or hang on arbitrary bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(Some(container)) =
        argos_fs::apfs::Apfs::open(&mut Cursor::new(data), ByteOffset::new(0))
    {
        let _ = container.recover_deleted(&mut Cursor::new(data));
    }
});
