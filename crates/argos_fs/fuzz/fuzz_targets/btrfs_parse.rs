//! Totality fuzz: every btrfs entry point — the superblock validator, the
//! chunk map, the tree walk and the stale-block sweep — returns in bounded time
//! and memory on arbitrary bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::{ByteOffset, ByteRange};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_fs::btrfs::crc32c(data);
    // The primary mirror's own offset: a superblock only validates against an
    // anchor it could actually have been found at, so the pure validator is
    // fuzzed at a position that lets it get past its first check.
    let _ = argos_fs::btrfs::from_superblock(data, ByteOffset::new(0x1_0000));

    // The medium-reading path allocates from superblock geometry and walks
    // trees; fuzz it separately from the in-memory one.
    if let Ok(Some(fs)) = argos_fs::btrfs::Btrfs::open(&mut Cursor::new(data), ByteOffset::new(0))
        && let Ok(live) = fs.live(&mut Cursor::new(data))
    {
        let _ = fs.recover_deleted(&mut Cursor::new(data), &live);
        let _ = fs.orphan_scan(
            &mut Cursor::new(data),
            &live,
            ByteRange::new(ByteOffset::new(0), data.len() as u64),
        );
    }
});
