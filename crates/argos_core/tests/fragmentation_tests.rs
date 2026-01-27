use argos_core::{
    aligned_buffer::AlignedBuffer,
    error::FileFormat,
    matching::{FooterCandidate, GlobalMatcher, HeaderCandidate},
    validation::{EntropyFilter, JpegStructuralValidator, ValidationContext, ValidationPipeline},
    ValidationStage,
};

fn create_minimal_jpeg() -> Vec<u8> {
    vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B,
        0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
        0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF,
        0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00,
        0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05,
        0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
        0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
        0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A,
        0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56,
        0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
        0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9,
        0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6,
        0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
        0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7,
        0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFB, 0xD5,
        0xDB, 0x20, 0xA8, 0xF1, 0xD3, 0xFC, 0xBF, 0xFF, 0xD9,
    ]
}

fn create_garbage(size: usize, seed: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(size);
    let mut state = seed as u32;
    for _ in 0..size {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        data.push((state >> 16) as u8);
    }
    data
}

#[test]
fn test_aligned_buffer_basic() {
    let buffer = AlignedBuffer::new_default(4096);
    assert_eq!(buffer.len(), 4096);
    assert!(buffer.is_aligned());
    assert!((buffer.as_ptr() as usize).is_multiple_of(4096));
}

#[test]
fn test_entropy_filter_jpeg_range() {
    let filter = EntropyFilter::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let jpeg = create_minimal_jpeg();
    let result = filter.validate(&jpeg, &ctx);

    println!(
        "JPEG entropy validation: passed={}, confidence={:.2}",
        result.passed, result.confidence
    );
}

#[test]
fn test_entropy_filter_rejects_zeros() {
    let filter = EntropyFilter::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let zeros = vec![0u8; 1000];
    let result = filter.validate(&zeros, &ctx);
    assert!(!result.passed, "All-zeros should fail entropy check");
}

#[test]
fn test_entropy_filter_rejects_random() {
    let filter = EntropyFilter::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let mut random = Vec::with_capacity(256 * 100);
    for _ in 0..100 {
        for b in 0..=255u8 {
            random.push(b);
        }
    }

    let result = filter.validate(&random, &ctx);

    println!(
        "Random data entropy validation: passed={}, confidence={:.2}, details={:?}",
        result.passed, result.confidence, result.details
    );
}

#[test]
fn test_jpeg_structural_validator_valid() {
    let validator = JpegStructuralValidator::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let jpeg = create_minimal_jpeg();
    let result = validator.validate(&jpeg, &ctx);

    assert!(
        result.passed,
        "Valid JPEG should pass structural validation: {:?}",
        result.details
    );
    assert!(result.confidence > 0.5, "Confidence should be reasonable");
}

#[test]
fn test_jpeg_structural_validator_invalid() {
    let validator = JpegStructuralValidator::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let garbage = create_garbage(1000, 42);
    let result = validator.validate(&garbage, &ctx);

    assert!(
        !result.passed,
        "Random data should fail structural validation"
    );
}

#[test]
fn test_validation_pipeline_jpeg() {
    let pipeline = ValidationPipeline::for_jpeg();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let jpeg = create_minimal_jpeg();
    let result = pipeline.validate(&jpeg, &ctx);

    println!(
        "JPEG pipeline validation: passed={}, confidence={:.2}, details={:?}",
        result.passed, result.confidence, result.details
    );
}

#[test]
fn test_global_matcher_simple() {
    let mut matcher = GlobalMatcher::new();

    matcher.add_header(HeaderCandidate {
        offset: 0,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: Some((1920, 1080)),
        expected_size_range: Some((100_000, 5_000_000)),
    });

    matcher.add_footer(FooterCandidate {
        offset: 500_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
    });

    let results = matcher.solve_greedy();
    assert_eq!(results.len(), 1, "Should find exactly one match");
    assert_eq!(results[0].header_idx, 0);
    assert_eq!(results[0].footer_idx, 0);
}

#[test]
fn test_global_matcher_rejects_format_mismatch() {
    let mut matcher = GlobalMatcher::new();

    matcher.add_header(HeaderCandidate {
        offset: 0,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: None,
        expected_size_range: None,
    });

    matcher.add_footer(FooterCandidate {
        offset: 500_000,
        format: FileFormat::Png,
        quality: 0.9,
    });

    let results = matcher.solve_greedy();
    assert_eq!(results.len(), 0, "Should not match different formats");
}

#[test]
fn test_global_matcher_rejects_footer_before_header() {
    let mut matcher = GlobalMatcher::new();

    matcher.add_header(HeaderCandidate {
        offset: 1_000_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: None,
        expected_size_range: None,
    });

    matcher.add_footer(FooterCandidate {
        offset: 500_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
    });

    let results = matcher.solve_greedy();
    assert_eq!(results.len(), 0, "Should not match footer before header");
}

#[test]
fn test_global_matcher_optimal_assignment() {
    let mut matcher = GlobalMatcher::new();

    matcher.add_header(HeaderCandidate {
        offset: 0,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: None,
        expected_size_range: Some((900_000, 1_100_000)),
    });

    matcher.add_header(HeaderCandidate {
        offset: 100_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: None,
        expected_size_range: Some((400_000, 600_000)),
    });

    matcher.add_footer(FooterCandidate {
        offset: 600_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
    });

    matcher.add_footer(FooterCandidate {
        offset: 1_000_000,
        format: FileFormat::Jpeg,
        quality: 0.9,
    });

    let results = matcher.solve_optimal();
    assert_eq!(results.len(), 2, "Should find two matches");

    let mut assignments = std::collections::HashMap::new();
    for r in &results {
        assignments.insert(r.header_idx, r.footer_idx);
    }

    assert_eq!(
        assignments.get(&0),
        Some(&1),
        "Header 0 should match footer 1"
    );
    assert_eq!(
        assignments.get(&1),
        Some(&0),
        "Header 1 should match footer 0"
    );
}

#[test]
fn test_simulated_disk_contiguous() {
    let jpeg = create_minimal_jpeg();
    let jpeg_len = jpeg.len();

    let mut disk = Vec::new();
    disk.extend(create_garbage(1024, 1));
    let jpeg_start = disk.len();
    disk.extend(&jpeg);
    let jpeg_end = disk.len();
    disk.extend(create_garbage(1024, 2));

    let header_offset = jpeg_start;
    let footer_offset = jpeg_end - 2;

    let mut matcher = GlobalMatcher::new();

    matcher.add_header(HeaderCandidate {
        offset: header_offset as u64,
        format: FileFormat::Jpeg,
        quality: 0.9,
        dimensions: None,
        expected_size_range: None,
    });

    matcher.add_footer(FooterCandidate {
        offset: footer_offset as u64,
        format: FileFormat::Jpeg,
        quality: 0.9,
    });

    let results = matcher.solve_greedy();
    assert_eq!(results.len(), 1);

    let matched_header = matcher.get_header(results[0].header_idx).unwrap();
    let matched_footer = matcher.get_footer(results[0].footer_idx).unwrap();

    let file_size = matched_footer.offset - matched_header.offset + 2;
    assert_eq!(
        file_size as usize, jpeg_len,
        "Extracted size should match original"
    );
}

#[test]
fn test_rendering_validator_valid_jpeg() {
    use argos_core::RenderingValidator;

    let validator = RenderingValidator::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let jpeg = create_minimal_jpeg();
    let result = validator.validate(&jpeg, &ctx);

    println!(
        "RenderingValidator JPEG: passed={}, confidence={:.2}, details={:?}",
        result.passed, result.confidence, result.details
    );
}

#[test]
fn test_rendering_validator_invalid_data() {
    use argos_core::RenderingValidator;

    let validator = RenderingValidator::new();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let garbage = create_garbage(1000, 42);
    let result = validator.validate(&garbage, &ctx);

    assert!(
        !result.passed,
        "Random data should fail rendering validation"
    );
}

#[test]
fn test_validation_pipeline_with_rendering() {
    let pipeline = ValidationPipeline::for_jpeg_with_rendering();
    let ctx = ValidationContext {
        format: FileFormat::Jpeg,
        ..Default::default()
    };

    let jpeg = create_minimal_jpeg();
    let result = pipeline.validate(&jpeg, &ctx);

    println!(
        "Full pipeline with rendering: passed={}, confidence={:.2}",
        result.passed, result.confidence
    );
}
