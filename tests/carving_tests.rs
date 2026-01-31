use argos::carving::{linear_carve, RecoveryStats};
use argos::types::{Fragment, FragmentKind, ImageFormat, RecoveredFile, RecoveryMethod};
#[test]
fn test_linear_carve_contiguous_jpeg() {
    let fragments = vec![
        Fragment::new(0, 0, FragmentKind::JpegHeader, 7.8),
        Fragment::new(1000, 2, FragmentKind::JpegFooter, 0.0),
    ];
    let recovered = linear_carve(&fragments);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].fragments[0].start, 0);
    assert_eq!(recovered[0].fragments[0].end, 1002);
}
#[test]
fn test_linear_carve_multiple_jpegs() {
    let fragments = vec![
        Fragment::new(0, 0, FragmentKind::JpegHeader, 7.8),
        Fragment::new(1000, 2, FragmentKind::JpegFooter, 0.0),
        Fragment::new(2000, 0, FragmentKind::JpegHeader, 7.9),
        Fragment::new(3000, 2, FragmentKind::JpegFooter, 0.0),
    ];
    let recovered = linear_carve(&fragments);
    assert_eq!(recovered.len(), 2);
}
#[test]
fn test_linear_carve_png() {
    let fragments = vec![
        Fragment::new(0, 0, FragmentKind::PngHeader, 7.5),
        Fragment::new(5000, 12, FragmentKind::PngIend, 0.0),
    ];
    let recovered = linear_carve(&fragments);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].format, ImageFormat::Png);
}
#[test]
fn test_recovery_stats() {
    let files = vec![
        RecoveredFile::new(vec![0..1000], RecoveryMethod::Linear, ImageFormat::Jpeg),
        RecoveredFile::new(vec![2000..3000], RecoveryMethod::Linear, ImageFormat::Jpeg),
        RecoveredFile::new(vec![5000..6000], RecoveryMethod::Linear, ImageFormat::Png),
    ];
    let stats = RecoveryStats::from_recovered(&files);
    assert_eq!(stats.jpeg_linear, 2);
    assert_eq!(stats.png_linear, 1);
    assert_eq!(stats.total_files(), 3);
}
