use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut scanner = match argos::carve::ssd::Scanner::new() {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = scanner.scan_block(data);
});
