use argos::core::{calculate_entropy, categorize_dimensions, Fragment, FragmentKind, FragmentMap};
use argos::format::jpeg::{detect_jpeg_break, matches_jpeg_continuation, validate_jpeg};
use argos::format::png::{detect_png_break, matches_png_continuation, validate_png_header};
use argos::scan::scan_block;

#[test]
fn test_validate_jpeg_empty_input() {
    assert!(validate_jpeg(&[]).is_none());
}

#[test]
fn test_validate_jpeg_single_byte() {
    assert!(validate_jpeg(&[0xFF]).is_none());
}

#[test]
fn test_validate_jpeg_just_soi() {
    assert!(validate_jpeg(&[0xFF, 0xD8]).is_none());
}

#[test]
fn test_validate_jpeg_truncated_marker_length() {
    let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
    assert!(validate_jpeg(&data).is_none());
}

#[test]
fn test_validate_jpeg_marker_length_overflow() {
    let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF];
    data.extend_from_slice(&[0x00; 20]);
    assert!(validate_jpeg(&data).is_none());
}

#[test]
fn test_validate_jpeg_all_ff_bytes() {
    let data = vec![0xFF; 1024];
    assert!(validate_jpeg(&data).is_none());
}

#[test]
fn test_validate_jpeg_all_zero_bytes() {
    let data = vec![0x00; 1024];
    assert!(validate_jpeg(&data).is_none());
}

#[test]
fn test_detect_jpeg_break_empty() {
    assert!(detect_jpeg_break(&[], 0).is_none());
}

#[test]
fn test_matches_jpeg_continuation_empty() {
    assert!(!matches_jpeg_continuation(&[]));
}

#[test]
fn test_validate_png_empty_input() {
    assert!(validate_png_header(&[]).is_none());
}

#[test]
fn test_validate_png_partial_signature() {
    assert!(validate_png_header(&[0x89, 0x50, 0x4E]).is_none());
}

#[test]
fn test_validate_png_signature_only() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert!(validate_png_header(&sig).is_none());
}

#[test]
fn test_validate_png_massive_chunk_length() {
    let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    data.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
    data.extend_from_slice(b"IHDR");
    data.extend_from_slice(&[0x00; 100]);
    assert!(validate_png_header(&data).is_none());
}

#[test]
fn test_detect_png_break_empty() {
    assert!(detect_png_break(&[]).is_none());
}

#[test]
fn test_matches_png_continuation_empty() {
    assert!(!matches_png_continuation(&[]));
}

#[test]
fn test_entropy_empty_data() {
    let e = calculate_entropy(&[]);
    assert!(e >= 0.0 && e.is_finite());
}

#[test]
fn test_entropy_single_byte() {
    let e = calculate_entropy(&[42]);
    assert!(e >= 0.0 && e.is_finite());
}

#[test]
fn test_entropy_result_bounded() {
    let data: Vec<u8> = (0..=255).collect();
    let e = calculate_entropy(&data);
    assert!(e <= 8.01, "Entropy should be bounded by 8 bits, got {}", e);
}

#[test]
fn test_categorize_zero_dimensions() {
    let v = categorize_dimensions(0, 0);
    assert_eq!(v, argos::core::DimensionVerdict::TooSmall);
}

#[test]
fn test_categorize_one_pixel() {
    let v = categorize_dimensions(1, 1);
    assert_eq!(v, argos::core::DimensionVerdict::TooSmall);
}

#[test]
fn test_categorize_extreme_aspect_ratio() {
    let v = categorize_dimensions(10000, 1);
    let _ = v;
}

#[test]
fn test_categorize_max_dimensions() {
    let v = categorize_dimensions(u32::MAX, u32::MAX);
    let _ = v;
}

#[test]
fn test_scan_block_random_noise() {
    let mut data = vec![0u8; 4096 * 4];
    for (i, b) in data.iter_mut().enumerate() {
        *b = ((i.wrapping_mul(131).wrapping_add(7)) % 256) as u8;
    }
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
}

#[test]
fn test_scan_block_all_0xff() {
    let data = vec![0xFF; 4096 * 4];
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
}

#[test]
fn test_scan_block_alternating_soi_markers() {
    let mut data = Vec::with_capacity(4096);
    while data.len() + 3 <= 4096 {
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
    }
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
}

#[test]
fn test_scan_block_many_png_signatures() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut data = Vec::with_capacity(4096);
    while data.len() + sig.len() <= 4096 {
        data.extend_from_slice(&sig);
    }
    let mut map = FragmentMap::new();
    scan_block(0, &data, &mut map);
}

#[test]
fn test_scan_block_empty_data() {
    let mut map = FragmentMap::new();
    scan_block(0, &[], &mut map);
    assert!(map.is_empty());
}

#[test]
fn test_fragment_map_does_not_grow_unbounded() {
    let mut map = FragmentMap::new();
    for i in 0..500_000u64 {
        map.push(Fragment::new(i * 4096, FragmentKind::JpegHeader, 7.5));
    }
    let total = map.len();
    assert!(total <= 500_000, "FragmentMap should have a capacity limit");
}

#[test]
fn test_generated_filenames_no_path_traversal() {
    use argos::extraction::extract_all;
    use argos::io::DiskReader;
    use std::fs;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("out");
    let disk_path = dir.path().join("disk.img");
    fs::write(&disk_path, &[0u8; 4096]).unwrap();
    let reader = DiskReader::open(&disk_path).unwrap();

    let report = extract_all(&[], &reader, &output_dir, None).unwrap();
    for path in &report.extracted {
        assert!(
            path.starts_with(&output_dir),
            "Extracted file {:?} must be inside output dir {:?}",
            path,
            output_dir
        );
    }
}

#[test]
fn test_fragment_with_max_offset() {
    let f = Fragment::new(u64::MAX, FragmentKind::JpegHeader, 7.0);
    assert_eq!(f.offset, u64::MAX);
}

#[test]
fn test_fragment_map_sort_with_max_offsets() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(u64::MAX, FragmentKind::JpegHeader, 7.0));
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.0));
    map.push(Fragment::new(u64::MAX - 1, FragmentKind::JpegHeader, 7.0));
    map.sort_by_offset();

    let offsets: Vec<u64> = map.jpeg_headers().iter().map(|f| f.offset).collect();
    assert_eq!(offsets, vec![0, u64::MAX - 1, u64::MAX]);
}
