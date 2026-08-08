//! Totality fuzz: the entropy decoder never panics, OOMs or hangs on arbitrary
//! bytes, and never claims a completeness it has not established
//! (A-FUZZ-EVERY-PARSER).
//!
//! This is the oracle every reassembly answer rests on, so a false "complete"
//! here would become fabricated evidence downstream.
#![no_main]

use std::io::Cursor;

use argos_carve::Scratch;
use argos_carve::mcu::{self, ScanStop};
use argos_core::geometry::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut src = Cursor::new(data);
    let mut scratch = Scratch::new();
    let Ok(outcome) = mcu::scan(
        &mut src,
        ByteOffset::new(0),
        data.len() as u64,
        &mut scratch,
    ) else {
        return;
    };

    // The decoder may never report having decoded more of a frame than the
    // frame asks for; reassembly treats equality as proof the bytes are the
    // image.
    assert!(
        outcome.mcus_decoded <= outcome.mcus_required || outcome.mcus_required == 0,
        "decoded {} of {} MCUs",
        outcome.mcus_decoded,
        outcome.mcus_required
    );
    if outcome.stop != ScanStop::Complete {
        assert!(!outcome.is_complete());
    }
    // The reported end is where a fragment gets trimmed to, so it must stay
    // inside the bytes offered.
    assert!(
        outcome.end.get() <= data.len() as u64,
        "end {} past a {}-byte input",
        outcome.end,
        data.len()
    );
    let _ = outcome.pixel_row_of(u32::MAX);
});
