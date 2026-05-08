use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = argos::validate::jpeg::validate(data);
});
