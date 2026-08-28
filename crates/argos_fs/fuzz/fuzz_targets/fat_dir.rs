//! Totality fuzz: FAT32/exFAT boot-sector parsing, volume opening and the
//! root-directory walk never panic, OOM or hang on arbitrary bytes
//! (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(volume) = argos_fs::fat::Fat::from_boot_sector(data, ByteOffset::new(0)) {
        let _ = volume.deleted_in_directory(data);
    }
    // The medium-reading path allocates from boot-sector geometry; fuzz it
    // separately from the in-memory one.
    if let Ok(Some(volume)) =
        argos_fs::fat::Fat::open(&mut Cursor::new(data), ByteOffset::new(0))
    {
        let _ = volume.recover_deleted(&mut Cursor::new(data));
    }
});
