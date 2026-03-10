#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let e = argos::core::calculate_entropy(data);
    assert!(e >= 0.0);
    assert!(e.is_finite());
    assert!(e <= 8.01);
});
