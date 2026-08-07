//! Image-file and mocked-device adapters behave identically through the port.

use std::io::Write as _;

use argos_core::fixture::MemDisk;
use argos_core::geometry::{Lba, SectorSize};
use argos_core::source::DeviceClass;
use argos_device::{Device, ImageSource};

const SECTOR: SectorSize = SectorSize::new(512);

#[expect(
    clippy::cast_possible_truncation,
    reason = "the byte pattern intentionally wraps modulo 256"
)]
fn patterned(sectors: u64) -> Vec<u8> {
    (0..sectors)
        .flat_map(|s| (0..u64::from(SECTOR.get())).map(move |i| (s * 31 + i) as u8))
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "opens real files")]
fn image_source_reads_the_bytes_at_the_claimed_offset() {
    let data = patterned(64);
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(&data).expect("write fixture image");

    let mut src = ImageSource::open(file.path()).expect("open image read-only");
    let geometry = src.geometry();
    assert_eq!(geometry.sector_count, 64);
    assert_eq!(geometry.class, DeviceClass::ImageFile);

    let mut buf = vec![0_u8; 512 * 3];
    src.read_at(Lba::new(10), &mut buf).expect("healthy read");
    assert_eq!(buf, data[10 * 512..13 * 512]);

    let err = src
        .read_at(Lba::new(63), &mut buf)
        .expect_err("read past the end must fail");
    assert!(err.is_out_of_range());
}

#[test]
#[cfg_attr(miri, ignore = "opens real files")]
fn image_source_ignores_a_trailing_partial_sector() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(&patterned(4)).expect("write whole sectors");
    file.write_all(&[0xAB; 100]).expect("write partial sector");

    let src = ImageSource::open(file.path()).expect("open image read-only");
    assert_eq!(src.geometry().sector_count, 4);
    assert_eq!(
        src.trailing_bytes(),
        100,
        "the excluded tail must be disclosed, never silently dropped"
    );
}

#[test]
fn mocked_device_serves_the_fixture_and_counts_reads() {
    let disk = MemDisk::new(SECTOR, patterned(16)).with_bad_sector(Lba::new(3));
    let (mut device, ctrl) = Device::new_mocked(disk);

    assert_eq!(device.geometry().sector_count, 16);

    let mut buf = vec![0_u8; 512];
    device.read_at(Lba::new(0), &mut buf).expect("healthy read");
    let err = device
        .read_at(Lba::new(3), &mut buf)
        .expect_err("scripted bad sector must fail");
    assert!(err.is_bad_sector());
    assert_eq!(ctrl.reads(), 2);
}

#[cfg(target_os = "linux")]
#[test]
#[cfg_attr(miri, ignore = "opens real files")]
fn opening_a_regular_file_as_a_device_is_rejected() {
    let file = tempfile::NamedTempFile::new().expect("temp file");
    let err = Device::open(file.path()).expect_err("a regular file is not a block device");
    assert!(err.is_not_block_device());
    assert_eq!(err.path(), file.path());
}
