//! What the graph walk costs on the shape of medium that makes it expensive.
//!
//! The walk is told which bytes another recovery already accounts for, so it
//! does not offer them again. On a fixture that list holds a handful of
//! extents. On a disk it holds one per recovered artifact — a scan of a
//! terabyte handed it **1,639,834** — and the walk consults it once per
//! candidate block it considers, scanning the whole list each time.
//!
//! A candidate block is 4 KiB and a held region is 256 MiB, so that is up to
//! 65,536 consultations per node of the walk. The cost is the product of the
//! two largest numbers a real medium produces, and nothing about the *answer*
//! depends on either: extents claimed elsewhere can never match a block here.
//!
//! Measured on this machine, the product law is exact — 16 times the blocks is
//! 16 times the time, four times the claimed set is four times the time, and
//! with an empty claimed set the cost disappears:
//!
//! | candidate blocks | claimed extents | walk |
//! | --- | --- | --- |
//! | 8,192 | 0 | 0.057 s |
//! | 8,192 | 100,000 | 1.455 s |
//! | 8,192 | 400,000 | 5.759 s |
//!
//! That is a stage which finishes in a moment on every fixture and does not
//! finish at all in the field. These tests assert the shape rather than a
//! number, so they mean the same thing in debug and in release.

use std::time::Instant;

use argos_carve::classify::classify;
use argos_carve::fixture::{Fragmented, fragmented, photo_jpeg};
use argos_carve::reassemble::{self, Broken, Candidate, Limits};
use argos_carve::{Format, Scratch};
use argos_core::geometry::{ByteOffset, ByteRange};

/// Block size the fixtures fragment on: the reassembly search grid.
const BLOCK: usize = argos_carve::classify::BLOCK_BYTES;

/// Blocks the planted medium holds.
///
/// Enough that the per-block consultation dominates the one-off cost of
/// setting a walk up, and small enough that a fixed walk is instant.
const BLOCKS: usize = 2048;

/// Extents a previous stage claimed, in the arrangement a real scan produces.
const CLAIMED: u64 = 400_000;

fn planted() -> Fragmented {
    fragmented(
        BLOCKS * BLOCK,
        &photo_jpeg(320, 240, 0x1234_5678_9ABC_DEF0),
        &[4 * BLOCK, 20 * BLOCK],
        BLOCK,
    )
}

fn candidate_blocks(layout: &Fragmented) -> Vec<Candidate> {
    layout
        .disk
        .chunks(BLOCK)
        .enumerate()
        .map(|(index, block)| Candidate {
            start: ByteOffset::new((index * BLOCK) as u64),
            profile: classify(block),
        })
        .filter(|candidate| candidate.profile.class.can_hold_image_data())
        .collect()
}

fn broken_at(layout: &Fragmented) -> Broken {
    let mut src = layout.source();
    let mut scratch = Scratch::new();
    reassemble::locate_break(
        &mut src,
        ByteOffset::new(4 * BLOCK as u64),
        Format::Jpeg,
        layout.disk.len() as u64,
        &mut scratch,
    )
    .expect("in-memory read")
    .expect("the fixture must actually be fragmented")
}

/// `count` extents another recovery already claimed, none of them here.
///
/// Placed past the planted medium on purpose: they cannot match any candidate
/// block, so they change no answer. Every microsecond spent on them is a
/// microsecond spent establishing that.
///
/// This is the real arrangement rather than a contrived one — a scan reports
/// every artifact recovered from the whole surface, and a walk searching one
/// region is handed all of them.
fn claimed_elsewhere(count: u64) -> Vec<ByteRange> {
    let past = (BLOCKS * BLOCK) as u64;
    (0..count)
        .map(|index| {
            ByteRange::new(
                ByteOffset::new(past.saturating_add(index.saturating_mul(8192))),
                4096,
            )
        })
        .collect()
}

/// Runs one walk against `claimed`, returning what it recovered and its cost.
fn walk_with(claimed: &[ByteRange]) -> (usize, f64) {
    let layout = planted();
    let broken = broken_at(&layout);
    let candidates = candidate_blocks(&layout);
    let mut src = layout.source();
    let mut scratch = Scratch::new();

    let at = Instant::now();
    let walk = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &candidates,
        claimed,
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");
    (walk.assembled.len(), at.elapsed().as_secs_f64())
}

#[test]
fn extents_claimed_elsewhere_change_nothing_about_what_is_recovered() {
    // The property that has to hold before a cost assertion means anything.
    let (without, _) = walk_with(&[]);
    let (with, _) = walk_with(&claimed_elsewhere(CLAIMED));

    assert_eq!(without, 1, "the planted image must be reassembled");
    assert_eq!(
        with, without,
        "extents claimed elsewhere on the medium must not change the result"
    );
}

#[test]
fn the_walk_costs_what_the_medium_holds_not_what_was_recovered_from_it() {
    // The shape, not a number: a walk that consults the claimed set per block
    // gets slower as a previous stage recovers more, and a walk that does not
    // is unchanged. Measured broken, this ratio is about 98; fixed, about 1.
    //
    // Ratios rather than seconds, so this means the same in debug and release.
    let (_, empty) = walk_with(&[]);
    let (_, full) = walk_with(&claimed_elsewhere(CLAIMED));

    assert!(
        full < empty * 5.0 + 0.05,
        "the walk took {full:.3}s against {empty:.3}s with nothing claimed — it is scanning \
         all {CLAIMED} claimed extents for every candidate block, which on a real medium is \
         the product of 65,536 blocks and one extent per recovered artifact"
    );
}

#[test]
fn doubling_what_was_already_recovered_does_not_double_the_search() {
    // The same law from the other side, so a partial fix that merely shrinks
    // the constant still fails: growth against the claimed set has to be flat.
    let (_, small) = walk_with(&claimed_elsewhere(CLAIMED / 4));
    let (_, large) = walk_with(&claimed_elsewhere(CLAIMED));

    assert!(
        large < small * 2.0 + 0.05,
        "four times the claimed extents took {large:.3}s against {small:.3}s — the search \
         still grows with what was recovered rather than with what the medium holds"
    );
}
