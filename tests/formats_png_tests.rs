use argos::format::png::{detect_png_break, matches_png_continuation, validate_png_header};

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

#[test]
fn test_validate_png_minimum_size() {
    let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
    assert!(validate_png_header(&data).is_none());
}

#[test]
fn test_validate_png_wrong_signature() {
    let mut data = vec![0x00; 33];
    data[0..8].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0D, 0x0A, 0x1A, 0x0A]);
    assert!(validate_png_header(&data).is_none());
}

#[test]
fn test_validate_png_ihdr_bad_crc() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr_payload = Vec::new();

    ihdr_payload.extend_from_slice(&100u32.to_be_bytes());
    ihdr_payload.extend_from_slice(&100u32.to_be_bytes());
    ihdr_payload.push(8);
    ihdr_payload.push(2);
    ihdr_payload.extend_from_slice(&[0, 0, 0]);

    let mut ihdr_chunk = Vec::new();

    ihdr_chunk.extend_from_slice(&(ihdr_payload.len() as u32).to_be_bytes());
    ihdr_chunk.extend_from_slice(b"IHDR");
    ihdr_chunk.extend_from_slice(&ihdr_payload);
    ihdr_chunk.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

    let mut data = Vec::new();

    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr_chunk);

    assert!(
        validate_png_header(&data).is_none(),
        "Bad IHDR CRC should be rejected"
    );
}

#[test]
fn test_validate_png_ihdr_wrong_length() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr_payload = vec![0u8; 10];
    let ihdr_chunk = make_png_chunk(b"IHDR", &ihdr_payload);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr_chunk);

    assert!(
        validate_png_header(&data).is_none(),
        "IHDR length ≠ 13 should be rejected"
    );
}

#[test]
fn test_validate_png_ancillary_chunks() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let text_chunk = make_png_chunk(b"tEXt", b"Comment\x00Test");
    let iccp_chunk = make_png_chunk(b"iCCP", &[0u8; 20]);
    let idat = make_png_chunk(b"IDAT", &[0xAA; 50]);
    let iend = make_png_chunk(b"IEND", &[]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&text_chunk);
    data.extend_from_slice(&iccp_chunk);
    data.extend_from_slice(&idat);
    data.extend_from_slice(&iend);

    let info = validate_png_header(&data);
    assert!(info.is_some());
    let info = info.unwrap();
    assert!(
        info.metadata.has_text_chunks,
        "tEXt chunk should be detected"
    );
    assert!(
        info.metadata.has_icc_profile,
        "iCCP chunk should be detected"
    );
}

#[test]
fn test_validate_png_idat_counting() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let idat1 = make_png_chunk(b"IDAT", &[0xAA; 100]);
    let idat2 = make_png_chunk(b"IDAT", &[0xBB; 200]);
    let idat3 = make_png_chunk(b"IDAT", &[0xCC; 50]);
    let iend = make_png_chunk(b"IEND", &[]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&idat1);
    data.extend_from_slice(&idat2);
    data.extend_from_slice(&idat3);
    data.extend_from_slice(&iend);

    let info = validate_png_header(&data);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.idat_count, 3);
    assert_eq!(info.idat_total_bytes, 350);
}

#[test]
fn test_chunk_iterator_truncated_chunk() {
    use argos::format::png::PngChunkIterator;

    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut data = Vec::new();

    data.extend_from_slice(&sig);
    data.extend_from_slice(&100u32.to_be_bytes());
    data.extend_from_slice(b"IDAT");
    data.extend_from_slice(&[0xAA; 10]);

    let iter = PngChunkIterator::new(&data);
    assert!(iter.is_some());
    let chunks: Vec<_> = iter.unwrap().collect();
    assert_eq!(chunks.len(), 0, "Truncated chunk should not be yielded");
}

#[test]
fn test_detect_png_break_crc_mismatch() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let idat_valid = make_png_chunk(b"IDAT", &[0xAA; 100]);
    let mut idat_bad = make_png_chunk(b"IDAT", &[0xBB; 100]);
    let len = idat_bad.len();
    idat_bad[len - 1] ^= 0xFF;
    let iend = make_png_chunk(b"IEND", &[]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&idat_valid);
    data.extend_from_slice(&idat_bad);
    data.extend_from_slice(&iend);

    let bp = detect_png_break(&data);
    assert!(bp.is_some(), "CRC mismatch after IDAT should trigger break");
}

#[test]
fn test_detect_png_break_zero_run() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let ihdr = make_valid_ihdr();
    let idat = make_png_chunk(b"IDAT", &[0xAA; 100]);

    let mut data = Vec::new();
    data.extend_from_slice(&sig);
    data.extend_from_slice(&ihdr);
    data.extend_from_slice(&idat);
    data.extend_from_slice(&[0x00; 600]);

    let bp = detect_png_break(&data);
    assert!(bp.is_some(), "Zero run after IDAT should trigger break");
}

#[test]
fn test_matches_png_continuation_too_short() {
    let data = vec![0xAA; 11]; // < 12 bytes
    assert!(!matches_png_continuation(&data));
}

#[test]
fn test_matches_png_continuation_not_idat_type() {
    let chunk = make_png_chunk(b"tIME", &[0x00; 7]);
    assert!(!matches_png_continuation(&chunk));
}

#[test]
fn test_matches_png_continuation_valid_complete_chunk() {
    let idat_data: Vec<u8> = (0..200).map(|i| ((i * 37 + 13) % 251) as u8).collect();
    let chunk = make_png_chunk(b"IDAT", &idat_data);
    assert!(matches_png_continuation(&chunk));
}
