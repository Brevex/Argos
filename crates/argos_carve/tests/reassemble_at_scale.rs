//! Reassembly measured on a medium bigger than its own search.
//!
//! The suite in `reassemble.rs` plants fragments a few hundred kilobytes apart,
//! where every candidate block is a neighbour of every header and every gap is
//! within reach whatever the budget. A real medium is neither, and a search
//! that is bounded or ordered wrongly still passes there while recovering
//! nothing in the field.
//!
//! These fixtures are hundreds of megabytes, generated per byte read rather
//! than held ([`SparseDisk`]), so the distances are the ones a disk produces.

use argos_carve::classify::classify;
use argos_carve::fixture::{Planted, SparseDisk, photo_jpeg, planted};
use argos_carve::reassemble::{self, Broken, Candidate, Limits};
use argos_carve::{Format, Scratch};
use argos_core::geometry::ByteOffset;

/// Fragment grid the fixtures and the search share.
const BLOCK: u64 = argos_carve::classify::BLOCK_BYTES as u64;

const MIB: u64 = 1024 * 1024;

/// A real photograph, big enough that fragmenting it is meaningful and that a
/// full decode can judge the result.
fn image() -> Vec<u8> {
    photo_jpeg(320, 240, 0x1234_5678_9ABC_DEF0)
}

/// Finds where the decoder gives up on a planted candidate.
fn broken_at(layout: &Planted, header: ByteOffset) -> Broken {
    let mut src = layout.source();
    let mut scratch = Scratch::new();
    reassemble::locate_break(
        &mut src,
        header,
        Format::Jpeg,
        layout.disk.len(),
        &mut scratch,
    )
    .expect("in-memory read")
    .expect("the fixture must actually be fragmented, but it carved contiguously")
}

/// The profile a block of this medium's filler classifies as.
///
/// Decoy candidates carry it so they are offered to the walk exactly as the
/// filler blocks of a real medium would be.
fn filler_profile(disk: &SparseDisk, at: u64) -> argos_carve::classify::BlockProfile {
    let block = argos_core::geometry::ByteRange::new(ByteOffset::new(at), BLOCK);
    classify(&disk.read_extents(&[block]))
}

#[test]
fn a_second_fragment_past_the_first_sixteen_megabytes_is_still_found() {
    let image = image();
    // 24 MiB of gap: inside MAX_GAP_BYTES, and past what 4096 hypotheses of
    // 4 KiB can step through.
    let header = 8 * MIB;
    let layout = planted(64 * MIB, &image, &[header, header + 24 * MIB], BLOCK);
    let broken = broken_at(&layout, ByteOffset::new(header));

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len(),
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read")
    .reassembly
    .expect("a gap inside MAX_GAP_BYTES must be searched to the end");

    assert_eq!(recovered.extents, layout.extents, "claimed extents");
    assert_eq!(
        layout.disk.read_extents(&recovered.extents),
        image,
        "the extents must hold the planted bytes"
    );
}

#[test]
fn a_fragment_stored_before_its_header_is_found_by_the_gap_search() {
    let image = image();
    // The tail sits at a lower offset than the head, which is what an
    // allocator does when it fills a hole behind the write position.
    let header = 40 * MIB;
    let layout = planted(64 * MIB, &image, &[header, 8 * MIB], BLOCK);
    let broken = broken_at(&layout, ByteOffset::new(header));

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len(),
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read")
    .reassembly
    .expect("the gap search must look behind the header as well as ahead");

    assert_eq!(recovered.extents, layout.extents, "claimed extents");
    assert_eq!(
        layout.disk.read_extents(&recovered.extents),
        image,
        "the extents must hold the planted bytes"
    );
}

#[test]
fn the_graph_walk_reaches_a_continuation_its_candidate_list_ranks_last() {
    let image = image();
    // The header is near the end of the medium and its continuation is just
    // past it, which is where an allocator puts one. Every other candidate on
    // the medium sits at a lower offset.
    let header = 200 * MIB;
    let tail = header + 8 * MIB;
    let layout = planted(256 * MIB, &image, &[header, tail], BLOCK);
    let broken = broken_at(&layout, ByteOffset::new(header));

    let profile = filler_profile(&layout.disk, 0);
    // Filler blocks from the start of the medium, as classification would
    // offer them: far more than one search's hypotheses, and all ahead of the
    // real continuation in medium order.
    let mut blocks: Vec<Candidate> = (0..20_000)
        .map(|index| Candidate {
            start: ByteOffset::new(index * BLOCK),
            profile,
        })
        .collect();
    blocks.push(Candidate {
        start: ByteOffset::new(tail),
        profile,
    });

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let assembled = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &blocks,
        &[],
        layout.disk.len(),
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");

    let (_, recovered) =
        assembled.assembled.into_iter().next().expect(
            "the walk must try the blocks nearest the header, not the lowest-numbered ones",
        );
    assert_eq!(recovered.extents, layout.extents, "claimed extents");
    assert_eq!(
        layout.disk.read_extents(&recovered.extents),
        image,
        "the extents must hold the planted bytes"
    );
}

#[test]
fn a_header_whose_continuation_is_not_on_the_medium_reassembles_to_nothing() {
    // The fabrication guard at scale. The first fragment is planted and its
    // remainder is not, so every hypothesis a widened search tests is wrong.
    // Reporting any of them would be inventing a file.
    let image = image();
    let header = 8 * MIB;
    let whole = planted(64 * MIB, &image, &[header], BLOCK);
    let first_fragment = &image[..image.len() / 2];
    let layout = Planted {
        image: image.clone(),
        extents: whole.extents.clone(),
        disk: SparseDisk::noisy(64 * MIB, 0x5DEE_CE66_D000_0001).with(header, first_fragment),
    };
    let broken = broken_at(&layout, ByteOffset::new(header));

    let profile = filler_profile(&layout.disk, 0);
    let blocks: Vec<Candidate> = (0..4_000)
        .map(|index| Candidate {
            start: ByteOffset::new(header + 4 * MIB + index * BLOCK),
            profile,
        })
        .collect();

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let gap = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len(),
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");
    assert!(
        gap.reassembly.is_none(),
        "the gap search reported an image whose bytes were never planted"
    );

    let walked = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &blocks,
        &[],
        layout.disk.len(),
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");
    assert!(
        walked.assembled.is_empty(),
        "the graph walk reported an image whose bytes were never planted"
    );
}
