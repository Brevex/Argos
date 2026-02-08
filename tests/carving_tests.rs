use argos::carving::{linear_carve, RecoveryStats};
use argos::types::{
    Fragment, FragmentKind, FragmentMap, ImageFormat, RecoveredFile, RecoveryMethod,
};

#[test]
fn test_linear_carve_contiguous_jpeg() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.8));
    map.push(Fragment::new(4096, FragmentKind::JpegFooter, 0.0));
    let recovered = linear_carve(&map);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].fragments[0].start, 0);
    assert_eq!(recovered[0].fragments[0].end, 4098);
}

#[test]
fn test_linear_carve_multiple_jpegs() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.8));
    map.push(Fragment::new(4096, FragmentKind::JpegFooter, 0.0));
    map.push(Fragment::new(8192, FragmentKind::JpegHeader, 7.9));
    map.push(Fragment::new(12288, FragmentKind::JpegFooter, 0.0));
    let recovered = linear_carve(&map);
    assert_eq!(recovered.len(), 2);
}

#[test]
fn test_linear_carve_png() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::PngHeader, 7.5));
    map.push(Fragment::new(8192, FragmentKind::PngIend, 0.0));
    let recovered = linear_carve(&map);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].format, ImageFormat::Png);
}

#[test]
fn test_linear_carve_filters_low_entropy() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 3.0));
    map.push(Fragment::new(4096, FragmentKind::JpegFooter, 0.0));
    let recovered = linear_carve(&map);
    assert_eq!(recovered.len(), 0);
}

#[test]
fn test_recovery_stats() {
    let files = vec![
        RecoveredFile::new(
            vec![0..1000],
            RecoveryMethod::Linear,
            ImageFormat::Jpeg,
            7.5,
        ),
        RecoveredFile::new(
            vec![2000..3000],
            RecoveryMethod::Linear,
            ImageFormat::Jpeg,
            7.6,
        ),
        RecoveredFile::new(
            vec![5000..6000],
            RecoveryMethod::Linear,
            ImageFormat::Png,
            7.2,
        ),
    ];
    let stats = RecoveryStats::from_recovered(&files);
    assert_eq!(stats.jpeg_linear, 2);
    assert_eq!(stats.png_linear, 1);
    assert_eq!(stats.total_files(), 3);
}
