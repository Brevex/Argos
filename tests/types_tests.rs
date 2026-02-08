use argos::types::{BlockDevice, DeviceType, Fragment, FragmentKind, FragmentMap};
#[test]
fn test_fragment_size() {
    assert_eq!(std::mem::size_of::<Fragment>(), 32);
}
#[test]
fn test_fragment_alignment() {
    assert_eq!(std::mem::align_of::<Fragment>(), 32);
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
    assert_eq!(map.jpeg_headers().count(), 1);
    assert_eq!(map.jpeg_footers().count(), 1);
}
#[test]
fn test_viable_headers_filter_low_entropy() {
    let mut map = FragmentMap::new();
    map.push(Fragment::new(0, FragmentKind::JpegHeader, 7.8));
    map.push(Fragment::new(1000, FragmentKind::JpegHeader, 3.0));
    map.push(Fragment::new(2000, FragmentKind::JpegHeader, 5.6));
    assert_eq!(map.jpeg_headers().count(), 3);
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
