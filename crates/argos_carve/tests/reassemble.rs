//! Fragment reassembly against known ground truth.
//!
//! Every test plants an image whose fragment layout the test itself chose, so
//! "recovered" means the extents Argos claims hold exactly the bytes that were
//! planted — not merely that something decoded.

use argos_carve::classify::{BlockClass, classify};
use argos_carve::fixture::{Disk, Fragmented, Jpeg, fragmented, photo_jpeg};
use argos_carve::reassemble::{self, Broken, Candidate, Limits};
use argos_carve::{Format, Scratch};
use argos_core::geometry::{ByteOffset, ByteRange};

/// Block size the fixtures fragment on: the reassembly search grid.
const BLOCK: usize = argos_carve::classify::BLOCK_BYTES;

/// Reads an extent out of a fixture disk.
fn extent_bytes(disk: &[u8], extent: argos_core::geometry::ByteRange) -> &[u8] {
    let start = usize::try_from(extent.start.get()).expect("fixture offsets fit usize");
    let len = usize::try_from(extent.len).expect("fixture lengths fit usize");
    &disk[start..start + len]
}

/// A real photograph big enough to span several blocks, so fragmenting it is
/// meaningful and so a full decode can judge the result.
fn jpeg_image() -> Vec<u8> {
    argos_carve::fixture::photo_jpeg(320, 240, 0x1234_5678_9ABC_DEF0)
}

/// Finds where the decoder gives up on a fragmented candidate.
fn broken_at(layout: &Fragmented, header: ByteOffset, format: Format) -> Broken {
    let mut src = layout.source();
    let mut scratch = Scratch::new();
    reassemble::locate_break(
        &mut src,
        header,
        format,
        layout.disk.len() as u64,
        &mut scratch,
    )
    .expect("in-memory read")
    .expect("the fixture must actually be fragmented, but it carved contiguously")
}

#[test]
fn a_two_fragment_image_is_reassembled_to_the_exact_planted_bytes() {
    let image = jpeg_image();
    // The second fragment sits well past the first, with unrelated filler in
    // the gap — the shape an allocator produces when it cannot find one run.
    let layout = fragmented(64 * BLOCK, &image, &[4 * BLOCK, 20 * BLOCK], BLOCK);
    let header = ByteOffset::new(4 * BLOCK as u64);
    let broken = broken_at(&layout, header, Format::Jpeg);

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read")
    .expect("a two-fragment image must be reassembled");

    assert_eq!(recovered.extents, layout.extents, "claimed extents");
    assert_eq!(recovered.length, image.len() as u64);
    // The claim is only worth anything if those extents really hold the image.
    let mut assembled = Vec::new();
    for extent in &recovered.extents {
        assembled.extend_from_slice(extent_bytes(&layout.disk, *extent));
    }
    assert_eq!(assembled, image, "the extents must hold the planted bytes");
}

#[test]
fn a_fragment_stored_before_its_predecessor_is_still_reassembled_in_file_order() {
    let image = jpeg_image();
    // The tail sits at a *lower* offset than the head: reassembly must follow
    // file order, not medium order.
    let layout = fragmented(64 * BLOCK, &image, &[40 * BLOCK, 8 * BLOCK], BLOCK);
    let header = ByteOffset::new(40 * BLOCK as u64);
    let broken = broken_at(&layout, header, Format::Jpeg);

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");

    // A backwards second fragment is outside bifragment's forward gap search.
    // What matters is that it invents nothing: the entropy decoder refuses
    // every forward splice, so the answer is "not found", never a fabrication.
    assert!(
        recovered.is_none(),
        "bifragment searches forward only and must not invent a backwards splice"
    );

    // The graph walk, which is not restricted to searching forward, does find
    // it — and lands on the planted bytes exactly.
    let candidates = candidate_blocks(&layout);
    let mut src = layout.source();
    let assembled = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &candidates,
        &[],
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");
    let [(_, walked)] = assembled.as_slice() else {
        panic!("the backwards layout must be reassembled by the walk, got {assembled:?}");
    };
    assert_eq!(walked.extents, layout.extents, "claimed extents");
}

#[test]
fn an_n_fragment_image_is_reassembled_by_the_graph_walk() {
    let image = jpeg_image();
    let layout = fragmented(
        96 * BLOCK,
        &image,
        &[4 * BLOCK, 20 * BLOCK, 44 * BLOCK],
        BLOCK,
    );
    let header = ByteOffset::new(4 * BLOCK as u64);
    let broken = broken_at(&layout, header, Format::Jpeg);

    let candidates = candidate_blocks(&layout);
    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let assembled = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &candidates,
        &[],
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");

    let [(_, recovered)] = assembled.as_slice() else {
        panic!("the three-fragment image must be reassembled, got {assembled:?}");
    };
    assert_eq!(recovered.extents, layout.extents, "claimed extents");
    assert_eq!(recovered.length, image.len() as u64);
}

/// Every block of the disk that classification says could hold image data.
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

#[test]
fn reassembly_never_claims_an_image_that_is_not_there() {
    // A header and a footer with nothing between them that completes the file.
    // The search must exhaust and report nothing rather than assemble bytes
    // that happen to decode.
    let image = jpeg_image();
    let truncated = argos_carve::fixture::truncated(&image, 2 * BLOCK);
    let disk = Disk::filled(64 * BLOCK)
        .with(4 * BLOCK, &truncated)
        .into_bytes();
    let layout = Fragmented {
        image: truncated.clone(),
        extents: Vec::new(),
        disk,
    };
    let header = ByteOffset::new(4 * BLOCK as u64);
    let broken = broken_at(&layout, header, Format::Jpeg);

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len() as u64,
        Limits::default(),
        &mut scratch,
    )
    .expect("in-memory read");

    assert!(
        recovered.is_none(),
        "with no second fragment on the medium there is nothing to reassemble"
    );
}

#[test]
fn the_hypothesis_budget_bounds_the_search() {
    let image = jpeg_image();
    let layout = fragmented(64 * BLOCK, &image, &[4 * BLOCK, 20 * BLOCK], BLOCK);
    let header = ByteOffset::new(4 * BLOCK as u64);
    let broken = broken_at(&layout, header, Format::Jpeg);

    let mut src = layout.source();
    let mut scratch = Scratch::new();
    let recovered = reassemble::bifragment(
        &mut src,
        broken,
        layout.disk.len() as u64,
        Limits {
            max_hypotheses: 1,
            ..Limits::default()
        },
        &mut scratch,
    )
    .expect("in-memory read");

    assert!(
        recovered.is_none(),
        "a budget of one attempt cannot reach a fragment twenty blocks away"
    );
}

#[test]
fn a_jpeg_entropy_stream_classifies_as_one_and_filler_does_not() {
    let image = Jpeg::new().with_entropy_bytes(8 * BLOCK).build();
    // A block from deep inside the entropy-coded scan.
    let scan_block = &image[3 * BLOCK..4 * BLOCK];
    assert_eq!(classify(scan_block).class, BlockClass::JpegStream);

    let filler = Disk::filled(BLOCK).into_bytes();
    assert_ne!(classify(&filler).class, BlockClass::JpegStream);

    let zeros = vec![0_u8; BLOCK];
    assert_eq!(classify(&zeros).class, BlockClass::LowEntropy);
}

#[test]
fn a_png_idat_stream_classifies_as_deflate() {
    // Photographic data does not compress, so a real IDAT payload is the
    // high-entropy deflate stream a classifier meets on a medium.
    let image = argos_carve::fixture::noisy_png(128, 128, 0xF00D_BEEF);
    let idat = argos_carve::fixture::idat_payload_offset(&image);
    let profile = classify(&image[idat..idat + BLOCK.min(image.len() - idat)]);

    assert_eq!(profile.class, BlockClass::Deflate);
    assert!(profile.class.can_hold_image_data());
}

#[test]
fn restart_markers_are_offered_as_independent_entry_points() {
    let image = Jpeg::new()
        .with_restart_interval(4)
        .with_entropy_bytes(4 * BLOCK)
        .build();
    let points = reassemble::restart_points(&image, ByteOffset::new(0));

    assert!(
        !points.is_empty(),
        "an image with restart markers must offer re-entry points"
    );
    for point in &points {
        assert!(point.get() <= image.len() as u64);
    }
}

#[test]
fn a_block_too_short_to_measure_is_not_claimed_as_image_data() {
    let profile = classify(&[0xFF, 0x00, 0xFF, 0x00]);
    assert!(!profile.class.can_hold_image_data());
    assert!(profile.score.abs() < f32::EPSILON);
}

#[test]
fn the_walk_never_reads_the_same_bytes_twice() {
    // A fragment is offered to the walk as one extent, but classification
    // offers *every* block inside it as a candidate. Excluding candidates by
    // start offset alone would leave every block after an extent's first one
    // available, letting a path splice a region onto itself and report a
    // layout no allocator could have produced.
    let image = photo_jpeg(640, 480, 0x0432_1001);
    let layout = fragmented(160 * BLOCK, &image, &[4 * BLOCK, 60 * BLOCK], BLOCK);
    let broken = broken_at(
        &layout,
        ByteOffset::new(layout.extents[0].start.get()),
        Format::Jpeg,
    );

    let candidates = candidate_blocks(&layout);
    let mut src = layout.source();
    let assembled = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &candidates,
        &[],
        layout.disk.len() as u64,
        Limits::default(),
        &mut Scratch::new(),
    )
    .expect("in-memory read");

    for (_, reassembly) in &assembled {
        for (index, extent) in reassembly.extents.iter().enumerate() {
            for other in &reassembly.extents[index + 1..] {
                let (a_start, a_end) = (extent.start.get(), extent.end_saturating().get());
                let (b_start, b_end) = (other.start.get(), other.end_saturating().get());
                assert!(
                    a_end <= b_start || b_end <= a_start,
                    "extents {a_start}..{a_end} and {b_start}..{b_end} overlap: the same \
                     bytes were reported twice in one recovered file"
                );
            }
        }
    }
}

#[test]
fn bytes_another_recovery_already_claimed_are_not_offered_to_the_walk() {
    // The engine runs the gap search first and hands its answers here. Left
    // out, the walk would be free to build a second artifact over the same
    // bytes, and the merge step cannot collapse the pair — their content
    // hashes differ, so both would reach the manifest as separate files.
    let image = photo_jpeg(640, 480, 0x0432_1002);
    let layout = fragmented(160 * BLOCK, &image, &[4 * BLOCK, 60 * BLOCK], BLOCK);
    let header = ByteOffset::new(layout.extents[0].start.get());
    let broken = broken_at(&layout, header, Format::Jpeg);

    // Claim the whole region the true remainder lives in.
    let claimed = [ByteRange::new(
        ByteOffset::new(60 * BLOCK as u64),
        40 * BLOCK as u64,
    )];
    let candidates = candidate_blocks(&layout);
    let mut src = layout.source();
    let assembled = reassemble::parallel_unique_path(
        &mut src,
        &[broken],
        &candidates,
        &claimed,
        layout.disk.len() as u64,
        Limits::default(),
        &mut Scratch::new(),
    )
    .expect("in-memory read");

    for (_, reassembly) in &assembled {
        for extent in &reassembly.extents {
            let start = extent.start.get();
            assert!(
                !(claimed[0].start.get() <= start && start < claimed[0].end_saturating().get()),
                "the walk claimed bytes at {start}, which another recovery already holds"
            );
        }
    }
}
