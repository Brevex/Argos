use argos::analysis::{entropy, scan_block};
use argos::formats::{jpeg::validate_jpeg_header, png::validate_png_header};
use argos::types::FragmentMap;
#[test]
fn test_entropy_zeros() {
    let data = vec![0u8; 1000];
    let ent = entropy(&data);
    assert_eq!(ent, 0.0, "Zeros should have 0 entropy");
}
#[test]
fn test_entropy_random() {
    let data: Vec<u8> = (0..=255).cycle().take(256 * 100).collect();
    let ent = entropy(&data);
    assert!(ent > 7.9, "Uniform distribution should have ~8 entropy");
}
#[test]
fn test_entropy_text() {
    let text = b"Hello World! This is a test of entropy calculation for text data.";
    let ent = entropy(text);
    assert!(ent > 3.0 && ent < 6.0, "Text should have moderate entropy");
}
#[test]
fn test_validate_jpeg_header_valid() {
    let jpeg = [
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00,
    ];
    assert!(validate_jpeg_header(&jpeg).is_some());
}
#[test]
fn test_validate_jpeg_header_invalid() {
    let not_jpeg = [0x89, 0x50, 0x4E, 0x47];
    assert!(validate_jpeg_header(&not_jpeg).is_none());
}
#[test]
fn test_validate_png_header_valid() {
    let png = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x91, 0x68, 0x36,
    ];
    let result = validate_png_header(&png);
    assert!(result.is_some());
    let info = result.unwrap();
    assert_eq!(info.width, 16);
    assert_eq!(info.height, 16);
}
#[test]
fn test_validate_png_header_invalid_crc() {
    let png = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x02, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00,
    ];
    assert!(validate_png_header(&png).is_none());
}
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
    let mut data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
    data[500] = 0xFF;
    data[501] = 0xD9;
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
    assert!(map.jpeg_footers().count() >= 1);
}
