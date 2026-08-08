//! Totality fuzz: the reassembly search terminates within its budget and never
//! panics on arbitrary media bytes (A-FUZZ-EVERY-PARSER).
//!
//! The first bytes of the input choose the header and break offsets, so the
//! fuzzer explores the search's own control flow as well as the parsers under
//! it.
#![no_main]

use std::io::Cursor;

use argos_carve::reassemble::{self, Broken, Limits};
use argos_carve::{Format, Scratch};
use argos_core::geometry::ByteOffset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((control, medium)) = data.split_at_checked(5) else {
        return;
    };
    let header = u64::from(u16::from_le_bytes([control[0], control[1]]));
    let break_at = header.saturating_add(u64::from(u16::from_le_bytes([control[2], control[3]])));
    let format = if control[4] & 1 == 0 {
        Format::Jpeg
    } else {
        Format::Png
    };
    let broken = Broken {
        header: ByteOffset::new(header),
        break_at: ByteOffset::new(break_at),
        format,
    };
    // A small budget keeps each case fast while still exercising the loops;
    // the point is termination and absence of panics, not recovery rate.
    let limits = Limits {
        max_hypotheses: 64,
        max_fragments: 4,
        ..Limits::default()
    };
    let mut scratch = Scratch::new();
    let mut src = Cursor::new(medium);
    let _ = reassemble::bifragment(
        &mut src,
        broken,
        medium.len() as u64,
        limits,
        &mut scratch,
    );
});
