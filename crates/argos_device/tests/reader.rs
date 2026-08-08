//! `BlockReader` turns a sector-addressed medium into the `Read + Seek` view
//! every recovery crate consumes. What matters is that the bytes it yields are
//! exactly the bytes on the medium, at every alignment.

use std::io::{Read, Seek, SeekFrom};

use argos_core::fixture::MemDisk;
use argos_core::geometry::{Lba, SectorSize};
use argos_device::BlockReader;

const SECTOR: usize = 512;

/// A disk whose every byte is a function of its offset, so a misread is
/// visible as a wrong value rather than as plausible-looking data.
fn patterned(sectors: usize) -> Vec<u8> {
    (0..sectors * SECTOR)
        .map(|i| u8::try_from(i % 251).expect("modulo 251 fits u8"))
        .collect()
}

fn reader(data: Vec<u8>) -> BlockReader<MemDisk> {
    BlockReader::new(MemDisk::new(SectorSize::new(512), data))
}

#[test]
fn reads_the_whole_medium_byte_for_byte() {
    let data = patterned(8);
    let mut src = reader(data.clone());

    let mut out = Vec::new();
    src.read_to_end(&mut out).expect("read to end");

    assert_eq!(out, data);
    assert_eq!(src.len(), data.len() as u64);
}

#[test]
fn unaligned_reads_yield_the_same_bytes_as_aligned_ones() {
    let data = patterned(4);
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
    let mut src = reader(patterned(2));

    let at = src.seek(SeekFrom::End(4096)).expect("seek past end");
    assert_eq!(at, 2 * SECTOR as u64 + 4096);

    let mut out = [0_u8; 16];
    assert_eq!(src.read(&mut out).expect("read past end"), 0);
}

#[test]
fn seeking_before_the_start_is_an_error_not_a_wrapped_offset() {
    let mut src = reader(patterned(2));

    let err = src
        .seek(SeekFrom::Current(-1))
        .expect_err("a position before byte zero is not addressable");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn a_bad_sector_surfaces_as_an_error_never_as_zeros() {
    let disk = MemDisk::new(SectorSize::new(512), patterned(4)).with_bad_sector(Lba::new(2));
    let mut src = BlockReader::new(disk);

    src.seek(SeekFrom::Start(2 * SECTOR as u64)).expect("seek");
    let mut out = [0_u8; SECTOR];
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
    let mut data = patterned(3);
    data.extend(std::iter::repeat_n(0xAB, SECTOR / 2));
    std::fs::write(&path, &data).expect("write image");

    let image = argos_device::ImageSource::open(&path).expect("open image");
    assert_eq!(image.trailing_bytes(), SECTOR as u64 / 2);
    let mut src = BlockReader::new(image);

    let mut out = Vec::new();
    src.read_to_end(&mut out).expect("read to end");

    assert_eq!(out.len(), 3 * SECTOR);
    assert_eq!(out, data[..3 * SECTOR]);
}
