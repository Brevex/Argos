//! Recovering pixels from entropy-coded fragments no header reaches.
//!
//! A fragment of a JPEG whose header is gone decodes against nothing: the
//! Huffman tables, the sampling factors and the frame geometry all live in the
//! header. The published technique estimates them from a corpus of camera
//! settings wide enough to cover the one that wrote the file (Uzun & Sencar,
//! *Carving Orphaned JPEG File Fragments*, IEEE TIFS 10(8), 2015). When a file
//! from the same batch survives, they are known instead of estimated, and that
//! file's header is lent to the fragment.
//!
//! This is deliberately **not** a stage of the scan pipeline. What it produces
//! is pixels in a container this tool built — the frame size is the
//! reference's, each strip's position inside it is unknown, and those bytes in
//! that order never lay on the medium. A scan's artifacts are files the medium
//! held; these are not, and they carry [`Confidence::Grafted`] to say so
//! (`A-CONFIDENCE-HONEST`).

use std::io::{Read, Seek, SeekFrom};

use argos_carve::classify::{self, BlockClass};
use argos_carve::mcu;
use argos_carve::reassemble::restart_points;
use argos_carve::reference::Reference;

// `sweep` takes a reference, so the type is part of this crate's surface and a
// caller must be able to name it without depending on `argos_carve` — which
// the binary deliberately does not (`M-FOREIGN-REEXPORTS`).
pub use argos_carve::reference::{Fault, Reference as JpegReference, ReferenceError};
use argos_core::{ByteOffset, Confidence};

/// Bytes read from the medium at a time.
const READ_BYTES: usize = 4 * 1024 * 1024;

/// The last of the eight restart markers, `RST7`. Source: ITU-T T.81 Table B.1.
///
/// The eight are cyclic, so this is the one a graft enters after: the marker
/// following it is `RST0`, which is what a decoder leaving `SOS` expects.
const MARKER_RST7: u8 = 0xD7;

/// Most consecutive image-bearing blocks treated as one orphan.
///
/// A longer run is not one strip of one photograph but a stretch of medium
/// dense in entropy data, and grafting all of it draws stitched-together
/// nonsense rather than a picture. Bounded independently of anything read from
/// the medium (`A-BOUNDED-ALLOC`).
const MAX_RUN_BLOCKS: usize = 512;

/// Smallest run worth entering, in blocks.
///
/// Below this there is not enough entropy data to draw rows a person could
/// recognise, and every one costs a decode.
const MIN_RUN_BLOCKS: usize = 4;

/// Fewest MCUs a graft must decode before it is reported.
///
/// A graft is a fragment from the middle of a scan, so it can never fill the
/// reference's frame and its tail is always cut mid-MCU: the strict pixel
/// decoder rejects it by design, because for a whole file "ends where its frame
/// says" is what turns *decoded* into evidence. The entropy decoder is the
/// oracle instead — the one the reassembly search already answers to — and what
/// it measures is how many MCUs genuinely decoded before the stream stopped
/// being this picture. Sixty-four is eight rows of an eight-MCU-wide frame:
/// enough picture to recognise, and far more than a run of plausible bytes
/// decodes by accident (`M-DOCUMENTED-MAGIC`).
const MIN_MCUS: u32 = 64;

/// Pixels recovered from a fragment that had no header of its own.
#[derive(Clone, Debug)]
pub struct Grafted {
    /// Where the entropy data was entered, on the medium. Real, and the only
    /// part of this artifact that locates anything.
    pub at: ByteOffset,
    /// Medium bytes the graft consumed, from `at`.
    pub len: u64,
    /// Reference header, medium bytes, `EOI`.
    pub bytes: Vec<u8>,
    /// Frame the reference declares, which is not this strip's own size.
    pub dimensions: (u32, u32),
    /// MCUs the entropy decoder accounted for before the stream stopped.
    ///
    /// How much picture this carries. The frame is the reference's, so this is
    /// the only number that describes the strip itself.
    pub mcus: u32,
    /// Always [`Confidence::Grafted`]: the floor, because the container is
    /// this tool's and only the pixels are the medium's.
    pub confidence: Confidence,
}

/// Sweeps `range` of `view`, grafting every orphaned run onto `reference`.
///
/// `entered` counts runs a restart marker let it try; the returned artifacts
/// are the ones that decoded to a picture. A run the medium refuses is skipped
/// and nothing is fabricated for it.
pub fn sweep<V: Read + Seek>(
    view: &mut V,
    range: std::ops::Range<u64>,
    reference: &Reference,
    mut found: impl FnMut(Grafted),
) -> usize {
    let mut entered = 0_usize;
    let mut buf = vec![0_u8; READ_BYTES];
    let mut at = range.start;

    while at < range.end {
        let take = READ_BYTES.min(usize::try_from(range.end - at).unwrap_or(READ_BYTES));
        let window = &mut buf[..take];
        if view.seek(SeekFrom::Start(at)).is_err() || view.read_exact(window).is_err() {
            at = at.saturating_add(take as u64);
            continue;
        }
        for run in orphan_runs(window) {
            entered += 1;
            if let Some(grafted) = graft_run(reference, window, at, &run) {
                found(grafted);
            }
        }
        at = at.saturating_add(take as u64);
    }
    entered
}

/// Ranges inside `window` that look like JPEG entropy data.
///
/// Only `JpegStream` counts. The other classes a block can carry — deflate,
/// undifferentiated high entropy — are not JPEG scan data, and entering one
/// draws a frame of noise the decoder happens to accept.
fn orphan_runs(window: &[u8]) -> Vec<std::ops::Range<usize>> {
    let block = classify::BLOCK_BYTES;
    let mut runs = Vec::new();
    let mut current: Option<std::ops::Range<usize>> = None;
    let long_enough =
        |run: &std::ops::Range<usize>| (run.end - run.start) / block >= MIN_RUN_BLOCKS;

    for (index, chunk) in window.chunks_exact(block).enumerate() {
        let is_scan = classify::classify(chunk).class == BlockClass::JpegStream;
        let start = index * block;
        let full = current
            .as_ref()
            .is_some_and(|run| (run.end - run.start) / block >= MAX_RUN_BLOCKS);
        if is_scan
            && !full
            && let Some(run) = current.as_mut()
        {
            run.end = start + block;
            continue;
        }
        if let Some(run) = current.take()
            && long_enough(&run)
        {
            runs.push(run);
        }
        if is_scan {
            current = Some(start..start + block);
        }
    }
    if let Some(run) = current
        && long_enough(&run)
    {
        runs.push(run);
    }
    runs
}

/// Enters `run` at its first restart marker and grafts it onto `reference`.
///
/// `None` when the run holds no restart marker — there is then no offset at
/// which a decoder's predictors are known, and none can be invented for it —
/// or when the graft decodes to nothing a person would call a picture.
fn graft_run(
    reference: &Reference,
    window: &[u8],
    window_at: u64,
    run: &std::ops::Range<usize>,
) -> Option<Grafted> {
    let bytes = window.get(run.clone())?;
    // Not any restart marker: the one after which the cadence lines up. A
    // decoder leaving `SOS` counts one interval and then expects `RST0`, so
    // entering after `RST5` presents it `RST6` and the stream breaks at the
    // first marker it meets. Entering after `RST7` is the only offset whose
    // next marker is the one the decoder is waiting for.
    let entry = restart_points(bytes, ByteOffset::new(0))
        .into_iter()
        .filter_map(|point| usize::try_from(point.get()).ok())
        .find(|&point| bytes.get(point.wrapping_sub(1)) == Some(&MARKER_RST7))?;
    let entropy = bytes.get(entry..)?;

    let grafted = reference.graft(entropy);
    let mut scratch = argos_carve::Scratch::new();
    let outcome = mcu::scan(
        &mut std::io::Cursor::new(&grafted),
        ByteOffset::new(0),
        grafted.len() as u64,
        &mut scratch,
    )
    .ok()?;
    if outcome.mcus_decoded < MIN_MCUS {
        return None;
    }
    Some(Grafted {
        at: ByteOffset::new(
            window_at
                .saturating_add(run.start as u64)
                .saturating_add(entry as u64),
        ),
        len: entropy.len() as u64,
        dimensions: (outcome.width, outcome.height),
        mcus: outcome.mcus_decoded,
        bytes: grafted,
        confidence: Confidence::Grafted,
    })
}
