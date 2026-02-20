use argos::carving::RecoveryStats;
use argos::types::{FragmentRanges, ImageFormat, RecoveredFile, RecoveryMethod};

#[test]
fn test_recovery_stats() {
    let files = vec![
        RecoveredFile::new(
            FragmentRanges::Linear(0..1000),
            RecoveryMethod::Linear,
            ImageFormat::Jpeg,
            7.5,
        ),
        RecoveredFile::new(
            FragmentRanges::Linear(2000..3000),
            RecoveryMethod::Linear,
            ImageFormat::Jpeg,
            7.6,
        ),
        RecoveredFile::new(
            FragmentRanges::Linear(5000..6000),
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

#[test]
fn test_recovery_stats_bifragment() {
    let files = vec![
        RecoveredFile::new(
            FragmentRanges::Bifragment([0..500, 1000..1500]),
            RecoveryMethod::Bifragment,
            ImageFormat::Jpeg,
            7.8,
        ),
        RecoveredFile::new(
            FragmentRanges::Linear(2000..3000),
            RecoveryMethod::Linear,
            ImageFormat::Png,
            7.0,
        ),
    ];
    let stats = RecoveryStats::from_recovered(&files);
    assert_eq!(stats.jpeg_bifragment, 1);
    assert_eq!(stats.png_linear, 1);
    assert_eq!(stats.total_files(), 2);
}

#[test]
fn test_recovery_stats_empty() {
    let stats = RecoveryStats::from_recovered(&[]);
    assert_eq!(stats.total_files(), 0);
}
