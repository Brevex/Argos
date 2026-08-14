//! Measured recovery rates per fragmentation pattern — the phase's exit
//! criterion.
//!
//! Two different things are checked here, and only one of them is a threshold.
//!
//! **Zero false positives is absolute.** Every recovery this suite reports must
//! be byte-identical to the image that was planted. A tool that recovers more
//! by occasionally assembling bytes that were never a file is worse than one
//! that recovers less, because an examiner cannot tell the two results apart.
//!
//! **The recovery rate is measured, not assumed.** It is printed per pattern
//! and held to a documented floor, so a regression in the search shows up as a
//! failure rather than as quietly worse output. The floors are also what fixes
//! how wide the search may go: widening it past three fragments raised the
//! rates here and fabricated an answer, which is how the bound on it was
//! chosen.

use std::fmt::Write as _;

use argos_carve::fixture::{Fragmented, fragmented, photo_jpeg, photo_jpeg_rgb};
use argos_carve::reassemble::{self, Candidate, Limits};
use argos_carve::{Format, Scratch};
use argos_core::geometry::ByteOffset;

/// Fragment grid the fixtures and the search share.
const BLOCK: usize = argos_carve::classify::BLOCK_BYTES;

/// Distinct seeded images per pattern. Enough that a rate means something
/// without making the suite slow.
const SAMPLES: usize = 8;

/// One layout to attempt, and the ground truth to check any answer against.
struct Case {
    layout: Fragmented,
}

impl Case {
    /// Runs both reassembly techniques the way the engine does — gap search
    /// first, graph walk for what it cannot complete.
    ///
    /// Returns `Some` only when something was reported, whether right or
    /// wrong; the caller decides which it was.
    fn attempt(&self) -> Option<Vec<u8>> {
        let mut src = self.layout.source();
        let mut scratch = Scratch::new();
        let medium_len = self.layout.disk.len() as u64;
        let header = ByteOffset::new(self.layout.extents[0].start.get());

        let broken =
            reassemble::locate_break(&mut src, header, Format::Jpeg, medium_len, &mut scratch)
                .expect("in-memory read")?;

        let recovered = reassemble::bifragment(
            &mut src,
            broken,
            medium_len,
            Limits::default(),
            &mut scratch,
        )
        .expect("in-memory read")
        .reassembly
        .or_else(|| {
            let blocks = self.candidate_blocks();
            reassemble::parallel_unique_path(
                &mut src,
                &[broken],
                &blocks,
                &[],
                medium_len,
                Limits::default(),
                &mut scratch,
            )
            .expect("in-memory read")
            .assembled
            .into_iter()
            .next()
            .map(|(_, reassembly)| reassembly)
        })?;

        let mut bytes = Vec::with_capacity(usize::try_from(recovered.length).unwrap_or(0));
        for extent in &recovered.extents {
            let start = usize::try_from(extent.start.get()).expect("fixture offsets fit usize");
            let len = usize::try_from(extent.len).expect("fixture lengths fit usize");
            bytes.extend_from_slice(&self.layout.disk[start..start + len]);
        }
        Some(bytes)
    }

    fn candidate_blocks(&self) -> Vec<Candidate> {
        self.layout
            .disk
            .chunks(BLOCK)
            .enumerate()
            .map(|(index, block)| Candidate {
                start: ByteOffset::new((index * BLOCK) as u64),
                profile: argos_carve::classify::classify(block),
            })
            .filter(|candidate| candidate.profile.class.can_hold_image_data())
            .collect()
    }
}

/// What one pattern's samples came to.
struct Rate {
    pattern: &'static str,
    recovered: usize,
    fabricated: usize,
    total: usize,
}

impl Rate {
    fn percent(&self) -> usize {
        if self.total == 0 {
            return 0;
        }
        self.recovered * 100 / self.total
    }
}

/// Fills a layout's unused space with a *different* photograph from the same
/// encoder.
///
/// This is the condition that defeats an entropy-only oracle, and it is the
/// modal real medium: one camera, one resolution, one quality setting, so
/// every file on the disk shares Huffman tables. A splice from one scan into
/// another therefore decodes cleanly and reaches the right MCU count. Only the
/// picture at the splice separates them, so a suite that plants one image in
/// noise is not testing the guarantee it asserts.
fn with_competing_photo(mut layout: Fragmented, seed: u64) -> Fragmented {
    let decoy = photo_jpeg(640, 480, seed ^ 0xFFFF_FFFF_FFFF_FFFF);
    // Drop it into the gap between the planted fragments, block-aligned like a
    // neighbouring file would be.
    let gap = 24 * BLOCK;
    if gap + decoy.len() <= layout.disk.len() {
        let clashes = layout.extents.iter().any(|extent| {
            let start = usize::try_from(extent.start.get()).unwrap_or(0);
            let len = usize::try_from(extent.len).unwrap_or(0);
            start < gap + decoy.len() && gap < start + len
        });
        if !clashes {
            layout.disk[gap..gap + decoy.len()].copy_from_slice(&decoy);
        }
    }
    layout
}

/// Runs `SAMPLES` seeded layouts of one shape and scores them.
fn measure(pattern: &'static str, layout_of: impl Fn(&[u8], usize) -> Fragmented) -> Rate {
    let mut recovered = 0;
    let mut fabricated = 0;
    for sample in 0..SAMPLES {
        // A different image per sample, so a rate is not one image's luck.
        let seed = 0x51ED_2A11_0000_0001_u64.wrapping_mul(sample as u64 + 1);
        // Large enough that the deepest pattern still gives every fragment a
        // whole block, and alternating colour models so a rate is not one
        // encoder path's luck.
        let image = if sample % 2 == 0 {
            photo_jpeg(640, 480, seed)
        } else {
            photo_jpeg_rgb(512, 384, seed)
        };
        let case = Case {
            layout: layout_of(&image, sample),
        };
        match case.attempt() {
            None => {}
            Some(bytes) if bytes == case.layout.planted_bytes() => recovered += 1,
            // Reported something that is not what was planted. This is the
            // outcome the whole design exists to make impossible.
            Some(bytes) => {
                let planted = case.layout.planted_bytes();
                let common = bytes
                    .iter()
                    .zip(&planted)
                    .take_while(|(got, want)| got == want)
                    .count();
                println!(
                    "  FABRICATED {pattern} sample {sample}: got {} bytes, planted {}, \
                     agree for {common}",
                    bytes.len(),
                    planted.len()
                );
                fabricated += 1;
            }
        }
    }
    Rate {
        pattern,
        recovered,
        fabricated,
        total: SAMPLES,
    }
}

#[test]
fn recovery_rates_per_fragmentation_pattern() {
    let rates = vec![
        // Two fragments, the remainder landing just past the first — what an
        // allocator does when a short run was free nearby.
        measure("2 fragments, small gap", |image, sample| {
            fragmented(
                96 * BLOCK,
                image,
                &[4 * BLOCK, (10 + sample) * BLOCK],
                BLOCK,
            )
        }),
        // Two fragments far apart, the remainder in a different region.
        measure("2 fragments, large gap", |image, sample| {
            fragmented(
                160 * BLOCK,
                image,
                &[4 * BLOCK, (60 + sample * 4) * BLOCK],
                BLOCK,
            )
        }),
        // Three fragments, forward order.
        measure("3 fragments", |image, sample| {
            fragmented(
                160 * BLOCK,
                image,
                &[4 * BLOCK, (20 + sample) * BLOCK, (44 + sample * 2) * BLOCK],
                BLOCK,
            )
        }),
        // Four fragments, forward order.
        measure("4 fragments", |image, sample| {
            fragmented(
                200 * BLOCK,
                image,
                &[
                    4 * BLOCK,
                    (16 + sample) * BLOCK,
                    (40 + sample) * BLOCK,
                    (70 + sample) * BLOCK,
                ],
                BLOCK,
            )
        }),
        // The remainder stored *before* the header, which a forward gap search
        // cannot reach and only the graph walk can.
        measure("2 fragments, stored backwards", |image, sample| {
            fragmented(
                96 * BLOCK,
                image,
                &[(60 + sample) * BLOCK, 8 * BLOCK],
                BLOCK,
            )
        }),
        // The hard case: another photograph from the same encoder sits between
        // the fragments, sharing Huffman tables, so a wrong splice decodes.
        measure("2 fragments, competing photo", |image, sample| {
            let seed = 0x51ED_2A11_0000_0001_u64.wrapping_mul(sample as u64 + 1);
            with_competing_photo(
                fragmented(
                    160 * BLOCK,
                    image,
                    &[4 * BLOCK, (60 + sample * 4) * BLOCK],
                    BLOCK,
                ),
                seed,
            )
        }),
    ];

    let mut table = String::from("\nrecovery rate by fragmentation pattern\n");
    for rate in &rates {
        let _ = writeln!(
            table,
            "  {:<30} {:>3}%   ({}/{} recovered, {} fabricated)",
            rate.pattern,
            rate.percent(),
            rate.recovered,
            rate.total,
            rate.fabricated
        );
    }
    println!("{table}");

    // The absolute requirement: nothing reported that was not planted.
    for rate in &rates {
        assert_eq!(
            rate.fabricated, 0,
            "{} fabricated {} of {} answers — a reassembly that is not the \
             planted bytes is worse than no reassembly at all\n{table}",
            rate.pattern, rate.fabricated, rate.total
        );
    }

    // The measured floors, recorded so a regression in the search fails here
    // rather than quietly shipping worse output.
    //
    // Two and three fragments are covered, forwards or backwards: the walk
    // reconsiders its steps up to `MAX_BRANCHING_FRAGMENTS`, so a step whose
    // best candidate is not the true continuation no longer loses the path.
    //
    // Past that depth it commits and does not look back, and the reason is the
    // oracle rather than the budget. Branching at four fragments recovered more
    // and produced an assembly of the right length, decoding end to end, whose
    // three seams all passed, and which was not the planted bytes. No seam
    // threshold separated that case without refusing a third of the true
    // recoveries with it, so the search stops where it can still tell the
    // difference.
    let floor_for = |pattern: &str| match pattern {
        // The seam gate refuses an assembly it cannot judge as readily as one
        // it judges badly, so a little recall is traded for the zero above.
        // That is the right direction for a tool whose wrong answers are worse
        // than its missing ones.
        "2 fragments, small gap"
        | "2 fragments, large gap"
        | "2 fragments, stored backwards"
        | "2 fragments, competing photo"
        | "3 fragments" => 85,
        // Reached only by the greedy tail of the walk, so it is the pattern
        // this suite covers least — held at what it measures so a regression
        // shows, not at zero.
        _ => 20,
    };
    for rate in &rates {
        assert!(
            rate.percent() >= floor_for(rate.pattern),
            "{} recovered {}%, below the {}% this suite records\n{table}",
            rate.pattern,
            rate.percent(),
            floor_for(rate.pattern)
        );
    }
}
