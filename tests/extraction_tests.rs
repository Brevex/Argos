use argos::extraction::generate_filename;
use argos::types::ImageFormat;
#[test]
fn test_generate_filename() {
    assert_eq!(
        generate_filename(0, ImageFormat::Jpeg),
        "recovered_000000.jpg"
    );
    assert_eq!(
        generate_filename(123, ImageFormat::Png),
        "recovered_000123.png"
    );
}
