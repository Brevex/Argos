use argos::types::{
    BlockDevice, ConfidenceTier, DeviceType, Fragment, FragmentKind, FragmentMap, JpegMetadata,
    PngMetadata, QuantizationQuality,
};

#[test]
fn test_fragment_size() {
    assert_eq!(std::mem::size_of::<Fragment>(), 16);
}
#[test]
fn test_fragment_alignment() {
    assert_eq!(std::mem::align_of::<Fragment>(), 8);
}
#[test]
fn test_fragment_kind_size() {
    assert_eq!(std::mem::size_of::<FragmentKind>(), 1);
}
#[test]
fn test_fragment_map_operations() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.8));
    map.push(Fragment::new(1000, FragmentKind::JpegFooter, 0.0));
    assert_eq!(map.len(), 2);
    assert_eq!(map.jpeg_headers().len(), 1);
    assert_eq!(map.jpeg_footers().len(), 1);
}
#[test]
fn test_viable_headers_filter_low_entropy() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.8));
    map.push(Fragment::new(1000, FragmentKind::JpegHeader, 3.0));
    map.push(Fragment::new(2000, FragmentKind::JpegHeader, 5.6));
    assert_eq!(map.jpeg_headers().len(), 3);
    assert_eq!(map.viable_jpeg_headers().count(), 2);
}
#[test]
fn test_size_human() {
    let device = BlockDevice {
        name: "sda".to_string(),
        device_type: DeviceType::Hdd,
        size: 1_000_000_000_000,
        path: "/dev/sda".to_string(),
    };
    assert!(device.size_human().contains("TB") || device.size_human().contains("GB"));
}

#[test]
fn test_confidence_tier_high() {
    assert_eq!(ConfidenceTier::from_score(100), ConfidenceTier::High);
    assert_eq!(ConfidenceTier::from_score(60), ConfidenceTier::High);
    assert_eq!(ConfidenceTier::from_score(80), ConfidenceTier::High);
}

#[test]
fn test_confidence_tier_partial() {
    assert_eq!(ConfidenceTier::from_score(59), ConfidenceTier::Partial);
    assert_eq!(ConfidenceTier::from_score(30), ConfidenceTier::Partial);
    assert_eq!(ConfidenceTier::from_score(45), ConfidenceTier::Partial);
}

#[test]
fn test_confidence_tier_low() {
    assert_eq!(ConfidenceTier::from_score(29), ConfidenceTier::Low);
    assert_eq!(ConfidenceTier::from_score(1), ConfidenceTier::Low);
    assert_eq!(ConfidenceTier::from_score(15), ConfidenceTier::Low);
}

#[test]
fn test_confidence_tier_dirnames() {
    assert_eq!(ConfidenceTier::High.dirname(), "high");
    assert_eq!(ConfidenceTier::Partial.dirname(), "partial");
    assert_eq!(ConfidenceTier::Low.dirname(), "low");
}

#[test]
fn test_score_jpeg_high_confidence_photo() {
    let metadata = JpegMetadata {
        has_exif: true,
        has_icc_profile: true,
        has_jfif: false,
        quantization_quality: QuantizationQuality::High,
        marker_count: 8,
        has_sos: true,
        scan_data_entropy: 7.5,
    };
    let score = argos::types::score_jpeg(2048, 1536, &metadata);
    assert!(
        score >= 60,
        "Camera photo should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_jpeg_camera_phone_with_exif() {
    let metadata = JpegMetadata {
        has_exif: true,
        has_icc_profile: false,
        has_jfif: true,
        quantization_quality: QuantizationQuality::Medium,
        marker_count: 5,
        has_sos: true,
        scan_data_entropy: 7.0,
    };
    let score = argos::types::score_jpeg(640, 480, &metadata);
    assert!(
        score >= 60,
        "Camera phone with EXIF should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_jpeg_favicon_vetoed() {
    let metadata = JpegMetadata::default();
    let score = argos::types::score_jpeg(32, 32, &metadata);
    assert_eq!(score, 0, "Favicon should be hard-vetoed");
}

#[test]
fn test_score_jpeg_low_quality_thumbnail() {
    let metadata = JpegMetadata {
        has_exif: false,
        has_icc_profile: false,
        has_jfif: false,
        quantization_quality: QuantizationQuality::Low,
        marker_count: 3,
        has_sos: true,
        scan_data_entropy: 7.0,
    };
    let score = argos::types::score_jpeg(128, 128, &metadata);
    assert!(
        score <= 15,
        "System thumbnail should be capped low, got {}",
        score
    );
}

#[test]
fn test_score_png_with_metadata() {
    let metadata = PngMetadata {
        has_text_chunks: true,
        has_icc_profile: true,
        has_physical_dimensions: true,
        is_screen_resolution: false,
        chunk_variety: 6,
    };
    let score = argos::types::score_png(1920, 1080, &metadata, 10);
    assert!(
        score >= 60,
        "Rich PNG should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_png_no_idat_vetoed() {
    let metadata = PngMetadata::default();
    let score = argos::types::score_png(800, 600, &metadata, 0);
    assert_eq!(score, 0, "PNG with no IDAT should be vetoed");
}

#[test]
fn test_score_png_screen_icon() {
    let metadata = PngMetadata {
        has_text_chunks: false,
        has_icc_profile: false,
        has_physical_dimensions: true,
        is_screen_resolution: true,
        chunk_variety: 2,
    };
    let score = argos::types::score_png(128, 128, &metadata, 1);
    assert!(
        score < 30,
        "Screen icon should be low confidence, got {}",
        score
    );
}
