use argos::formats::png::{detect_png_break, matches_png_continuation, validate_png_header};

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
fn test_validate_png_header_invalid_signature() {
    let not_png = [0xFF, 0xD8, 0xFF, 0xE0];
    assert!(validate_png_header(&not_png).is_none());
}

fn make_png_chunk(chunk_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(payload);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(chunk_type);
    hasher.update(payload);
    let crc = hasher.finalize();
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

fn make_valid_ihdr() -> Vec<u8> {
    let mut ihdr_payload = Vec::new();
    ihdr_payload.extend_from_slice(&100u32.to_be_bytes());
    ihdr_payload.extend_from_slice(&100u32.to_be_bytes());
    ihdr_payload.push(8);
    ihdr_payload.push(2);
    ihdr_payload.extend_from_slice(&[0, 0, 0]);
    make_png_chunk(b"IHDR", &ihdr_payload)
}

#[test]
fn test_detect_png_break_no_break() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let idat = make_png_chunk(b"IDAT", &[0xAA; 100]);
    let iend = make_png_chunk(b"IEND", &[]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&idat);
    data.extend_from_slice(&iend);

    let bp = detect_png_break(&data);
    assert!(bp.is_none());
}

#[test]
fn test_detect_png_break_truncated() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let idat = make_png_chunk(b"IDAT", &[0xBB; 100]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&idat);
    data.extend_from_slice(&[0xFF; 20]);

    let bp = detect_png_break(&data);
    assert!(bp.is_some());
}

#[test]
fn test_matches_png_continuation_valid_idat() {
    let chunk = make_png_chunk(
        b"IDAT",
        &(0..100)
            .map(|i| ((i * 37 + 13) % 251) as u8)
            .collect::<Vec<u8>>(),
    );
    assert!(matches_png_continuation(&chunk));
}

#[test]
fn test_matches_png_continuation_not_idat() {
    let chunk = make_png_chunk(b"tEXt", b"hello");
    assert!(!matches_png_continuation(&chunk));
}
