use argos_core::fixture::MemDisk;
use argos_core::geometry::{Lba, SectorSize};
const SECTOR: SectorSize = SectorSize::new(512);

/// A synthetic medium where byte `i` of sector `s` is `(s * 31 + i) as u8`, so any
/// misplaced read shows up as a content mismatch, not just a length error.
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
fn reads_return_the_bytes_at_the_claimed_offset() {
    let mut disk = MemDisk::new(SECTOR, patterned(8));
    let mut buf = vec![0_u8; SECTOR.get() as usize * 2];
    disk.read_at(Lba::new(3), &mut buf).expect("healthy read");
    assert_eq!(buf, patterned(8)[3 * 512..5 * 512]);
}

#[test]
fn bad_sector_fails_the_covering_read_and_names_the_sector() {
    let mut disk = MemDisk::new(SECTOR, patterned(8)).with_bad_sector(Lba::new(5));
    let mut buf = vec![0_u8; SECTOR.get() as usize * 4];

    let err = disk
        .read_at(Lba::new(4), &mut buf)
        .expect_err("chunk covering the bad sector must fail");
    assert!(err.is_bad_sector());
    assert_eq!(err.lba(), Lba::new(5));

    let mut sector = vec![0_u8; SECTOR.get() as usize];
    disk.read_at(Lba::new(4), &mut sector)
        .expect("neighbouring sector stays readable");
}

#[test]
fn out_of_range_reads_are_rejected_not_truncated() {
    let mut disk = MemDisk::new(SECTOR, patterned(4));
    let mut buf = vec![0_u8; SECTOR.get() as usize * 2];
    let err = disk
        .read_at(Lba::new(3), &mut buf)
        .expect_err("read past the end must fail");
    assert!(err.is_out_of_range());
}
