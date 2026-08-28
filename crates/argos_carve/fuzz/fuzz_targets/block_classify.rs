//! Totality fuzz: block classification never panics, OOMs or hangs on
//! arbitrary medium bytes (A-FUZZ-EVERY-PARSER).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let profile = argos_carve::classify::classify(data);
    // Scores feed ranking decisions, so they must stay in range whatever the
    // medium held; a NaN would poison every comparison downstream.
    assert!(
        profile.score.is_finite() && (0.0..=1.0).contains(&profile.score),
        "score {} out of range for a {}-byte block",
        profile.score,
        data.len()
    );
    assert!(
        profile.entropy.is_finite() && (0.0..=8.0).contains(&profile.entropy),
        "entropy {} out of range for a {}-byte block",
        profile.entropy,
        data.len()
    );
    let _ = argos_carve::reassemble::restart_points(data, argos_core::ByteOffset::new(0));
});
