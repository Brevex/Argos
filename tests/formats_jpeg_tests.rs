use argos::formats::jpeg::{
    detect_jpeg_break, find_sos_offset, is_valid_marker, matches_jpeg_continuation, validate_jpeg,
};

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

#[test]
fn test_find_sos_offset() {
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]);
    jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    jpeg.extend_from_slice(b"JFIF\x00\x01\x01\x00\x00\x48\x00\x48\x00\x00");
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    jpeg.extend_from_slice(&[0xAA; 100]);

    let sos = find_sos_offset(&jpeg);
    assert!(sos.is_some());
    let offset = sos.unwrap();
    assert!(offset > 20);
}

#[test]
fn test_find_sos_offset_no_sos() {
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x02];
    assert!(find_sos_offset(&jpeg).is_none());
}

#[test]
fn test_detect_jpeg_break_zero_run() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xFF, 0xD8]);
    data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    let scan_start = data.len();
    for i in 0..1000 {
        data.push(((i * 131 + 17) % 251) as u8);
    }
    let zero_start = data.len();
    data.extend_from_slice(&[0x00; 600]);

    let bp = detect_jpeg_break(&data, scan_start);
    assert!(bp.is_some());
    let bp_offset = bp.unwrap();
    assert_eq!(bp_offset, zero_start);
}

#[test]
fn test_detect_jpeg_break_valid_eoi() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xFF, 0xD8]);
    data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    let scan_start = data.len();
    for i in 0..500 {
        data.push(((i * 131 + 17) % 251) as u8);
    }
    data.extend_from_slice(&[0xFF, 0xD9]);

    let bp = detect_jpeg_break(&data, scan_start);
    assert!(bp.is_none());
}

#[test]
fn test_matches_jpeg_continuation_high_entropy() {
    let data: Vec<u8> = (0..64).map(|i| ((i * 131 + 17) % 251) as u8).collect();
    assert!(matches_jpeg_continuation(&data));
}

#[test]
fn test_matches_jpeg_continuation_low_entropy() {
    let data = vec![0x00u8; 64];
    assert!(!matches_jpeg_continuation(&data));
}
