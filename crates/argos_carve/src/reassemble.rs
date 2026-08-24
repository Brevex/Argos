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

use std::io::{self, Read, Seek, SeekFrom};

use argos_core::geometry::{ByteOffset, ByteRange};

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
/// This is what bounds the time a single fragmented candidate can take,
/// independent of anything read from the medium. It is set to what a complete
/// search proposes rather than below it, so a candidate whose remainder is on
/// the medium is not missed for want of budget: [`MAX_PREFIX_CANDIDATES`]
/// splices, each sweeping [`MAX_GAP_BYTES`] either side of the header in
/// [`BLOCK_BYTES`] steps, is 8 x 2 x 16384.
pub const MAX_HYPOTHESES: u32 = 262_144;

/// How far from the end of a path the walk looks for its continuation.
///
/// A fragmented file's pieces lie near one another: the allocator split it
/// because no single run was long enough, and took the nearest runs it had.
/// Beyond this the candidates are other files' blocks, and testing them costs
/// the budget the near ones need.
pub(crate) const SEARCH_RADIUS_BYTES: u64 = MAX_GAP_BYTES;

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
pub(crate) const MAX_SEAM_RATIO: f32 = 3.0;

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
    /// How far from a path's end the graph walk considers a continuation.
    pub search_radius_bytes: u64,
    /// Lowest offset a fragment may be proposed at.
    ///
    /// The searchable region's far end is `medium_len`; this is its near end.
    /// A caller serving the search from a held region sets both to the region,
    /// so no hypothesis asks for bytes it does not have.
    pub search_floor: u64,
    /// Offset the block grid is counted from.
    ///
    /// A filesystem allocates in units counted from *its own* start, not the
    /// medium's, so a volume that does not begin on a multiple of its cluster
    /// size puts every real fragment boundary off the absolute grid. Setting
    /// this to the volume's start is what makes the search step on the
    /// boundaries an allocator actually produced.
    pub block_origin: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_gap_bytes: MAX_GAP_BYTES,
            max_fragments: MAX_FRAGMENTS,
            max_hypotheses: MAX_HYPOTHESES,
            block_bytes: BLOCK_BYTES,
            search_radius_bytes: SEARCH_RADIUS_BYTES,
            search_floor: 0,
            block_origin: 0,
        }
    }
}

impl Limits {
    /// Distance from the grid's origin, for an offset at or above it.
    fn since_origin(self, at: u64) -> Option<u64> {
        at.checked_sub(self.block_origin)
    }

    /// The block boundary at or below `at`.
    fn floor(self, at: u64) -> u64 {
        let block = self.block_bytes.max(1);
        let Some(offset) = self.since_origin(at) else {
            return at;
        };
        at - (offset % block)
    }

    /// The block boundary at or above `at`.
    fn ceil(self, at: u64) -> u64 {
        let block = self.block_bytes.max(1);
        let Some(offset) = self.since_origin(at) else {
            return at;
        };
        match offset % block {
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
    /// Pixel dimensions the frame header declares, when the format states them
    /// before its data.
    ///
    /// Known without decoding anything, which is what makes it usable as a
    /// gate: a search can tell a photograph-sized frame from a cache entry for
    /// the cost of reading a header, and spend its budget accordingly. `None`
    /// for a format whose parser does not report them, and a `None` is never
    /// read as "too small".
    pub declared: Option<(u32, u32)>,
    /// Units of the image the decoder accounted for before it stopped — MCUs
    /// for JPEG. Zero for a format that does not count in them.
    pub decoded: u32,
    /// Units the whole image requires, in the same counting.
    pub required: u32,
    /// First byte past the last unit that decoded whole.
    ///
    /// `[header, decoded_end)` is the part of the image the medium still holds
    /// and the decoder confirmed, which is what a caller reports when the rest
    /// cannot be found. It is at or below [`Broken::break_at`], which marks
    /// where the stream stopped being this file rather than where the file
    /// stopped.
    pub decoded_end: ByteOffset,
}

impl Broken {
    /// Whether the frame declares a picture at least `floor` pixels on its
    /// long side.
    ///
    /// A frame that declares nothing clears it: the floor exists to keep
    /// caches of small pictures out of a search, not to punish a format whose
    /// header this crate does not read dimensions from.
    #[must_use]
    pub fn clears(&self, floor: u32) -> bool {
        self.declared
            .is_none_or(|(width, height)| width.max(height) >= floor)
    }

    /// How much of the image the decoder reached, as a fraction.
    ///
    /// Zero when the image's size is unknown, so a caller cannot mistake "not
    /// counted" for "nothing decoded".
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.required == 0 {
            return 0.0;
        }
        f64::from(self.decoded) / f64::from(self.required)
    }
}

/// A reassembled image: which bytes, in which order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reassembly {
    /// Source extents in file order. Concatenating them yields the image.
    pub extents: Vec<ByteRange>,
    /// Image length in bytes, as the decoder confirmed it.
    pub length: u64,
}

/// What one search produced and what it cost.
///
/// The cost is reported whether or not anything was found, because a caller
/// rationing a budget across a medium's candidates has to charge what a search
/// actually spent. Charging every failure its ceiling instead spends the budget
/// on the accounting: a scan of a 1 TB disk reached 63 of its 200 fragmentation
/// points that way (`docs/defects/05-reassembly-never-ran.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// The image, when the oracle confirmed one.
    pub reassembly: Option<Reassembly>,
    /// Decode attempts spent.
    pub hypotheses: u32,
}

/// What a walk over several headers produced and what it cost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    /// One reassembly per header that completed.
    pub assembled: Vec<(Broken, Reassembly)>,
    /// Decode attempts spent across every header.
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
    let (break_at, declared, decoded, required, decoded_end) = match format {
        Format::Jpeg => {
            let outcome = crate::mcu::scan(src, header, limit, scratch)?;
            if outcome.is_complete()
                || outcome.stop == crate::mcu::ScanStop::Unsupported
                || outcome.mcus_decoded == 0
            {
                return Ok(None);
            }
            (
                outcome.end,
                (outcome.width > 0 && outcome.height > 0)
                    .then_some((outcome.width, outcome.height)),
                outcome.mcus_decoded,
                outcome.mcus_required,
                outcome.settled,
            )
        }
        Format::Png => {
            // A PNG's per-chunk CRC32 stops the structural walk exactly where
            // the data stops being the file, so it needs no other oracle. It
            // counts verified bytes rather than picture units, so nothing is
            // claimed about how much of the picture that is.
            //
            // The `IHDR` is read separately and cheaply, because what a
            // candidate declares itself to be is what lets a search skip the
            // icons and cache entries that outnumber photographs on a used
            // disk. Without it every PNG candidate clears every size floor.
            let declared = crate::png::header_dimensions(src, header, limit, scratch)?;
            match crate::validate(format, src, header, limit, scratch)? {
                Verdict::Complete { .. } => return Ok(None),
                Verdict::Corrupt { at, .. } => (at, declared, 0, 0, at),
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
        declared,
        decoded,
        required,
        decoded_end,
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
) -> Result<Attempt, CarveError> {
    let header = broken.header.get();
    let break_at = broken.break_at.get();
    let mut attempts = 0_u32;
    let nothing = |attempts| Attempt {
        reassembly: None,
        hypotheses: attempts,
    };
    if break_at <= header || break_at >= medium_len {
        return Ok(nothing(attempts));
    }

    let step = limits.block_bytes.max(1);
    let mut trial = Vec::new();
    // Ends of the first fragment: the block boundary at or below the break,
    // then earlier ones, since a wrong splice can parse on for a while.
    let prefixes = prefix_candidates(header, break_at, limits);
    let mut remaining_prefixes = prefixes.len();
    for first_end in prefixes {
        // Each prefix gets a share of what is left, so an early one that finds
        // nothing cheaply does not deny a later one its turn — and whatever it
        // leaves unspent rolls forward.
        let share = limits
            .max_hypotheses
            .saturating_sub(attempts)
            .div_ceil(u32::try_from(remaining_prefixes.max(1)).unwrap_or(1));
        remaining_prefixes = remaining_prefixes.saturating_sub(1);
        let ceiling = attempts.saturating_add(share).min(limits.max_hypotheses);
        if attempts >= ceiling {
            continue;
        }

        let first = [ByteRange::new(ByteOffset::new(header), first_end - header)];
        let Some(oracle) = Oracle::at(src, broken.format, &first, scratch)? else {
            continue;
        };

        // Ahead of the first fragment, smallest gap first (Garfinkel's
        // ordering): an allocator that split a file usually put the remainder
        // just past it.
        let mut second_start = limits.ceil(first_end.saturating_add(1));
        let furthest = second_start
            .saturating_add(limits.max_gap_bytes)
            .min(medium_len);
        while second_start < furthest {
            if attempts >= ceiling {
                break;
            }
            attempts += 1;
            if let Some(found) = try_second(
                &oracle,
                src,
                broken.format,
                &first,
                second_start,
                medium_len,
                &mut trial,
                scratch,
            )? {
                return Ok(Attempt {
                    reassembly: Some(found),
                    hypotheses: attempts,
                });
            }
            second_start = second_start.saturating_add(step);
        }

        // Behind the header, which is where an allocator puts a remainder when
        // it fills a hole it had passed over. The trial fragment still runs
        // forward from there, so it may reach back across the header; an
        // assembly whose extents overlap is refused rather than reported.
        let lowest = header
            .saturating_sub(limits.max_gap_bytes)
            .max(limits.search_floor);
        let mut second_start = limits.floor(header);
        while let Some(next) = second_start.checked_sub(step) {
            if next < lowest || attempts >= ceiling {
                break;
            }
            second_start = next;
            attempts += 1;
            if let Some(found) = try_second(
                &oracle,
                src,
                broken.format,
                &first,
                second_start,
                medium_len,
                &mut trial,
                scratch,
            )? {
                return Ok(Attempt {
                    reassembly: Some(found),
                    hypotheses: attempts,
                });
            }
        }
    }
    Ok(nothing(attempts))
}

/// Whether an extent list describes each byte at most once.
///
/// A trial second fragment runs forward to the end of the searchable region,
/// so one proposed behind the header can reach back over it. Two extents that
/// overlap describe a layout no allocator produced, and reporting one would
/// claim the same bytes twice (`A-PROVENANCE`).
fn disjoint(extents: &[ByteRange]) -> bool {
    for (index, first) in extents.iter().enumerate() {
        for second in &extents[index + 1..] {
            let (from, to) = (first.start.get(), first.end_saturating().get());
            let (other_from, other_to) = (second.start.get(), second.end_saturating().get());
            if from < other_to && other_from < to {
                return false;
            }
        }
    }
    true
}

/// A path prepared once, so testing a continuation of it costs that
/// continuation's bytes rather than the path's.
///
/// Both searches ask the same question thousands of times over: given these
/// bytes so far, how far does appending those bytes carry the decoder? Decoding
/// "these bytes so far" from `SOI` each time makes the answer cost the path
/// rather than the candidate, which is linear in a fragment that can be
/// megabytes (`docs/defects/05-reassembly-never-ran.md`).
enum Oracle {
    /// The entropy decoder stopped inside the path, ready to carry on.
    Jpeg {
        resume: Box<crate::mcu::Resume>,
        /// The path from the resume point to its end, in medium extents. A
        /// resumed decode replays these before reaching the continuation; it
        /// is at most one MCU's worth, since a resume point is an MCU
        /// boundary.
        replay: Vec<ByteRange>,
        /// Path bytes before the resume point, for putting a resumed decode's
        /// positions back into the path's own coordinates.
        before: u64,
    },
    /// A PNG needs none of this: its per-chunk CRC32 already makes a chance
    /// assembly impossible, and its walk is cheap enough to repeat.
    Png,
}

impl Oracle {
    /// Prepares `path`, which must be non-empty.
    ///
    /// `None` when nothing decodable starts there, which is the answer for most
    /// signature hits on a used medium.
    fn at<R: Read + Seek>(
        src: &mut R,
        format: Format,
        path: &[ByteRange],
        scratch: &mut Scratch,
    ) -> Result<Option<Self>, CarveError> {
        let length = total_len(path);
        if length == 0 {
            return Ok(None);
        }
        if format == Format::Png {
            return Ok(Some(Self::Png));
        }
        let resume = {
            let mut stream = Assembled::new(src, path);
            crate::mcu::resume_at(&mut stream, ByteOffset::new(0), length, scratch)?
        };
        let Some(resume) = resume else {
            return Ok(None);
        };
        let before = resume.replay_from().get();
        if before > length {
            return Ok(None);
        }
        Ok(Some(Self::Jpeg {
            resume: Box::new(resume),
            replay: tail_extents(path, before),
            before,
        }))
    }

    /// Runs the decoder over the path followed by `second`.
    ///
    /// `trial` is the caller's reusable extent buffer, so the hottest loop in
    /// the search allocates nothing (`M-MEM-REUSE`). Progress and consumption
    /// come back in the path's own coordinates, so a caller cannot tell a
    /// resumed answer from a repeated one.
    fn probe<R: Read + Seek>(
        &self,
        src: &mut R,
        path: &[ByteRange],
        second: ByteRange,
        trial: &mut Vec<ByteRange>,
        scratch: &mut Scratch,
    ) -> Result<Probed, CarveError> {
        let Self::Jpeg {
            resume,
            replay,
            before,
        } = self
        else {
            trial.clear();
            trial.extend_from_slice(path);
            trial.push(second);
            return probe(src, Format::Png, trial, scratch);
        };
        trial.clear();
        trial.extend_from_slice(replay);
        trial.push(second);
        let splice = total_len(replay);
        let outcome = {
            let mut stream = Assembled::new(src, trial);
            crate::mcu::scan_resumed(resume, &mut stream, total_len(trial), &[splice], scratch)?
        };
        let consumed = before.saturating_add(outcome.end.get());
        Ok(Probed {
            progress: u64::from(outcome.mcus_decoded),
            consumed,
            complete: outcome.is_complete().then_some(consumed),
            seam_mcus: outcome.seam_mcus,
            seams: outcome.seams,
            mcus_across: outcome.mcus_across,
            mcu_rows: outcome.mcu_rows,
            unsupported: outcome.stop == crate::mcu::ScanStop::Unsupported,
        })
    }

    /// Tests one continuation of `path`, returning the image when both the
    /// decoder and the picture at every splice confirm it.
    ///
    /// A completion is re-decoded from the start over the whole assembly. The
    /// resumed answer cannot see the splices before its resume point, and an
    /// assembly is only a file when *every* join holds — so what is cheap
    /// answers "how far", and what is exact answers "is it".
    fn completes<R: Read + Seek>(
        &self,
        src: &mut R,
        format: Format,
        path: &[ByteRange],
        second: ByteRange,
        trial: &mut Vec<ByteRange>,
        scratch: &mut Scratch,
    ) -> Result<Option<(f32, Reassembly)>, CarveError> {
        if self
            .probe(src, path, second, trial, scratch)?
            .complete
            .is_none()
        {
            return Ok(None);
        }
        trial.clear();
        trial.extend_from_slice(path);
        trial.push(second);
        let probed = probe(src, format, trial, scratch)?;
        let Some(length) = probed.complete else {
            return Ok(None);
        };
        let extents = trim_to(trial, length);
        if extents.len() != trial.len() || !disjoint(&extents) {
            return Ok(None);
        }
        let Some(accepted) = score(src, format, &extents, length, &probed.seam_rows())? else {
            return Ok(None);
        };
        Ok(Some((
            accepted.seam,
            Reassembly {
                extents,
                length: accepted.length,
            },
        )))
    }
}

/// Tests one continuation starting at `second_start`.
#[expect(
    clippy::too_many_arguments,
    reason = "the gap search's inner loop; bundling would allocate per hypothesis"
)]
fn try_second<R: Read + Seek>(
    oracle: &Oracle,
    src: &mut R,
    format: Format,
    path: &[ByteRange],
    second_start: u64,
    medium_len: u64,
    trial: &mut Vec<ByteRange>,
    scratch: &mut Scratch,
) -> Result<Option<Reassembly>, CarveError> {
    let tail = tail_len(second_start, medium_len);
    if tail == 0 {
        return Ok(None);
    }
    let second = ByteRange::new(ByteOffset::new(second_start), tail);
    Ok(oracle
        .completes(src, format, path, second, trial, scratch)?
        .map(|(_, reassembly)| reassembly))
}

/// The part of `path` from assembled offset `from` to its end.
///
/// A resume point sits inside one of the path's extents, so the extent holding
/// it is cut and the rest are carried whole.
fn tail_extents(path: &[ByteRange], from: u64) -> Vec<ByteRange> {
    let mut out = Vec::new();
    let mut at = 0_u64;
    for extent in path {
        let end = at.saturating_add(extent.len);
        if end > from {
            let skip = from.saturating_sub(at);
            out.push(ByteRange::new(
                ByteOffset::new(extent.start.get().saturating_add(skip)),
                extent.len.saturating_sub(skip),
            ));
        }
        at = end;
    }
    out
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
) -> Result<Walk, CarveError> {
    debug_assert!(
        candidates.is_sorted_by_key(|candidate| candidate.start.get()),
        "candidates must be in medium order: the walk locates the blocks nearest \
         a header by binary search over them"
    );
    let mut shared = Shared {
        claimed: Claimed::over(spoken_for),
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
            shared.claimed.add(&reassembly.extents);
            assembled.push((header, reassembly));
        }
    }
    Ok(Walk {
        assembled,
        hypotheses: shared.attempts,
    })
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
    claimed: Claimed,
    attempts: u32,
}

/// The bytes some other recovery already accounts for, answered in log time.
///
/// The walk asks "is this block spoken for?" once per candidate block it
/// considers — up to 65,536 of them for a 256 MiB region — and the set it asks
/// about holds one extent per artifact the run has recovered. A scan of a
/// terabyte handed it 1,639,834. Scanning that list per question makes the
/// stage cost the *product* of the two, which is a stage that finishes on
/// every fixture and never finishes on a disk.
///
/// Nothing about the answer needs the list to be walked: the ranges are
/// positions on a medium, so sorting them once turns every later question into
/// a binary search. They are merged as they are sorted, because overlapping
/// claims are one claim and a merged list lets the search stop at the single
/// candidate range instead of continuing past it.
struct Claimed {
    /// Disjoint ranges in ascending order.
    ranges: Vec<ByteRange>,
}

impl Claimed {
    /// The set `extents` describes, sorted and merged.
    fn over(extents: &[ByteRange]) -> Self {
        let mut ranges: Vec<ByteRange> = extents.iter().copied().filter(|r| r.len > 0).collect();
        ranges.sort_unstable_by_key(|range| range.start.get());
        Self::merge(&mut ranges);
        Self { ranges }
    }

    /// Collapses touching or overlapping ranges, which the search relies on:
    /// with them merged, at most one range can contain any offset.
    fn merge(ranges: &mut Vec<ByteRange>) {
        let mut kept: Vec<ByteRange> = Vec::with_capacity(ranges.len());
        for range in ranges.drain(..) {
            match kept.last_mut() {
                Some(last) if range.start.get() <= last.end_saturating().get() => {
                    let end = last
                        .end_saturating()
                        .get()
                        .max(range.end_saturating().get());
                    last.len = end.saturating_sub(last.start.get());
                }
                _ => kept.push(range),
            }
        }
        *ranges = kept;
    }

    /// Adds what a completed path claimed, keeping the set sorted and merged.
    ///
    /// Called once per recovered image rather than once per hypothesis, so the
    /// re-sort is paid on the rare event and not in the hot loop.
    fn add(&mut self, extents: &[ByteRange]) {
        self.ranges
            .extend(extents.iter().copied().filter(|range| range.len > 0));
        self.ranges.sort_unstable_by_key(|range| range.start.get());
        Self::merge(&mut self.ranges);
    }

    /// Whether `at` falls inside any claimed range.
    ///
    /// The ranges are disjoint and ascending, so the only one that can contain
    /// `at` is the last one starting at or before it.
    fn covers(&self, at: u64) -> bool {
        let after = self.ranges.partition_point(|range| range.start.get() <= at);
        after
            .checked_sub(1)
            .and_then(|index| self.ranges.get(index))
            .is_some_and(|range| at < range.end_saturating().get())
    }
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
    let header = broken.header.get();
    let break_at = broken.break_at.get();
    if break_at <= header {
        return Ok(None);
    }
    // Every splice the break point allows, as the gap search tries them. The
    // break is an upper bound, not the splice: a wrong continuation often
    // parses on past the real boundary, and a walk that only ever tried the
    // nearest block boundary could not reach the file behind it.
    let mut best: Option<(f32, Reassembly)> = None;
    for first_end in prefix_candidates(header, break_at, limits) {
        if shared.attempts >= limits.max_hypotheses {
            break;
        }
        let grown = grow_from(
            src,
            Splice { broken, first_end },
            candidates,
            medium_len,
            shared,
            limits,
            scratch,
        )?;
        if let Some((seam, reassembly)) = grown
            && best.as_ref().is_none_or(|(worst, _)| seam < *worst)
        {
            best = Some((seam, reassembly));
        }
    }
    Ok(best.map(|(_, reassembly)| reassembly))
}

/// One header and one place its first fragment might end.
#[derive(Clone, Copy)]
struct Splice {
    broken: Broken,
    first_end: u64,
}

/// Grows one header's path from one candidate splice.
fn grow_from<R: Read + Seek>(
    src: &mut R,
    splice: Splice,
    candidates: &[Candidate],
    medium_len: u64,
    shared: &mut Shared,
    limits: Limits,
    scratch: &mut Scratch,
) -> Result<Option<(f32, Reassembly)>, CarveError> {
    let Splice { broken, first_end } = splice;
    let header = broken.header.get();
    if first_end <= header {
        return Ok(None);
    }
    let mut walk = Walking {
        format: broken.format,
        candidates,
        medium_len,
        limits,
        path: vec![ByteRange::new(ByteOffset::new(header), first_end - header)],
        trial: Vec::new(),
        best: None,
    };
    step(src, &mut walk, 0, shared, scratch)?;
    Ok(walk.best)
}

/// One header's search, as it stands.
struct Walking<'a> {
    format: Format,
    candidates: &'a [Candidate],
    medium_len: u64,
    limits: Limits,
    /// Fragments committed so far, in file order.
    path: Vec<ByteRange>,
    /// Reusable extent buffer for the probe, so the hottest loop allocates
    /// nothing (`M-MEM-REUSE`).
    trial: Vec<ByteRange>,
    /// The smoothest complete assembly found, and its score.
    best: Option<(f32, Reassembly)>,
}

/// Continuations one step keeps when its best candidate may be the wrong one.
///
/// Committing only the furthest-reaching fragment and never reconsidering is
/// what held deep fragmentation down: a step whose best candidate is not the
/// true continuation loses the path for good, and with three fragments that
/// happens often enough to cost most of them. Keeping a few and trying each in
/// turn costs `MAX_BRANCH` times the nodes per level, which the resumed decoder
/// makes affordable — a probe costs its own bytes rather than the path's.
///
/// Three rather than more because the true continuation is the furthest-
/// reaching candidate or close behind it; past that the branches are other
/// files' blocks, and they multiply.
pub(crate) const MAX_BRANCH: usize = 3;

/// Fragments a path may hold and still have its steps reconsidered.
///
/// Reconsidering costs more than time. Every branch is another chance for an
/// assembly that decodes and whose seams look like a photograph's to be the
/// wrong one, and the seam check is what has to tell them apart — so how wide
/// the search may go is a question about the oracle, not about the budget.
///
/// Measured against planted ground truth, three fragments is where it holds:
/// branching there recovers 87% of them with nothing fabricated, against 25%
/// before. At four it does not — the suite produced an assembly of the right
/// length, decoding end to end, whose three seams all passed, and which was not
/// the planted bytes. Tightening the seam ratio does not separate that case
/// (2.5 and 2.0 leave it, 1.6 removes it only by refusing a third of the true
/// recoveries too), so the honest bound is depth. Beyond this the walk commits
/// its best candidate and does not look back, which is what it did everywhere
/// before (`M-DOCUMENTED-MAGIC`).
pub const MAX_BRANCHING_FRAGMENTS: usize = 3;

/// Extends `walk.path` by one fragment, every promising way, depth first.
///
/// `reached` is how far the decoder has been carried by the path already, in
/// the units [`Probed::progress`] counts. A branch that does not beat it is not
/// a continuation, and following one would let the search circle.
fn step<R: Read + Seek>(
    src: &mut R,
    walk: &mut Walking<'_>,
    reached: u64,
    shared: &mut Shared,
    scratch: &mut Scratch,
) -> Result<(), CarveError> {
    if walk.path.len() >= walk.limits.max_fragments || shared.attempts >= walk.limits.max_hypotheses
    {
        return Ok(());
    }
    // The path decoded once per node, so a probe costs the bytes it appends
    // rather than everything before them
    // (`docs/defects/05-reassembly-never-ran.md`).
    let Some(oracle) = Oracle::at(src, walk.format, &walk.path, scratch)? else {
        // Nothing decodes from this path, so nothing can continue it.
        return Ok(());
    };

    // A file's next fragment lies near the end of its last one: the allocator
    // was writing forward and took the nearest run it could. Sweeping the
    // candidate list in medium order instead spends the whole budget on
    // whatever happens to sit lowest on the disk, which for a header late on a
    // terabyte is never the continuation.
    let pivot = walk
        .path
        .last()
        .map_or(0, |extent| extent.end_saturating().get());
    let width = if walk.path.len() < MAX_BRANCHING_FRAGMENTS {
        MAX_BRANCH
    } else {
        1
    };
    let mut branches: Vec<(u64, u64, u64)> = Vec::with_capacity(width + 1);

    for candidate in nearest_first(walk.candidates, pivot, walk.limits.search_radius_bytes) {
        if !candidate.profile.class.can_hold_image_data() {
            continue;
        }
        // Before the tests below, not after them: a spent budget must end the
        // loop rather than go on paying for questions whose answers it can no
        // longer act on.
        if shared.attempts >= walk.limits.max_hypotheses {
            break;
        }
        let start = candidate.start.get();
        // A block inside an extent this path already holds, or one another
        // recovery already claimed, is not free space. Testing the whole range
        // rather than the start offset is what stops a path from reading the
        // same bytes twice and reporting a layout no allocator could have
        // produced.
        //
        // The claimed set is asked in log time; the path is at most
        // `max_fragments` extents and is walked.
        if shared.claimed.covers(start) || covers(&walk.path, start) {
            continue;
        }
        let tail = tail_len(start, walk.medium_len);
        if tail == 0 {
            continue;
        }
        shared.attempts += 1;

        let second = ByteRange::new(ByteOffset::new(start), tail);
        let probed = oracle.probe(src, &walk.path, second, &mut walk.trial, scratch)?;
        if probed.unsupported {
            // The oracle cannot check this coding, so no assembly built on it
            // may be claimed.
            walk.best = None;
            return Ok(());
        }

        if probed.complete.is_some() {
            // Exactly, over the whole assembly, so every splice is judged.
            if let Some((seam, reassembly)) = oracle.completes(
                src,
                walk.format,
                &walk.path,
                second,
                &mut walk.trial,
                scratch,
            )? && walk.best.as_ref().is_none_or(|(worst, _)| seam < *worst)
            {
                walk.best = Some((seam, reassembly));
            }
        } else if probed.progress > reached {
            keep_best(
                &mut branches,
                (probed.progress, probed.consumed, start),
                width,
            );
        }
    }

    for (progress, consumed, start) in branches {
        if shared.attempts >= walk.limits.max_hypotheses {
            break;
        }
        // Commit the fragment at the length the decoder actually consumed — an
        // exact byte position from the entropy decoder, not arithmetic on a
        // stream offset. Left at the full tail it was offered, this fragment
        // would swallow the rest of the medium and no further one could ever be
        // reached. Fragments are block-granular, so it rounds to the grid.
        let before = total_len(&walk.path);
        let take = walk
            .limits
            .floor(consumed.saturating_sub(before))
            .max(walk.limits.block_bytes.max(1))
            .min(tail_len(start, walk.medium_len));
        walk.path.push(ByteRange::new(ByteOffset::new(start), take));
        step(src, walk, progress, shared, scratch)?;
        walk.path.pop();
    }
    Ok(())
}

/// Keeps `branch` among the `width` furthest-reaching, best first.
fn keep_best(branches: &mut Vec<(u64, u64, u64)>, branch: (u64, u64, u64), width: usize) {
    let at = branches.partition_point(|(progress, ..)| *progress >= branch.0);
    if at >= width {
        return;
    }
    branches.insert(at, branch);
    branches.truncate(width);
}

/// Candidates ordered by distance from `pivot`, nearest first, within
/// `radius`.
///
/// The list is in medium order, so the nearest is found by binary search and
/// the two sides are walked outwards together.
fn nearest_first(candidates: &[Candidate], pivot: u64, radius: u64) -> NearestFirst<'_> {
    let split = candidates.partition_point(|candidate| candidate.start.get() < pivot);
    NearestFirst {
        candidates,
        pivot,
        radius,
        ahead: split,
        behind: split,
    }
}

/// Iterator over [`nearest_first`].
struct NearestFirst<'a> {
    candidates: &'a [Candidate],
    pivot: u64,
    radius: u64,
    /// Next index at or above the pivot.
    ahead: usize,
    /// One past the next index below the pivot.
    behind: usize,
}

impl<'a> Iterator for NearestFirst<'a> {
    type Item = &'a Candidate;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let above = self.candidates.get(self.ahead);
            let below = self
                .behind
                .checked_sub(1)
                .and_then(|index| self.candidates.get(index));
            let take_above = match (above, below) {
                (None, None) => return None,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some(above), Some(below)) => {
                    above.start.get().abs_diff(self.pivot) <= self.pivot.abs_diff(below.start.get())
                }
            };
            let picked = if take_above {
                self.ahead += 1;
                self.candidates.get(self.ahead - 1)
            } else {
                self.behind -= 1;
                self.candidates.get(self.behind)
            };
            let picked = picked?;
            if picked.start.get().abs_diff(self.pivot) > self.radius {
                // The list is sorted, so everything further out on this side is
                // further still: that side is finished.
                if take_above {
                    self.ahead = self.candidates.len();
                } else {
                    self.behind = 0;
                }
                continue;
            }
            return Some(picked);
        }
    }
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

/// A list of source extents presented as one contiguous byte stream.
///
/// This is what lets a hypothesis be tested: a format state machine
/// validates the proposed extents without knowing the bytes were ever
/// apart. Offsets addressed through it are **file offsets**, not medium
/// offsets — position zero is the file's first byte, wherever it lives.
#[derive(Debug)]
pub struct Assembled<'a, R> {
    src: &'a mut R,
    extents: &'a [ByteRange],
    /// Extent currently being read.
    index: usize,
    /// Bytes already produced from the current extent.
    consumed: u64,
    /// Whether the source is positioned for the current extent.
    positioned: bool,
}

impl<'a, R: Read + Seek> Assembled<'a, R> {
    /// Presents `extents` of `src`, in the given order, as one stream.
    ///
    /// The order is the file's order, which is not necessarily the medium's:
    /// a fragmented file's later bytes routinely sit at a lower offset than
    /// its earlier ones.
    pub fn new(src: &'a mut R, extents: &'a [ByteRange]) -> Self {
        Self {
            src,
            extents,
            index: 0,
            consumed: 0,
            positioned: false,
        }
    }

    /// Total length of the assembled stream.
    #[must_use]
    pub fn len(&self) -> u64 {
        total_len(self.extents)
    }

    /// Whether the assembled stream holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Position in the assembled stream: bytes produced so far.
    fn position(&self) -> u64 {
        self.extents
            .iter()
            .take(self.index)
            .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
            .saturating_add(self.consumed)
    }
}

/// Total length of an extent list, saturating rather than wrapping on a length
/// that could only come from a corrupt medium.
#[must_use]
pub(crate) fn total_len(extents: &[ByteRange]) -> u64 {
    extents
        .iter()
        .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
}

impl<R: Read + Seek> Read for Assembled<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let Some(extent) = self.extents.get(self.index) else {
                return Ok(0);
            };
            let remaining = extent.len.saturating_sub(self.consumed);
            if remaining == 0 {
                self.index += 1;
                self.consumed = 0;
                self.positioned = false;
                continue;
            }
            if !self.positioned {
                let at = extent
                    .start
                    .checked_add(self.consumed)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "extent position overflows the medium's address space",
                        )
                    })?
                    .get();
                self.src.seek(SeekFrom::Start(at))?;
                self.positioned = true;
            }
            let want = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            if want == 0 {
                return Ok(0);
            }
            let read = self.src.read(&mut buf[..want])?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the medium ends inside an extent this file claims",
                ));
            }
            self.consumed = self.consumed.saturating_add(read as u64);
            return Ok(read);
        }
    }
}

impl<R: Read + Seek> Seek for Assembled<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len().checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position().checked_add_signed(delta),
        };
        let Some(target) = target else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the assembled file",
            ));
        };

        // Walk to the extent holding `target`. Extent lists are bounded by the
        // caller (a fragment budget, or a filesystem's own run count), so this
        // stays short.
        let mut remaining = target;
        self.index = self.extents.len();
        self.consumed = 0;
        self.positioned = false;
        for (index, extent) in self.extents.iter().enumerate() {
            if remaining < extent.len {
                self.index = index;
                self.consumed = remaining;
                break;
            }
            remaining = remaining.saturating_sub(extent.len);
        }
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position())
    }
}
