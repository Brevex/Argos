use argos::analysis::scan_block;
use argos::types::FragmentMap;

#[test]
fn test_scan_block_finds_jpeg_header() {
    let mut data = vec![0u8; 1000];
    data[100] = 0xFF;
    data[101] = 0xD8;
    data[102] = 0xFF;
    data[103] = 0xE0;
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert!(map.jpeg_headers().count() >= 1);
}

#[test]
fn test_scan_block_finds_jpeg_footer() {
    let mut data = vec![0u8; 1000];
    data[500] = 0xFF;
    data[501] = 0xD9;
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert!(map.jpeg_footers().count() >= 1);
}

#[test]
fn test_scan_block_finds_png_header() {
    let mut data = vec![0u8; 1000];
    let png_magic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    data[100..108].copy_from_slice(&png_magic);
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert!(map.png_headers().count() >= 1);
}

#[test]
fn test_scan_block_finds_png_footer() {
    let mut data = vec![0u8; 1000];
    data[500..504].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    data[504..508].copy_from_slice(b"IEND");
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert!(map.png_footers().count() >= 1);
}

#[test]
fn test_scan_block_multiple_headers() {
    let mut data = vec![0u8; 1000];
    data[100] = 0xFF;
    data[101] = 0xD8;
    data[102] = 0xFF;
    data[500] = 0xFF;
    data[501] = 0xD8;
    data[502] = 0xFF;
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert_eq!(map.jpeg_headers().count(), 2);
}
