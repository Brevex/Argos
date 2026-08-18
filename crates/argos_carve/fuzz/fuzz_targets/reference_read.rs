//! Totality fuzz: for any input the reference reader returns a header or a
//! fault in bounded time and memory — no panic, no OOM, no hang
//! (`A-FUZZ-EVERY-PARSER`).
//!
//! The bytes here stand for a file an examiner pointed at, which is untrusted
//! for the same reason the medium is: it came off the medium.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(reference) = argos_carve::reference::Reference::read(data) {
        // A header that parsed must also graft without panicking, and the
        // graft is where a length is trusted if one ever were.
        let grafted = reference.graft(data);
        assert!(
            grafted.len() >= reference.prefix().len(),
            "a graft cannot be shorter than the header it starts with"
        );
    }
});
