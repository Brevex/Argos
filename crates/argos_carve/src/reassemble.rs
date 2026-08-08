//! Fragment reassembly: recovering images whose bytes are not contiguous.
//!
//! Contiguous carving recovers the easy 90%. The rest are fragmented — split
//! by a filesystem that had no run long enough — and for those a signature and
//! a footer are not enough: the extents in between have to be *found*.
//!
//! Both techniques here work the same way. Propose an extent list, hand it to
//! the real format state machine through [`Assembled`], and keep only what
//! decodes end to end. The decoder is the oracle; nothing is accepted because
//! it looked plausible.
//!
//! - [`bifragment`] covers the dominant real-world pattern — two fragments
//!   with a gap between them (Garfinkel, "Carving contiguous and fragmented
//!   files with fast object validation", DFRWS 2007).
//! - [`parallel_unique_path`] covers more than two, growing the best path per
//!   header greedily with each extent assignable to one path (Pal, Sencar &
//!   Memon, DFRWS 2008; Pal & Memon, IEEE SPM 2009).
//!
//! Everything produced here is [`Confidence::Reassembled`] — below a
//! contiguous carve, because a reconstruction is a claim about which bytes
//! belong together, and that claim rests on the decoder accepting them rather
//! than on the medium having stored them side by side. The exact extent list
//! is recorded so the result is reproducible (A-PROVENANCE).
//!
//! [`Confidence::Reassembled`]: argos_core::Confidence

use std::io::{Read, Seek};

use argos_core::geometry::{ByteOffset, ByteRange};

use crate::assemble::{Assembled, total_len};
use crate::classify::{self, BlockProfile};
use crate::decode;
use crate::{CarveError, Format, Scratch, Verdict};

/// Granularity of a fragment boundary.
///
/// Filesystems allocate in clusters, so a file's fragments start and end on
/// multiples of the cluster size. 4 KiB is the smallest cluster any filesystem
/// Argos handles uses, which makes it the safe search step: a coarser one
/// would step over real boundaries.
pub const BLOCK_BYTES: u64 = classify::BLOCK_BYTES as u64;

/// Largest gap [`bifragment`] will search between two fragments.
///
/// Fragmentation happens because an allocator could not find one long enough
/// run; the second fragment usually lands within the same allocation region,
/// so gaps are small. Searching further multiplies the hypothesis count
/// without finding much, and every hypothesis is a full decode.
pub const MAX_GAP_BYTES: u64 = 64 * 1024 * 1024;

/// Largest number of fragments [`parallel_unique_path`] will assemble.
///
/// Real fragmented images run to a handful of pieces. The bound is what stops
/// a crafted medium from driving an unbounded search (A-BOUNDED-ALLOC).
pub const MAX_FRAGMENTS: usize = 16;

/// Largest number of decode attempts one reassembly may make.
///
/// Every hypothesis costs a full validation pass, so this is what bounds the
/// time a single fragmented candidate can take, independent of anything read
/// from the medium.
pub const MAX_HYPOTHESES: u32 = 4096;

/// How far the stitch row may stand out before an assembly is refused.
///
/// The entropy decoder settles whether bytes decode; it cannot settle whether
/// they belong together, because two photographs from one camera share Huffman
/// tables and a splice between them decodes cleanly. What separates them is the
/// picture at the splice: a real image's rows change gradually, so the stitch
/// row is unremarkable against the frame's median row difference, while a
/// spliced one shows a hard edge there.
///
/// Three times the median is well outside what a photograph's own content
/// produces and well inside what a splice does.
pub const MAX_SEAM_RATIO: f32 = 3.0;

/// Candidate first-fragment ends tried per header.
///
/// The decoder's break point is an upper bound on where the first fragment
/// ends; the true boundary is the block boundary at or below it. Trying the
/// nearest few covers the case where the next fragment's bytes happened to
/// keep parsing for a while past the real splice.
pub const MAX_PREFIX_CANDIDATES: usize = 8;

/// How hard a reassembly is allowed to try.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest gap between two fragments.
    pub max_gap_bytes: u64,
    /// Largest number of fragments in one assembly.
    pub max_fragments: usize,
    /// Largest number of decode attempts.
    pub max_hypotheses: u32,
    /// Fragment boundary granularity.
    pub block_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_gap_bytes: MAX_GAP_BYTES,
            max_fragments: MAX_FRAGMENTS,
            max_hypotheses: MAX_HYPOTHESES,
            block_bytes: BLOCK_BYTES,
        }
    }
}

impl Limits {
    /// The block boundary at or below `at`.
    fn floor(self, at: u64) -> u64 {
        let block = self.block_bytes.max(1);
        at - (at % block)
    }

    /// The block boundary at or above `at`.
    fn ceil(self, at: u64) -> u64 {
        let block = self.block_bytes.max(1);
        match at % block {
            0 => at,
            remainder => at.saturating_add(block - remainder),
        }
    }
}

/// A candidate image that broke mid-stream, with what the decoder learned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Broken {
    /// Where the image starts on the medium.
    pub header: ByteOffset,
    /// Earliest offset at which the stream stopped being this file — the
    /// fragmentation point the decoder localized.
    pub break_at: ByteOffset,
    /// Format the header claimed.
    pub format: Format,
}

/// A reassembled image: which bytes, in which order, and how hard it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reassembly {
    /// Source extents in file order. Concatenating them yields the image.
    pub extents: Vec<ByteRange>,
    /// Image length in bytes, as the decoder confirmed it.
    pub length: u64,
    /// Decode attempts spent reaching this result, so a report can say what
    /// the search cost.
    pub hypotheses: u32,
}

impl Reassembly {
    /// Number of fragments the image was assembled from.
    #[must_use]
    pub fn fragments(&self) -> usize {
        self.extents.len()
    }
}

/// Locates the fragmentation point of a candidate that failed to carve.
///
/// The break point has to come from the decoder that actually reads the image
/// data, not from the marker grammar. Handed unknown bytes, the grammar reads
/// a segment length out of them and skips forward by up to 65533 — repeatedly
/// — so its idea of where the file stopped can sit tens of kilobytes past the
/// real splice, and every search that starts from it starts in the wrong
/// place. The entropy decoder stops on the byte where the data stopped being
/// this image ([`crate::mcu`]).
///
/// `None` when the candidate is whole, when nothing decoded at all, or when
/// the frame uses a coding this oracle cannot check — in each case there is no
/// fragmentation point to hand on.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn locate_break<R: Read + Seek>(
    src: &mut R,
    header: ByteOffset,
    format: Format,
    medium_len: u64,
    scratch: &mut Scratch,
) -> Result<Option<Broken>, CarveError> {
    let limit = medium_len.min(header.get().saturating_add(crate::MAX_IMAGE_BYTES));
    let break_at = match format {
        Format::Jpeg => {
            let outcome = crate::mcu::scan(src, header, limit, scratch)?;
            if outcome.is_complete()
                || outcome.stop == crate::mcu::ScanStop::Unsupported
                || outcome.mcus_decoded == 0
            {
                return Ok(None);
            }
            outcome.end
        }
        Format::Png => {
            // A PNG's per-chunk CRC32 stops the structural walk exactly where
            // the data stops being the file, so it needs no other oracle.
            match crate::validate(format, src, header, limit, scratch)? {
                Verdict::Complete { .. } => return Ok(None),
                Verdict::Corrupt { at, .. } => at,
            }
        }
    };
    if break_at.get() <= header.get() {
        return Ok(None);
    }
    Ok(Some(Broken {
        header,
        break_at,
        format,
    }))
}

/// Searches for a second fragment that completes `broken`.
///
/// The decoder stopped at [`Broken::break_at`], so the first fragment ends at
/// a block boundary at or below it, and the second starts at a later block
/// boundary. Hypotheses are tried smallest gap first, because an allocator
/// that split a file usually put the remainder nearby.
///
/// The second fragment is offered as running to the end of the searchable
/// region rather than to a located footer: the decoder is what knows where the
/// image ends, and the accepted extents are trimmed to the length it reports.
/// Enumerating footers instead would make the result depend on which of the
/// many false `FF D9` hits on a medium happened to be tried first.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails. A hypothesis that does not
/// decode is not an error — it is the expected outcome for most of them.
pub fn bifragment<R: Read + Seek>(
    src: &mut R,
    broken: Broken,
    medium_len: u64,
    limits: Limits,
    scratch: &mut Scratch,
) -> Result<Option<Reassembly>, CarveError> {
    let header = broken.header.get();
    let break_at = broken.break_at.get();
    if break_at <= header || break_at >= medium_len {
        return Ok(None);
    }

    let mut attempts = 0_u32;
    // Ends of the first fragment: the block boundary at or below the break,
    // then earlier ones, since a wrong splice can parse on for a while.
    for first_end in prefix_candidates(header, break_at, limits) {
        // Smallest gap first (Garfinkel's ordering): the second fragment
        // starts as soon after the first as the block grid allows.
        let mut second_start = limits.ceil(first_end.saturating_add(1));
        let furthest = second_start
            .saturating_add(limits.max_gap_bytes)
            .min(medium_len);
        while second_start < furthest {
            if attempts >= limits.max_hypotheses {
                return Ok(None);
            }
            attempts += 1;

            let extents = [
                ByteRange::new(ByteOffset::new(header), first_end - header),
                ByteRange::new(
                    ByteOffset::new(second_start),
                    tail_len(second_start, medium_len),
                ),
            ];
            if let Some(accepted) = decodes(src, broken.format, &extents, scratch)? {
                return Ok(Some(Reassembly {
                    extents: trim_to(&extents, accepted.length),
                    length: accepted.length,
                    hypotheses: attempts,
                }));
            }
            second_start = second_start.saturating_add(limits.block_bytes.max(1));
        }
    }
    Ok(None)
}

/// How far a trial fragment starting at `start` is allowed to run.
///
/// Capped by [`crate::MAX_IMAGE_BYTES`], so a trial can never ask the decoder
/// to walk the whole medium.
fn tail_len(start: u64, medium_len: u64) -> u64 {
    medium_len.saturating_sub(start).min(crate::MAX_IMAGE_BYTES)
}

/// Block boundaries where the first fragment could end, nearest the break
/// first.
fn prefix_candidates(header: u64, break_at: u64, limits: Limits) -> Vec<u64> {
    let mut ends = Vec::with_capacity(MAX_PREFIX_CANDIDATES);
    let mut end = limits.floor(break_at);
    while ends.len() < MAX_PREFIX_CANDIDATES && end > header {
        ends.push(end);
        end = end.saturating_sub(limits.block_bytes.max(1));
    }
    // A file whose header sits mid-block still has a first fragment; when no
    // block boundary falls inside it, the break point itself is the only
    // candidate.
    if ends.is_empty() && break_at > header {
        ends.push(break_at);
    }
    ends
}

/// Grows the best extent path for each broken candidate, greedily.
///
/// This is Parallel Unique Path: every header grows its own path, at each step
/// taking the fragment that carries the decoder furthest, and an extent
/// already claimed by one path is not offered to another. Candidate fragments
/// come from block classification — only blocks that could hold image data are
/// considered, which is what makes the search finite.
///
/// `spoken_for` are extents another technique has already recovered — the gap
/// search runs first, and its answers must not be offered here as free space.
/// Two artifacts claiming the same bytes would be two reports of one file that
/// the merge step cannot collapse, because their content hashes differ
/// (A-PROVENANCE).
///
/// Returns one [`Reassembly`] per header that completed; headers that did not
/// complete yield nothing, because a partial path is not a recoverable image.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn parallel_unique_path<R: Read + Seek>(
    src: &mut R,
    broken: &[Broken],
    candidates: &[Candidate],
    spoken_for: &[ByteRange],
    medium_len: u64,
    limits: Limits,
    scratch: &mut Scratch,
) -> Result<Vec<(Broken, Reassembly)>, CarveError> {
    let mut shared = Shared {
        claimed: spoken_for.to_vec(),
        attempts: 0,
    };
    let mut assembled = Vec::new();

    for &header in broken {
        if shared.attempts >= limits.max_hypotheses {
            break;
        }
        if let Some(reassembly) = grow_path(
            src,
            header,
            candidates,
            medium_len,
            &mut shared,
            limits,
            scratch,
        )? {
            shared.claimed.extend_from_slice(&reassembly.extents);
            assembled.push((header, reassembly));
        }
    }
    Ok(assembled)
}

/// What every header's walk shares: the blocks already spoken for, and the one
/// decode budget between them.
///
/// The budget is shared on purpose. Given per header it would multiply by the
/// number of broken candidates, and the whole point of bounding it is that a
/// medium full of false signature hits cannot make the stage run long.
struct Shared {
    /// Byte ranges already spoken for, by an earlier technique or by a path
    /// that completed. Whole ranges rather than start offsets: an extent
    /// covers many blocks, and withholding only its first one would let a
    /// later path read the same bytes again.
    claimed: Vec<ByteRange>,
    attempts: u32,
}

/// A block that could hold image data, as classification saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Where the block starts on the medium.
    pub start: ByteOffset,
    /// What the block looks like.
    pub profile: BlockProfile,
}

/// Grows one header's path, keeping the smoothest complete assembly it finds.
///
/// The walk does **not** stop at the first assembly that decodes. A shorter,
/// wrong path routinely decodes: splice a later fragment straight onto the
/// first and the decoder will often accept it, producing an image whose
/// remaining rows are reconstructed from mismatched coefficients. That is what
/// the smoothness term exists to separate — the correct assembly is the one
/// whose rows follow one another like a photograph's — so every completion is
/// scored and the best one wins.
fn grow_path<R: Read + Seek>(
    src: &mut R,
    broken: Broken,
    candidates: &[Candidate],
    medium_len: u64,
    shared: &mut Shared,
    limits: Limits,
    scratch: &mut Scratch,
) -> Result<Option<Reassembly>, CarveError> {
    let Shared { claimed, attempts } = shared;
    let header = broken.header.get();
    let break_at = broken.break_at.get();
    if break_at <= header {
        return Ok(None);
    }
    let Some(&first_end) = prefix_candidates(header, break_at, limits).first() else {
        return Ok(None);
    };

    let mut path = vec![ByteRange::new(ByteOffset::new(header), first_end - header)];
    // How much of the file the decoder has been carried through, in the units
    // `Probed::progress` counts. Not a position on the medium and not one in
    // the assembled stream — a consumption count, so noise cannot inflate it.
    let mut reached = 0_u64;
    // Blocks this path has taken. They only become unavailable to other
    // headers if the path completes: a greedy step that led nowhere must not
    // deny its blocks to the next header (each extent is assignable to one
    // path, but only a path that turned out to be a file).
    let mut taken: Vec<ByteRange> = Vec::new();
    // The smoothest complete assembly seen so far, and its score.
    let mut best: Option<(f32, Reassembly)> = None;

    while path.len() < limits.max_fragments {
        // The candidate that carried the decoder furthest without completing.
        let mut furthest: Option<(u64, u64, u64)> = None;

        for candidate in candidates {
            if !candidate.profile.class.can_hold_image_data() {
                continue;
            }
            let start = candidate.start.get();
            // A block inside an extent this path already holds, or one another
            // recovery already claimed, is not free space. Testing the whole
            // range rather than the start offset is what stops a path from
            // reading the same bytes twice and reporting a layout no allocator
            // could have produced.
            if covers(claimed, start) || covers(&path, start) {
                continue;
            }
            let tail = tail_len(start, medium_len);
            if tail == 0 {
                continue;
            }
            if *attempts >= limits.max_hypotheses {
                return Ok(finish(best, &mut path, claimed, &taken));
            }
            *attempts += 1;

            path.push(ByteRange::new(ByteOffset::new(start), tail));
            let probed = probe(src, broken.format, &path, scratch)?;
            let scored = match probed.complete {
                Some(length) => score(
                    src,
                    broken.format,
                    &trim_to(&path, length),
                    length,
                    &probed.seam_rows(),
                )?,
                None => None,
            };
            // Only a hypothesis that both completed and scored needs its path
            // kept; cloning for every one of thousands would allocate on the
            // hottest loop here for nothing (`M-MEM-REUSE`).
            let trial = scored.is_some().then(|| path.clone());
            path.pop();

            if probed.unsupported {
                // The oracle cannot check this coding, so no assembly built on
                // it may be claimed.
                return Ok(None);
            }

            match (probed.complete, scored) {
                (Some(length), Some(accepted)) => {
                    let better = best.as_ref().is_none_or(|(seam, _)| accepted.seam < *seam);
                    if let (true, Some(trial)) = (better, trial) {
                        best = Some((
                            accepted.seam,
                            Reassembly {
                                extents: trim_to(&trial, length),
                                length,
                                hypotheses: *attempts,
                            },
                        ));
                    }
                }
                _ => {
                    if furthest.is_none_or(|(best_so_far, ..)| probed.progress > best_so_far) {
                        furthest = Some((probed.progress, probed.consumed, start));
                    }
                }
            }
        }

        // Nothing carried the decoder further, so the path cannot grow. What
        // was already completed stands; guessing beyond it would be invention.
        let Some((progress, consumed, start)) = furthest else {
            break;
        };
        if progress <= reached {
            break;
        }
        reached = progress;
        // Commit the fragment at the length the decoder actually consumed —
        // an exact byte position from the entropy decoder, not arithmetic on a
        // stream offset. Left at the full tail it was offered, this fragment
        // would swallow the rest of the medium and no further one could ever
        // be reached. Fragments are block-granular, so it rounds to the grid.
        let before = total_len(&path);
        let take = limits
            .floor(consumed.saturating_sub(before))
            .max(limits.block_bytes.max(1))
            .min(tail_len(start, medium_len));
        let committed = ByteRange::new(ByteOffset::new(start), take);
        path.push(committed);
        taken.push(committed);
    }

    Ok(finish(best, &mut path, claimed, &taken))
}

/// Claims the blocks a completed path used, and returns it.
///
/// A path that never completed claims nothing: its blocks stay available to
/// the headers that follow.
fn finish(
    best: Option<(f32, Reassembly)>,
    path: &mut Vec<ByteRange>,
    claimed: &mut Vec<ByteRange>,
    taken: &[ByteRange],
) -> Option<Reassembly> {
    path.clear();
    let (_, reassembly) = best?;
    claimed.extend_from_slice(taken);
    Some(reassembly)
}

/// Whether `at` falls inside any of `extents`.
fn covers(extents: &[ByteRange], at: u64) -> bool {
    extents
        .iter()
        .any(|extent| extent.start.get() <= at && at < extent.end_saturating().get())
}

/// What one trial extent list told the decoder.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Probed {
    /// How much of the file the decoder accounted for: MCUs for JPEG, bytes
    /// of verified structure for PNG. Comparable only within one format, and
    /// deliberately **not** a stream position — a position is inflated by the
    /// segment skips that unknown bytes provoke, so ranking candidates by one
    /// ranks them by where they sat on the disk (see [`crate::mcu`]).
    progress: u64,
    /// First byte past what the decoder consumed, in assembled coordinates.
    /// This is what a committed fragment is trimmed to.
    consumed: u64,
    /// The image's length when the decoder accounted for all of it.
    complete: Option<u64>,
    /// MCU the decoder was on when it crossed the first fragment boundary.
    /// MCU reached at each fragment boundary, and how many are meaningful.
    seam_mcus: [u32; crate::mcu::MAX_SEAMS],
    seams: usize,
    /// MCUs per row and rows per MCU, for turning that into a pixel row.
    mcus_across: u32,
    mcu_rows: u32,
    /// Set when the format is one this oracle cannot check, so nothing may be
    /// claimed about the candidate either way.
    unsupported: bool,
}

impl Probed {
    /// Pixel row the first fragment boundary produced.
    /// Pixel row of every splice the decoder crossed.
    ///
    /// A boundary the decoder never reached contributes nothing: there is no
    /// row to look at, and inventing one would be worse than checking fewer.
    fn seam_rows(&self) -> Vec<u32> {
        if self.mcus_across == 0 {
            return Vec::new();
        }
        self.seam_mcus[..self.seams]
            .iter()
            .filter(|mcu| **mcu > 0)
            .map(|mcu| (mcu / self.mcus_across).saturating_mul(self.mcu_rows))
            .collect()
    }
}

/// Runs the format's decoder over a trial extent list.
fn probe<R: Read + Seek>(
    src: &mut R,
    format: Format,
    extents: &[ByteRange],
    scratch: &mut Scratch,
) -> Result<Probed, CarveError> {
    let length = total_len(extents);
    let mut probed = Probed {
        progress: 0,
        consumed: 0,
        complete: None,
        seam_mcus: [0; crate::mcu::MAX_SEAMS],
        seams: 0,
        mcus_across: 0,
        mcu_rows: 0,
        unsupported: false,
    };
    if length == 0 {
        return Ok(probed);
    }
    let mut stream = Assembled::new(src, extents);

    match format {
        Format::Jpeg => {
            // Watch every fragment boundary. Each is a splice, and each has a
            // pixel row that says whether the pieces either side belong
            // together; checking only the first would let an assembly whose
            // later joins are wrong pass on the strength of its first one.
            let mut seams = [0_u64; crate::mcu::MAX_SEAMS];
            let mut at = 0_u64;
            let mut count = 0_usize;
            for extent in extents.iter().take(extents.len().saturating_sub(1)) {
                if count == seams.len() {
                    break;
                }
                at = at.saturating_add(extent.len);
                seams[count] = at;
                count += 1;
            }
            let outcome = crate::mcu::scan_watching(
                &mut stream,
                ByteOffset::new(0),
                length,
                &seams[..count],
                scratch,
            )?;
            probed.progress = u64::from(outcome.mcus_decoded);
            probed.consumed = outcome.end.get();
            probed.seam_mcus = outcome.seam_mcus;
            probed.seams = outcome.seams;
            probed.mcus_across = outcome.mcus_across;
            probed.mcu_rows = outcome.mcu_rows;
            probed.unsupported = outcome.stop == crate::mcu::ScanStop::Unsupported;
            if outcome.is_complete() {
                probed.complete = Some(outcome.end.get());
            }
        }
        Format::Png => {
            // Every PNG chunk carries a CRC32, so the structural walk already
            // is an exact oracle: a chance assembly cannot forge one, and the
            // position it reaches is bounded by verified structure.
            match crate::validate(format, &mut stream, ByteOffset::new(0), length, scratch)? {
                Verdict::Complete { length, .. } => {
                    probed.progress = length;
                    probed.consumed = length;
                    probed.complete = Some(length);
                }
                Verdict::Corrupt { at, .. } => {
                    probed.progress = at.get();
                    probed.consumed = at.get();
                }
            }
        }
    }
    Ok(probed)
}

/// Judges a complete assembly by the picture at each of its splices.
///
/// `None` rejects it. The entropy decoder has already settled that the bytes
/// decode; this settles whether they are one file, which decoding alone cannot
/// (see [`MAX_SEAM_RATIO`]).
///
/// Every splice has to pass. An assembly of four fragments joins three times,
/// and a single wrong join makes the result a file that never existed — so the
/// worst seam is the one that decides, and it is also the score the assembly
/// is ranked by.
fn score<R: Read + Seek>(
    src: &mut R,
    format: Format,
    extents: &[ByteRange],
    length: u64,
    seam_rows: &[u32],
) -> Result<Option<Accepted>, CarveError> {
    match format {
        // A PNG's per-chunk CRC32 already proves the pieces belong together.
        Format::Png => Ok(Some(Accepted { length, seam: 0.0 })),
        Format::Jpeg => {
            // A single-fragment assembly has no splice to judge.
            if extents.len() < 2 {
                return Ok(Some(Accepted { length, seam: 0.0 }));
            }
            // Every join must have produced a row to look at. A boundary the
            // decoder never crossed leaves a splice unjudged, and an unjudged
            // splice is exactly what a fabrication hides behind.
            if seam_rows.len() != extents.len() - 1 {
                return Ok(None);
            }
            let Some(bytes) = read_all(src, extents)? else {
                // Too large to render, so the seams cannot be checked and the
                // assembly cannot be claimed.
                return Ok(None);
            };
            let Some(decoded) = decode::decode_jpeg_luma(&bytes) else {
                return Ok(None);
            };
            let mut worst = 0.0_f32;
            for row in seam_rows {
                let Some(seam) = decoded.seam_ratio(*row) else {
                    // Too small to judge; refuse rather than guess.
                    return Ok(None);
                };
                if seam > MAX_SEAM_RATIO {
                    return Ok(None);
                }
                worst = worst.max(seam);
            }
            Ok(Some(Accepted {
                length,
                seam: worst,
            }))
        }
    }
}

/// Whether a trial extent list is really the image, and to what length.
///
/// The entropy decoder is the gate: an assembly is the image only when every
/// MCU the frame declares decoded and the stream then reached `EOI`. Nothing
/// heuristic decides this.
fn decodes<R: Read + Seek>(
    src: &mut R,
    format: Format,
    extents: &[ByteRange],
    scratch: &mut Scratch,
) -> Result<Option<Accepted>, CarveError> {
    let probed = probe(src, format, extents, scratch)?;
    let Some(length) = probed.complete else {
        return Ok(None);
    };
    score(
        src,
        format,
        &trim_to(extents, length),
        length,
        &probed.seam_rows(),
    )
}

/// An assembly both the decoder and the picture confirmed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Accepted {
    length: u64,
    /// How far the splice stood out; lower is a better join.
    seam: f32,
}

/// Reads a trial assembly into memory for decoding.
///
/// `None` when it exceeds what [`decode`] will handle — an assembly that
/// cannot be verified is not claimed.
fn read_all<R: Read + Seek>(
    src: &mut R,
    extents: &[ByteRange],
) -> Result<Option<Vec<u8>>, CarveError> {
    let length = total_len(extents);
    if length > decode::MAX_DECODE_BYTES as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    let mut stream = Assembled::new(src, extents);
    stream
        .read_to_end(&mut bytes)
        .map_err(|source| CarveError::io(ByteOffset::new(0), source))?;
    Ok(Some(bytes))
}

/// Cuts an extent list down to exactly `length` bytes.
///
/// A trial fragment runs to a candidate end of file, which is usually past the
/// image's real end; the decoder reports where that end actually is, and the
/// recorded extents must describe the image and nothing more, or the manifest
/// would claim bytes the file does not contain.
fn trim_to(extents: &[ByteRange], length: u64) -> Vec<ByteRange> {
    let mut kept = Vec::with_capacity(extents.len());
    let mut remaining = length;
    for extent in extents {
        if remaining == 0 {
            break;
        }
        let take = extent.len.min(remaining);
        kept.push(ByteRange::new(extent.start, take));
        remaining -= take;
    }
    kept
}

/// Offsets inside `block` where a JPEG restart marker begins, as absolute
/// positions given that `block` starts at `start`.
///
/// A restart marker resynchronizes the entropy decoder, so a fragment that
/// begins at one can in principle be decoded without the fragment before it
/// (Uzun & Sencar, 2015).
///
/// The graph walk does **not** yet use these: it is offered block-aligned
/// candidates only, so an orphaned fragment is still reachable only through
/// its predecessor. This reports where such entry points are; wiring them in
/// as first-class nodes is not implemented.
#[must_use]
pub fn restart_points(block: &[u8], start: ByteOffset) -> Vec<ByteOffset> {
    let mut points = Vec::new();
    let mut index = 0_usize;
    while index + 1 < block.len() {
        if block[index] != 0xFF {
            index += 1;
            continue;
        }
        // RST0..RST7. Source: T.81 Table B.1.
        if matches!(block[index + 1], 0xD0..=0xD7) {
            // The fragment starts after the marker, where entropy data resumes.
            let at = start.get().saturating_add(index as u64).saturating_add(2);
            points.push(ByteOffset::new(at));
        }
        index += 2;
    }
    points
}
