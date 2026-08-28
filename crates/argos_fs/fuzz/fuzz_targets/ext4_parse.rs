//! Totality fuzz: the ext4 superblock, journal walker and directory-entry
//! carver never panic, OOM or hang on arbitrary bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_fs::ext4::dir_entries(data);
    let _ = argos_fs::ext4::Ext4::from_superblock(data, ByteOffset::new(0));
    if let Ok(Some(fs)) = argos_fs::ext4::Ext4::open(&mut Cursor::new(data), ByteOffset::new(0)) {
        let _ = fs.recover_from_journal(&mut Cursor::new(data));
    }
});
