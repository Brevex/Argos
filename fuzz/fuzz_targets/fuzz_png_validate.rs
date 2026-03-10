#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos::format::png::validate_png_header(data);
    let _ = argos::format::png::detect_png_break(data);
    let _ = argos::format::png::matches_png_continuation(data);
});
