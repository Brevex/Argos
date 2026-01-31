use argos::formats::jpeg::{find_jpeg_footer, is_valid_marker, validate_jpeg_header};
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
fn test_find_jpeg_footer() {
    let data = [0xAA, 0xBB, 0xFF, 0xD9, 0xCC];
    assert_eq!(find_jpeg_footer(&data), Some(2));
}
#[test]
fn test_is_valid_marker() {
    assert!(is_valid_marker(0xC0));
    assert!(is_valid_marker(0xE0));
    assert!(is_valid_marker(0xDA));
    assert!(!is_valid_marker(0x00));
}
