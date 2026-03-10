#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos::format::jpeg::validate_jpeg(data);
    let _ = argos::format::jpeg::detect_jpeg_break(data, 0);
    let _ = argos::format::jpeg::matches_jpeg_continuation(data);
});
