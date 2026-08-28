//! Phase 1 exit criterion: acquiring a medium with injected read errors yields a
//! sector-accurate image, with every unreadable sector mapped in the report.

use std::io::Cursor;

use argos_core::fixture::MemDisk;
use argos_core::{Lba, SectorRange, SectorSize};
use argos_device::acquire;

const SECTOR: SectorSize = SectorSize::new(512);
const SECTOR_BYTES: usize = 512;

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
fn healthy_medium_is_acquired_bit_identical() {
    let data = patterned(300);
    let mut src = MemDisk::new(SECTOR, data.clone());
    let mut dest = Cursor::new(Vec::new());

    let report = acquire::run(
        &mut src,
        &mut dest,
        acquire::Options::new(),
        &mut |_| {},
        &|| false,
    )
    .expect("dest is ram");

    assert!(report.is_complete());
    assert_eq!(report.recovered_sectors(), 300);
    assert_eq!(dest.into_inner(), data);
}

#[test]
fn bad_sectors_are_zero_filled_and_mapped_exactly() {
    // Bad sectors 7 and 8 (adjacent, must merge) and 130 (a different sweep chunk).
    let data = patterned(300);
    let mut src = MemDisk::new(SECTOR, data.clone())
        .with_bad_sector(Lba::new(7))
        .with_bad_sector(Lba::new(8))
        .with_bad_sector(Lba::new(130));
    let mut dest = Cursor::new(Vec::new());

    let report = acquire::run(
        &mut src,
        &mut dest,
        acquire::Options::new(),
        &mut |_| {},
        &|| false,
    )
    .expect("dest is ram");
    let image = dest.into_inner();

    assert_eq!(
        report.unreadable(),
        [
            SectorRange::new(Lba::new(7), 2),
            SectorRange::new(Lba::new(130), 1),
        ]
    );
    assert_eq!(report.recovered_sectors(), 297);
    assert!(!report.is_complete());

    // Sector accuracy: every recovered byte matches the source at the same
    // offset; every unreadable sector is a zero placeholder.
    assert_eq!(image.len(), data.len());
    for sector in 0..300_usize {
        let range = sector * SECTOR_BYTES..(sector + 1) * SECTOR_BYTES;
        if [7, 8, 130].contains(&sector) {
            assert!(
                image[range].iter().all(|&byte| byte == 0),
                "unreadable sector {sector} must be a zero placeholder"
            );
        } else {
            assert_eq!(
                image[range.clone()],
                data[range],
                "recovered sector {sector} must match the source"
            );
        }
    }
}

#[test]
fn small_chunks_and_trailing_partial_chunk_are_handled() {
    let data = patterned(10);
    let mut src = MemDisk::new(SECTOR, data.clone()).with_bad_sector(Lba::new(9));
    let mut dest = Cursor::new(Vec::new());
    let options = acquire::Options::new().with_chunk_sectors(4);

    let report =
        acquire::run(&mut src, &mut dest, options, &mut |_| {}, &|| false).expect("dest is ram");

    assert_eq!(report.unreadable(), [SectorRange::new(Lba::new(9), 1)]);
    assert_eq!(
        dest.into_inner()[..9 * SECTOR_BYTES],
        data[..9 * SECTOR_BYTES]
    );
}

#[test]
fn both_passes_report_progress_that_reaches_their_own_total() {
    // A pass over a terabyte takes hours, and one that emits nothing is
    // indistinguishable from one that has stopped. The two passes count
    // different things: the sweep's denominator is the medium, refinement's is
    // only what the sweep could not read.
    let sectors = 4_000;
    let data = patterned(sectors);
    let mut src = MemDisk::new(SECTOR, data)
        .with_bad_sector(Lba::new(500))
        .with_bad_sector(Lba::new(2_000));
    let mut dest = Cursor::new(Vec::new());

    let mut swept = Vec::new();
    let mut refined = Vec::new();
    let report = acquire::run(
        &mut src,
        &mut dest,
        acquire::Options::new(),
        &mut |progress| match progress {
            acquire::Progress::Swept { done, total } => swept.push((done, total)),
            acquire::Progress::Refined { done, total } => refined.push((done, total)),
        },
        &|| false,
    )
    .expect("dest is ram");

    assert_eq!(
        swept.last().copied(),
        Some((sectors, sectors)),
        "the sweep must finish at the medium's own size: {swept:?}"
    );
    // Two failing sectors in different sweep chunks, so refinement revisits
    // both chunks entirely — the denominator is the suspect sectors, not two.
    let suspects = report
        .unreadable()
        .iter()
        .map(|range| range.sectors)
        .sum::<u64>();
    assert!(suspects > 0, "the fixture must leave something to refine");
    let (done, total) = refined.last().copied().expect("refinement must report");
    assert_eq!(done, total, "refinement must finish at its own total");
    assert!(
        total < sectors,
        "refinement counts what the sweep could not read, not the medium: {total} of {sectors}"
    );

    // Bounded, or a medium of two billion sectors emits two billion callbacks.
    assert!(
        swept.len() <= 201 && refined.len() <= 201,
        "progress must be capped: {} sweep, {} refine",
        swept.len(),
        refined.len()
    );
}

#[test]
fn a_stopped_acquisition_counts_what_it_never_reached_apart_from_what_the_medium_refused() {
    // A run its operator stopped says nothing about the medium. Folding the
    // sectors it never reached into the unreadable map would turn a cancelled
    // copy into a report of a damaged disk, which is the one thing this report
    // exists to state precisely (`A-CONFIDENCE-HONEST`).
    const SECTORS: u64 = 4096;
    let sectors = SECTORS;
    let bytes = usize::try_from(SECTORS * u64::from(SECTOR.get())).expect("the fixture fits");
    let data = vec![0xA5_u8; bytes];
    let mut src = MemDisk::new(SECTOR, data);
    let mut dest = Cursor::new(Vec::new());

    // Stop as soon as the first chunk has been reported, so the run ends well
    // inside the medium.
    let seen = std::cell::Cell::new(0_u32);
    let report = acquire::run(
        &mut src,
        &mut dest,
        acquire::Options::new(),
        &mut |_| seen.set(seen.get() + 1),
        &|| seen.get() > 0,
    )
    .expect("dest is ram");

    assert!(
        report.stopped_early(),
        "the run was stopped and must say so"
    );
    assert!(
        report.not_attempted() > 0,
        "a stopped run leaves sectors it never reached"
    );
    assert!(
        report.unreadable().is_empty(),
        "a healthy medium refused nothing; stopping is not damage: {:?}",
        report.unreadable()
    );
    assert!(
        !report.is_complete(),
        "a run that did not cover the medium is not a complete copy"
    );
    assert_eq!(
        report.recovered_sectors() + report.not_attempted(),
        sectors,
        "every sector is either recovered or untried, and none is counted twice"
    );
    assert!(
        report.recovered_sectors() > 0,
        "what was copied before the stop stays copied"
    );
}
