//! Totality fuzz: every NTFS entry point — boot sector, the `$MFT` walk, the
//! orphan scan, index buffers and USN records — returns in bounded time and
//! memory on arbitrary bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use std::io::Cursor;

use argos_core::geometry::{ByteOffset, ByteRange};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos_fs::ntfs::Ntfs::from_boot_sector(data, ByteOffset::new(0));
    let _ = argos_fs::ntfs::is_plausible_record(data);
    let _ = argos_fs::ntfs::indx_names(data);
    let _ = argos_fs::ntfs::usn_deletions(data);
    let _ = argos_fs::ntfs::orphan_scan(
        &mut Cursor::new(data),
        ByteRange::new(ByteOffset::new(0), data.len() as u64),
        ByteOffset::new(0),
        4096,
    );
    if let Ok(Some(volume)) =
        argos_fs::ntfs::Ntfs::open(&mut Cursor::new(data), ByteOffset::new(0))
    {
        let _ = volume.recover_deleted(&mut Cursor::new(data));
    }
});
