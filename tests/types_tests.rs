use argos::core::{
    BlockDevice, ConfidenceTier, DeviceType, ExtractionError, Fragment, FragmentKind, FragmentMap,
    ImageFormat, JpegMetadata, PngMetadata, QuantizationQuality,
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
    let score = argos::core::score_jpeg(2048, 1536, &metadata);
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
    let score = argos::core::score_jpeg(640, 480, &metadata);
    assert!(
        score >= 60,
        "Camera phone with EXIF should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_jpeg_favicon_vetoed() {
    let metadata = JpegMetadata::default();
    let score = argos::core::score_jpeg(32, 32, &metadata);
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
    let score = argos::core::score_jpeg(128, 128, &metadata);
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
    let score = argos::core::score_png(1920, 1080, &metadata, 10);
    assert!(
        score >= 60,
        "Rich PNG should be high confidence, got {}",
        score
    );
}

#[test]
fn test_score_png_no_idat_vetoed() {
    let metadata = PngMetadata::default();
    let score = argos::core::score_png(800, 600, &metadata, 0);
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
    let score = argos::core::score_png(128, 128, &metadata, 1);
    assert!(
        score < 30,
        "Screen icon should be low confidence, got {}",
        score
    );
}

#[test]
fn test_fragment_map_sort_by_offset() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(3000, FragmentKind::JpegHeader, 7.0));
    map.push(Fragment::new(1000, FragmentKind::JpegHeader, 7.5));
    map.push(Fragment::new(2000, FragmentKind::JpegHeader, 7.2));
    map.sort_by_offset();
    let offsets: Vec<_> = map.jpeg_headers().iter().map(|f| f.offset).collect();
    assert_eq!(offsets, vec![1000, 2000, 3000]);
}

#[test]
fn test_fragment_map_dedup_removes_same_offset() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(1000, FragmentKind::PngHeader, 7.0));
    map.push(Fragment::new(1000, FragmentKind::PngHeader, 7.5));
    map.push(Fragment::new(2000, FragmentKind::PngHeader, 7.0));
    map.sort_by_offset();
    map.dedup();
    assert_eq!(map.png_headers().len(), 2);
}

#[test]
fn test_fragment_map_count_by_kind() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.0));
    map.push(Fragment::new(100, FragmentKind::JpegFooter, 0.0));
    map.push(Fragment::new(200, FragmentKind::PngHeader, 6.5));
    map.push(Fragment::new(300, FragmentKind::PngIend, 0.0));
    map.push(Fragment::new(400, FragmentKind::PngHeader, 7.0));
    let counts = map.count_by_kind();
    assert_eq!(counts.jpeg_headers, 1);
    assert_eq!(counts.jpeg_footers, 1);
    assert_eq!(counts.png_headers, 2);
    assert_eq!(counts.png_footers, 1);
}

#[test]
fn test_fragment_map_is_empty() {
    let map = FragmentMap::new();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
}

#[test]
fn test_fragment_map_default() {
    let map = FragmentMap::default();
    assert!(map.is_empty());
}

#[test]
fn test_fragment_map_with_disk_estimate() {
    let map = FragmentMap::with_disk_estimate(1_000_000_000);
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
}

#[test]
fn test_fragment_map_viable_headers() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::PngHeader, 7.0));
    map.push(Fragment::new(100, FragmentKind::PngHeader, 3.0));
    map.push(Fragment::new(200, FragmentKind::PngHeader, 6.8));
    assert_eq!(map.viable_png_headers().count(), 2);
}

#[test]
fn test_image_format_extension_jpeg() {
    assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
}

#[test]
fn test_image_format_extension_png() {
    assert_eq!(ImageFormat::Png.extension(), "png");
}

#[test]
fn test_extraction_error_from_io_write_zero() {
    let io_err = std::io::Error::new(std::io::ErrorKind::WriteZero, "full");
    let err = ExtractionError::from(io_err);
    assert!(matches!(err, ExtractionError::DiskFull));
}

#[test]
fn test_extraction_error_is_fatal() {
    assert!(ExtractionError::DiskFull.is_fatal());
    assert!(ExtractionError::DeviceDisconnected.is_fatal());
    let io_err = std::io::Error::new(std::io::ErrorKind::Other, "something");
    assert!(!ExtractionError::from(io_err).is_fatal());
}

#[test]
fn test_extraction_error_display() {
    assert!(format!("{}", ExtractionError::DiskFull).contains("full"));
    assert!(format!("{}", ExtractionError::DeviceDisconnected).contains("disconnect"));
}

#[cfg(unix)]
#[test]
fn test_extraction_error_from_enospc() {
    let io_err = std::io::Error::from_raw_os_error(libc::ENOSPC);
    let err = ExtractionError::from(io_err);
    assert!(matches!(err, ExtractionError::DiskFull));
}

#[cfg(unix)]
#[test]
fn test_extraction_error_from_eio() {
    let io_err = std::io::Error::from_raw_os_error(libc::EIO);
    let err = ExtractionError::from(io_err);
    assert!(matches!(err, ExtractionError::DeviceDisconnected));
}

#[test]
fn test_fragment_has_viable_entropy() {
    let high = Fragment::new(0, FragmentKind::JpegHeader, 7.5);
    let low = Fragment::new(0, FragmentKind::JpegHeader, 3.0);
    assert!(high.has_viable_entropy());
    assert!(!low.has_viable_entropy());
}

#[test]
fn test_confidence_tier_boundaries() {
    assert_eq!(ConfidenceTier::from_score(0), ConfidenceTier::Low);
    assert_eq!(ConfidenceTier::from_score(255), ConfidenceTier::High);
}

#[test]
fn test_block_device_size_human_bytes() {
    let dev = BlockDevice {
        name: "x".into(),
        device_type: DeviceType::Unknown,
        size: 500,
        path: "/dev/x".into(),
    };
    assert!(dev.size_human().contains("B"));
}

#[test]
fn test_block_device_size_human_kb() {
    let dev = BlockDevice {
        name: "x".into(),
        device_type: DeviceType::Unknown,
        size: 2048,
        path: "/dev/x".into(),
    };
    assert!(dev.size_human().contains("KB"));
}

#[test]
fn test_block_device_size_human_mb() {
    let dev = BlockDevice {
        name: "x".into(),
        device_type: DeviceType::Unknown,
        size: 5_000_000,
        path: "/dev/x".into(),
    };
    assert!(dev.size_human().contains("MB"));
}

#[test]
fn test_block_device_size_human_gb() {
    let dev = BlockDevice {
        name: "x".into(),
        device_type: DeviceType::Unknown,
        size: 5_000_000_000,
        path: "/dev/x".into(),
    };
    assert!(dev.size_human().contains("GB"));
}

#[test]
fn test_device_type_display() {
    assert_eq!(format!("{}", DeviceType::Hdd), "HDD");
    assert_eq!(format!("{}", DeviceType::Ssd), "SSD");
    assert_eq!(format!("{}", DeviceType::NVMe), "NVMe");
    assert_eq!(format!("{}", DeviceType::Usb), "USB");
    assert_eq!(format!("{}", DeviceType::Unknown), "Unknown");
}
