use argos::formats::jpeg::{is_valid_marker, validate_jpeg};

#[test]
fn test_validate_jpeg_with_sof() {
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]);
    jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    jpeg.extend_from_slice(b"JFIF\x00\x01\x01\x00\x00\x48\x00\x48\x00\x00");
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    jpeg.extend_from_slice(&[0x01, 0x00]);
    jpeg.extend_from_slice(&[0x01, 0x40]);
    jpeg.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);

    let info = validate_jpeg(&jpeg);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 256);
    assert!(info.metadata.has_jfif);
}

#[test]
fn test_validate_jpeg_with_exif() {
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]);
    jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x10]);
    jpeg.extend_from_slice(b"Exif\x00\x00");
    jpeg.extend_from_slice(&[0x00; 8]);
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    jpeg.extend_from_slice(&[0x04, 0x00]);
    jpeg.extend_from_slice(&[0x06, 0x00]);
    jpeg.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);

    let info = validate_jpeg(&jpeg);
    assert!(info.is_some());
    let info = info.unwrap();
    assert!(info.metadata.has_exif);
    assert_eq!(info.width, 1536);
    assert_eq!(info.height, 1024);
}

#[test]
fn test_validate_jpeg_invalid() {
    let not_jpeg = [0x89, 0x50, 0x4E, 0x47];
    assert!(validate_jpeg(&not_jpeg).is_none());
}

#[test]
fn test_is_valid_marker() {
    assert!(is_valid_marker(0xC0));
    assert!(is_valid_marker(0xE0));
    assert!(is_valid_marker(0xDA));
    assert!(!is_valid_marker(0x00));
}
