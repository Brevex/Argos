use argos::core::{
    calculate_entropy, categorize_dimensions, score_jpeg, score_png, DimensionVerdict,
    JpegMetadata, PngMetadata, QuantizationQuality,
};

#[test]
fn test_calculate_entropy_empty_data() {
    assert_eq!(calculate_entropy(&[]), 0.0);
}

#[test]
fn test_calculate_entropy_single_byte() {
    assert_eq!(calculate_entropy(&[0x42]), 0.0);
}

#[test]
fn test_calculate_entropy_all_same_byte() {
    let data = vec![0xAA; 1024];
    assert_eq!(calculate_entropy(&data), 0.0);
}

#[test]
fn test_calculate_entropy_two_values_equal() {
    let mut data = Vec::new();

    for _ in 0..512 {
        data.push(0x00);
        data.push(0xFF);
    }

    let e = calculate_entropy(&data);
    assert!((e - 1.0).abs() < 0.01, "Expected ~1.0, got {}", e);
}

#[test]
fn test_calculate_entropy_uniform_distribution() {
    let mut data = Vec::new();
    for _ in 0..16 {
        for b in 0..=255u8 {
            data.push(b);
        }
    }
    let e = calculate_entropy(&data);
    assert!((e - 8.0).abs() < 0.01, "Expected ~8.0, got {}", e);
}

#[test]
fn test_calculate_entropy_high_entropy_jpeg_data() {
    let data: Vec<u8> = (0..4096).map(|i| ((i * 131 + 17) % 251) as u8).collect();
    let e = calculate_entropy(&data);
    assert!(
        e >= 7.0,
        "Pseudo-random data should have high entropy, got {}",
        e
    );
}

#[test]
fn test_calculate_entropy_always_bounded() {
    let data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
    let e = calculate_entropy(&data);
    assert!(e >= 0.0 && e <= 8.0, "Entropy out of bounds: {}", e);
}

#[test]
fn test_calculate_entropy_four_values() {
    let mut data = Vec::new();
    for _ in 0..256 {
        data.push(0);
        data.push(1);
        data.push(2);
        data.push(3);
    }
    let e = calculate_entropy(&data);
    assert!((e - 2.0).abs() < 0.01, "Expected ~2.0, got {}", e);
}

#[test]
fn test_categorize_dimensions_zero_width() {
    assert_eq!(categorize_dimensions(0, 100), DimensionVerdict::TooSmall);
}

#[test]
fn test_categorize_dimensions_zero_height() {
    assert_eq!(categorize_dimensions(100, 0), DimensionVerdict::TooSmall);
}

#[test]
fn test_categorize_dimensions_both_zero() {
    assert_eq!(categorize_dimensions(0, 0), DimensionVerdict::TooSmall);
}

#[test]
fn test_categorize_dimensions_favicon_boundary() {
    assert_eq!(categorize_dimensions(64, 64), DimensionVerdict::TooSmall);
    assert_eq!(categorize_dimensions(65, 65), DimensionVerdict::Asset);
}

#[test]
fn test_categorize_dimensions_icon_boundary() {
    assert_eq!(categorize_dimensions(256, 256), DimensionVerdict::Asset);
    assert_eq!(categorize_dimensions(257, 257), DimensionVerdict::Photo);
}

#[test]
fn test_categorize_dimensions_extreme_aspect_ratio() {
    assert_eq!(categorize_dimensions(1000, 100), DimensionVerdict::Asset);
    assert_eq!(categorize_dimensions(100, 1000), DimensionVerdict::Asset);
}

#[test]
fn test_categorize_dimensions_normal_photo() {
    assert_eq!(categorize_dimensions(1920, 1080), DimensionVerdict::Photo);
    assert_eq!(categorize_dimensions(4000, 3000), DimensionVerdict::Photo);
}

#[test]
fn test_categorize_dimensions_borderline_aspect_ratio() {
    assert_eq!(categorize_dimensions(500, 100), DimensionVerdict::Photo);
    assert_eq!(categorize_dimensions(600, 100), DimensionVerdict::Asset);
}

#[test]
fn test_score_jpeg_zero_width() {
    assert_eq!(score_jpeg(0, 480, &JpegMetadata::default()), 0);
}

#[test]
fn test_score_jpeg_zero_height() {
    assert_eq!(score_jpeg(640, 0, &JpegMetadata::default()), 0);
}

#[test]
fn test_score_jpeg_favicon_vetoed() {
    assert_eq!(score_jpeg(64, 64, &JpegMetadata::default()), 0);
    assert_eq!(score_jpeg(32, 32, &JpegMetadata::default()), 0);
}

#[test]
fn test_score_jpeg_all_metadata_max() {
    let metadata = JpegMetadata {
        has_exif: true,
        has_icc_profile: true,
        has_jfif: true,
        quantization_quality: QuantizationQuality::High,
        marker_count: 20,
        has_sos: true,
        scan_data_entropy: 7.5,
    };
    let score = score_jpeg(2048, 1536, &metadata);
    assert!(
        score >= 80,
        "Full metadata photo should score very high, got {}",
        score
    );
}

#[test]
fn test_score_jpeg_minimal() {
    let metadata = JpegMetadata::default();
    let score = score_jpeg(300, 300, &metadata);
    assert!(score > 0, "Non-favicon should not be vetoed");
    assert!(score < 60, "No metadata should be low, got {}", score);
}

#[test]
fn test_score_jpeg_low_entropy_penalty() {
    let metadata = JpegMetadata {
        has_sos: true,
        scan_data_entropy: 5.0,
        ..JpegMetadata::default()
    };
    let score_low = score_jpeg(800, 600, &metadata);

    let metadata_high = JpegMetadata {
        has_sos: true,
        scan_data_entropy: 7.5,
        ..JpegMetadata::default()
    };
    let score_high = score_jpeg(800, 600, &metadata_high);

    assert!(
        score_high > score_low,
        "High entropy should score better: {} vs {}",
        score_high,
        score_low
    );
}

#[test]
fn test_score_jpeg_extreme_aspect_penalty() {
    let metadata = JpegMetadata {
        has_exif: true,
        ..JpegMetadata::default()
    };
    let normal = score_jpeg(800, 600, &metadata);
    let extreme = score_jpeg(3000, 100, &metadata);
    assert!(
        normal > extreme,
        "Extreme aspect ratio should be penalized: {} vs {}",
        normal,
        extreme
    );
}

#[test]
fn test_score_jpeg_standard_thumb_cap() {
    let metadata = JpegMetadata {
        quantization_quality: QuantizationQuality::Low,
        has_sos: true,
        scan_data_entropy: 7.0,
        marker_count: 5,
        ..JpegMetadata::default()
    };

    let score = score_jpeg(128, 128, &metadata);
    assert!(
        score <= 15,
        "Standard low-quality thumbnail should be capped, got {}",
        score
    );
}

#[test]
fn test_score_jpeg_always_clamped_1_to_100() {
    let metadata = JpegMetadata {
        has_exif: true,
        has_icc_profile: true,
        has_jfif: true,
        quantization_quality: QuantizationQuality::High,
        marker_count: 100,
        has_sos: true,
        scan_data_entropy: 8.0,
    };
    let score = score_jpeg(4000, 3000, &metadata);
    assert!(
        score <= 100,
        "Score should be clamped at 100, got {}",
        score
    );
    assert!(score >= 1);
}

#[test]
fn test_score_jpeg_exif_bonus() {
    let without_exif = JpegMetadata::default();
    let with_exif = JpegMetadata {
        has_exif: true,
        ..JpegMetadata::default()
    };
    let s1 = score_jpeg(800, 600, &without_exif);
    let s2 = score_jpeg(800, 600, &with_exif);
    assert!(s2 > s1, "EXIF should add score: {} vs {}", s2, s1);
}

#[test]
fn test_score_jpeg_icc_bonus() {
    let without = JpegMetadata::default();
    let with = JpegMetadata {
        has_icc_profile: true,
        ..JpegMetadata::default()
    };
    let s1 = score_jpeg(800, 600, &without);
    let s2 = score_jpeg(800, 600, &with);
    assert!(s2 > s1, "ICC profile should add score: {} vs {}", s2, s1);
}

#[test]
fn test_score_jpeg_dimension_tiers() {
    let m = JpegMetadata::default();
    let s_tiny = score_jpeg(100, 100, &m);
    let s_small = score_jpeg(200, 200, &m);
    let s_med = score_jpeg(400, 400, &m);
    let s_big = score_jpeg(1000, 1000, &m);
    let s_huge = score_jpeg(2000, 2000, &m);
    assert!(s_tiny < s_small);
    assert!(s_small < s_med);
    assert!(s_med < s_big);
    assert!(s_big <= s_huge);
}

#[test]
fn test_score_png_zero_dims() {
    assert_eq!(score_png(0, 600, &PngMetadata::default(), 5), 0);
    assert_eq!(score_png(800, 0, &PngMetadata::default(), 5), 0);
}

#[test]
fn test_score_png_no_idat() {
    assert_eq!(score_png(800, 600, &PngMetadata::default(), 0), 0);
}

#[test]
fn test_score_png_favicon_vetoed() {
    assert_eq!(score_png(64, 64, &PngMetadata::default(), 5), 0);
    assert_eq!(score_png(32, 32, &PngMetadata::default(), 5), 0);
}

#[test]
fn test_score_png_rich_metadata() {
    let metadata = PngMetadata {
        has_text_chunks: true,
        has_icc_profile: true,
        has_physical_dimensions: true,
        is_screen_resolution: false,
        chunk_variety: 6,
    };
    let score = score_png(1920, 1080, &metadata, 10);
    assert!(
        score >= 60,
        "Rich PNG should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_png_screen_resolution_penalty() {
    let without_screen = PngMetadata {
        has_physical_dimensions: true,
        is_screen_resolution: false,
        ..PngMetadata::default()
    };
    let with_screen = PngMetadata {
        has_physical_dimensions: true,
        is_screen_resolution: true,
        ..PngMetadata::default()
    };
    let s1 = score_png(800, 600, &without_screen, 5);
    let s2 = score_png(800, 600, &with_screen, 5);
    assert!(
        s1 > s2,
        "Screen resolution should be penalized: {} vs {}",
        s1,
        s2
    );
}

#[test]
fn test_score_png_square_icon_cap() {
    let metadata = PngMetadata {
        chunk_variety: 3,
        ..PngMetadata::default()
    };
    let score = score_png(128, 128, &metadata, 5);
    assert!(
        score <= 15,
        "Square icon without metadata should be capped, got {}",
        score
    );
}

#[test]
fn test_score_png_max_chunk_variety_bonus() {
    let low_var = PngMetadata {
        chunk_variety: 1,
        ..PngMetadata::default()
    };
    let high_var = PngMetadata {
        chunk_variety: 6,
        ..PngMetadata::default()
    };
    let s1 = score_png(800, 600, &low_var, 5);
    let s2 = score_png(800, 600, &high_var, 5);
    assert!(
        s2 > s1,
        "More variety should score higher: {} vs {}",
        s2,
        s1
    );
}

#[test]
fn test_score_png_always_clamped() {
    let metadata = PngMetadata {
        has_text_chunks: true,
        has_icc_profile: true,
        has_physical_dimensions: true,
        is_screen_resolution: false,
        chunk_variety: 6,
    };
    let score = score_png(4000, 3000, &metadata, 100);
    assert!(score >= 1 && score <= 100, "Score out of bounds: {}", score);
}

#[test]
fn test_score_png_extreme_aspect_penalty() {
    let m = PngMetadata::default();
    let normal = score_png(800, 600, &m, 5);
    let extreme = score_png(3000, 100, &m, 5);
    assert!(
        normal > extreme,
        "Extreme aspect should be penalized: {} vs {}",
        normal,
        extreme
    );
}

#[test]
fn test_score_png_text_chunks_bonus() {
    let without = PngMetadata::default();
    let with = PngMetadata {
        has_text_chunks: true,
        ..PngMetadata::default()
    };
    let s1 = score_png(800, 600, &without, 5);
    let s2 = score_png(800, 600, &with, 5);
    assert!(s2 > s1, "Text chunks should add score: {} vs {}", s2, s1);
}
