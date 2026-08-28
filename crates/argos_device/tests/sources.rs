//! The adapter layer seen from outside: every `BlockSource` behaves
//! identically through the port, and `BlockReader` yields the medium's bytes
//! at every alignment.
//!
//! `BlockReader` is what every recovery crate actually consumes, so what
//! matters is that the bytes it hands out are exactly the bytes on the
//! medium, whatever the offset and buffer size ask for.

use std::io::{Read, Seek, SeekFrom, Write as _};

use argos_core::fixture::MemDisk;
use argos_core::ports::DeviceClass;
use argos_core::{Lba, SectorSize};
use argos_device::{BlockReader, Device, ImageSource};

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

const SECTOR_BYTES: usize = 512;

/// A disk whose every byte is a function of its offset, so a misread is
/// visible as a wrong value rather than as plausible-looking data.
fn ramp(sectors: usize) -> Vec<u8> {
    (0..sectors * SECTOR_BYTES)
        .map(|i| u8::try_from(i % 251).expect("modulo 251 fits u8"))
        .collect()
}

fn reader(data: Vec<u8>) -> BlockReader<MemDisk> {
    BlockReader::new(MemDisk::new(SectorSize::new(512), data))
}

#[test]
fn reads_the_whole_medium_byte_for_byte() {
    let data = ramp(8);
    let mut src = reader(data.clone());

    let mut out = Vec::new();
    src.read_to_end(&mut out).expect("read to end");

    assert_eq!(out, data);
    assert_eq!(src.len(), data.len() as u64);
}

#[test]
fn unaligned_reads_yield_the_same_bytes_as_aligned_ones() {
    let data = ramp(4);
    let mut src = reader(data.clone());

    // Start mid-sector and use a buffer that is neither a sector multiple nor
    // a divisor of one — the layout a parser reading a struct produces.
    src.seek(SeekFrom::Start(777)).expect("seek");
    let mut out = vec![0_u8; 1000];
    src.read_exact(&mut out).expect("read exact across sectors");

    assert_eq!(out, data[777..1777]);
}

#[test]
fn seeking_past_the_end_reads_nothing_rather_than_failing() {
    let mut src = reader(ramp(2));

    let at = src.seek(SeekFrom::End(4096)).expect("seek past end");
    assert_eq!(at, 2 * SECTOR_BYTES as u64 + 4096);

    let mut out = [0_u8; 16];
    assert_eq!(src.read(&mut out).expect("read past end"), 0);
}

#[test]
fn seeking_before_the_start_is_an_error_not_a_wrapped_offset() {
    let mut src = reader(ramp(2));

    let err = src
        .seek(SeekFrom::Current(-1))
        .expect_err("a position before byte zero is not addressable");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn a_bad_sector_surfaces_as_an_error_never_as_zeros() {
    let disk = MemDisk::new(SectorSize::new(512), ramp(4)).with_bad_sector(Lba::new(2));
    let mut src = BlockReader::new(disk);

    src.seek(SeekFrom::Start(2 * SECTOR_BYTES as u64))
        .expect("seek");
    let mut out = [0_u8; SECTOR_BYTES];
    let err = src
        .read_exact(&mut out)
        .expect_err("an unreadable sector must not be fabricated as data");

    assert!(err.to_string().contains("bad sector"));
}

#[test]
#[cfg_attr(miri, ignore = "creates a real file on disk")]
fn a_trailing_partial_sector_of_a_truncated_image_is_not_addressable() {
    // A `dd` cut short mid-sector: the geometry addresses 3 sectors, and the
    // reader must not hand out the half sector the medium cannot address.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("truncated.img");
    let mut data = ramp(3);
    data.extend(std::iter::repeat_n(0xAB, SECTOR_BYTES / 2));
    std::fs::write(&path, &data).expect("write image");

    let image = argos_device::ImageSource::open(&path).expect("open image");
    assert_eq!(image.trailing_bytes(), SECTOR_BYTES as u64 / 2);
    let mut src = BlockReader::new(image);

    let mut out = Vec::new();
    src.read_to_end(&mut out).expect("read to end");

    assert_eq!(out.len(), 3 * SECTOR_BYTES);
    assert_eq!(out, data[..3 * SECTOR_BYTES]);
}
