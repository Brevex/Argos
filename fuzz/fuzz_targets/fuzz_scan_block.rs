#![no_main]
use libfuzzer_sys::fuzz_target;
use argos::core::FragmentMap;

fuzz_target!(|data: &[u8]| {
    let mut map = FragmentMap::new();
    argos::scan::scan_block(0, data, &mut map);
});
