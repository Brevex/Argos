//! Stage E: reconstructing an image whose bytes are not contiguous.
//!
//! Planning the search, spending its budget across regions, and turning a
//! hypothesis the decoder accepted into a finding. The budget is finite, so
//! the order regions are searched in decides what a run reaches.
//!
//! **What a search reads from.** The search is thousands of hypotheses per
//! candidate, each reading a few dozen bytes from a different offset. Served
//! from the medium that is a seek per hypothesis, and on rotational media a
//! seek costs more than the decode it feeds by three orders of magnitude —
//! which is the difference between a hypothesis costing microseconds and one
//! costing milliseconds. So the region a candidate can reach is read once,
//! sequentially, and held; every hypothesis then reads memory. Regions are
//! built to give each header its whole search reach either side, which makes
//! consecutive regions overlap by at most that reach, and the medium is
//! therefore read at most twice over, against once per header before
//! (`docs/defects/01-reassembly-unbounded.md`).

use std::ops::Range;

use argos_carve::classify;

use super::*;

/// What the reassembly stage is given to work with.
#[derive(Clone, Copy)]
pub(super) struct Reassembling<'a> {
    /// Fragmentation points carving localized, in medium order.
    pub(super) broken: &'a [Broken],
    /// Findings from the earlier stages, whose extents are already accounted
    /// for and must not be offered to the search as free space.
    pub(super) already_recovered: &'a [Finding],
    /// How long the stage may search, or `None` for every candidate.
    pub(super) budget: Option<std::time::Duration>,
    /// Smallest long side a frame may declare and still be searched.
    pub(super) min_long_side: u32,
    /// Volumes the sweep located, whose allocation units are the grid a
    /// fragment boundary really falls on.
    pub(super) volumes: &'a [Volume],
    /// End of the searchable region.
    pub(super) medium_len: u64,
}

/// Recovers images the medium stored in pieces.
///
/// The medium is walked in regions, each read once and held while every header
/// inside it is searched: the gap search first, because two fragments with a
/// gap is the dominant real pattern, then the graph walk over the region's
/// classified blocks for whatever the gap search could not complete. Serving
/// the search from memory is what makes a hypothesis cost its own bytes rather
/// than a seek.
///
/// The stage shares one wall-clock budget. Reassembly runs by default, so a
/// medium carrying thousands of fragmentation points must not be able to turn
/// a scan into an overnight job; when the budget runs out the report says so
/// rather than implying the medium held nothing more.
pub(super) fn reassemble_broken<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    views: &mut [V],
    work: Reassembling<'_>,
    progress: &P,
    report: &mut ScanReport,
) -> Vec<Finding> {
    let Reassembling {
        broken,
        already_recovered,
        budget,
        min_long_side,
        volumes,
        medium_len,
    } = work;
    // Held before the reader is borrowed: a region is memory once loaded, so
    // the search over it wants every core the run was given rather than the one
    // thread that read it.
    let workers = views.len();
    let Some(view) = views.first_mut() else {
        return Vec::new();
    };
    let (broken, plans) = plan_search(broken, min_long_side, medium_len, report);
    let broken = &*broken;
    let deadline = budget.map(|limit| std::time::Instant::now() + limit);
    let spent = || deadline.is_some_and(|at| std::time::Instant::now() >= at);

    // Each header is counted twice — once for the gap search, once for the
    // walk — plus one step per region read, because they are one stage on
    // screen. That total is steps and not headers, and it says so: a reader who
    // takes it for a header count reads a quarter of the queue searched as an
    // eighth, and the manifest's `reassembly_attempted` is the header count
    // (`A-CONFIDENCE-HONEST`). `StageFinished` settles the bar at the end.
    //
    // Which of the two a run reports is decided by whether it has a deadline.
    // Steps are not equal here and `plan_search` hands out the expensive ones
    // first, so a fraction of them is not a fraction of the time and no display
    // may read it as one; with a budget there is a quantity that is, and it is
    // the budget (`docs/defects/09`).
    let steps = (broken.len() as u64)
        .saturating_mul(2)
        .saturating_add(plans.len() as u64);
    let counter = match budget {
        Some(budget) => Counter::until(progress, Stage::Reassembly, budget),
        None => Counter::start(progress, Stage::Reassembly, steps, Unit::Steps),
    };

    let mut scratch = Scratch::new();
    let mut found = Vec::new();
    // Extents another recovery already accounts for. The search must not offer
    // them again: two artifacts over the same bytes are two reports of one
    // file, and the merge step cannot collapse them because their content
    // hashes differ (A-PROVENANCE). Filesystem metadata counts here too — a
    // run list is a stronger statement about which bytes belong together than
    // anything this stage can derive.
    let mut spoken_for: Vec<ByteRange> = already_recovered
        .iter()
        .flat_map(|finding| finding.extents.iter().copied())
        .collect();
    // Sized once, to the largest region the plans ask for. Growing it region
    // by region instead would hold the old span and the new one at the same
    // time on every step up, which is resident memory the run never addresses
    // — and the last of those steps is the peak (`M-INITIAL-CAPACITY`).
    let widest = plans
        .iter()
        .map(|plan| plan.range.len)
        .max()
        .unwrap_or_default();
    let mut buffer = Vec::with_capacity(usize::try_from(widest).unwrap_or(0));

    // One thread does nothing but look at the clock while the search runs. The
    // stage's items are minutes long and the counter only speaks when one ends,
    // so without this a run reporting every five seconds by contract went an
    // hour and eight minutes without a word — which is what a stalled run looks
    // like, and what got a working one cancelled (`docs/defects/09`).
    let ticking = std::sync::atomic::AtomicBool::new(true);
    thread::scope(|scope| {
        scope.spawn(|| {
            while ticking.load(Ordering::Relaxed) {
                std::thread::sleep(TICK_POLL);
                counter.tick();
            }
        });

        // Immediately-called so the ticker is stopped on every way out,
        // including the two the search leaves by early.
        let found = (|| {
            for plan in plans {
                if control.is_cancelled() {
                    return found;
                }
                if spent() {
                    report.ceilings.reassembly_decodes = true;
                    return found;
                }
                let region = Region::load(view, plan.range, buffer);
                counter.step();
                let searched = search_region(
                    &region,
                    &broken[plan.headers.clone()],
                    allocation_grid(volumes, region.range()),
                    &spoken_for,
                    &mut Searching {
                        control,
                        counter: &counter,
                        spent: &spent,
                        scratch: &mut scratch,
                        report,
                        workers,
                    },
                );
                // What this region claimed carries to the next one, which
                // narrows from it again.
                for finding in &searched {
                    spoken_for.extend_from_slice(&finding.extents);
                }
                found.extend(searched);

                buffer = region.into_buffer();
            }
            report.ceilings.reassembly_decodes |= spent();
            found
        })();

        ticking.store(false, Ordering::Relaxed);
        found
    })
}

/// What one region's search shares with the stage around it.
///
/// Bundled because they travel together and none of them is about the region:
/// the flag that stops the run, the bar it reports on, the clock it is
/// rationed by, the working memory it reuses, and the account it adds to.
struct Searching<'a, P: ?Sized> {
    control: &'a Control,
    counter: &'a Counter<'a, P>,
    spent: &'a (dyn Fn() -> bool + Sync),
    scratch: &'a mut Scratch,
    report: &'a mut ScanReport,
    /// Threads the region's headers are searched across.
    workers: usize,
}

/// Searches `items` across `workers` threads, one result per item in the order
/// the items were given.
///
/// A region is memory by the time this runs, so a worker needs nothing from the
/// medium: it takes its own view of the held bytes and its own working buffers,
/// and what the threads share is the region, the stop flag and the counter.
/// Results come back in item order whatever order they were computed in, so the
/// stage does not depend on how many threads ran it.
///
/// `None` for an item the search never reached, which is how cancelling and a
/// spent budget arrive here.
fn in_parallel<T, R, P>(
    items: &[T],
    region: &Region,
    ctx: &Searching<'_, P>,
    search: impl Fn(&mut RegionView<'_>, &mut Scratch, &T) -> R + Sync,
) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    P: ProgressSink + ?Sized,
{
    let mut out = Vec::with_capacity(items.len());
    out.resize_with(items.len(), || None);
    if items.is_empty() {
        return out;
    }
    let next = AtomicUsize::new(0);
    let (control, counter, spent) = (ctx.control, ctx.counter, ctx.spent);
    let batches = thread::scope(|scope| {
        let mut workers = Vec::with_capacity(ctx.workers.max(1));
        for _ in 0..ctx.workers.max(1) {
            let (next, search) = (&next, &search);
            workers.push(scope.spawn(move || {
                let mut scratch = Scratch::new();
                let mut mine = Vec::new();
                loop {
                    if control.is_cancelled() || spent() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    mine.push((index, search(&mut region.view(), &mut scratch, item)));
                    counter.step();
                }
                mine
            }));
        }
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Vec<_>>()
    });
    for batch in batches {
        for (index, result) in batch {
            out[index] = Some(result);
        }
    }
    out
}

/// Searches one held region for the images its headers were split into.
///
/// The gap search first, because two fragments with a gap is the dominant real
/// pattern and it is the cheaper question; then the graph walk over the
/// region's classified blocks, for whatever the gap search could not complete.
///
/// `spoken_for` is every extent the run has already accounted for, across the
/// whole medium. It is narrowed to this region once, here: a hypothesis cannot
/// reach past what was held, so a claim that lies outside can never match one,
/// and a search handed all of them would rule out the whole surface once per
/// candidate (`crates/argos_carve/tests/reassemble_at_scale.rs`).
fn search_region<P: ProgressSink + ?Sized>(
    region: &Region,
    headers: &[Broken],
    grid: (u64, u64),
    spoken_for: &[ByteRange],
    ctx: &mut Searching<'_, P>,
) -> Vec<Finding> {
    let range = region.range();
    let limits = reassemble::Limits {
        search_floor: range.start.get(),
        block_bytes: grid.0,
        block_origin: grid.1,
        ..reassemble::Limits::default()
    };
    // Every hypothesis is bounded by what was held, so none can ask for a byte
    // this region does not have.
    let searchable = range.end_saturating().get();
    let mut claimed_here: Vec<ByteRange> = spoken_for
        .iter()
        .copied()
        .filter(|extent| overlaps(*extent, range))
        .collect();

    let mut found = Vec::new();

    // The gap search consults nothing the headers share, so which thread takes
    // which header cannot change an answer: this is the sequential search, run
    // on as many cores as the run was given.
    let gaps = in_parallel(headers, region, ctx, |view, scratch, &candidate| {
        reassemble::bifragment(view, candidate, searchable, limits, scratch)
            .map(|attempt| attempt.reassembly)
    });

    let mut unresolved = Vec::new();
    for (&candidate, outcome) in headers.iter().zip(&gaps) {
        // A header the search never reached is not a header it failed on.
        let Some(outcome) = outcome else { continue };
        ctx.report.reassembly_attempted = ctx.report.reassembly_attempted.saturating_add(1);
        match outcome {
            Ok(Some(reassembly)) => {
                ctx.report.reassembled = ctx.report.reassembled.saturating_add(1);
                claimed_here.extend_from_slice(&reassembly.extents);
                found.push(finding_from_reassembly(candidate, reassembly));
            }
            Ok(None) => unresolved.push(candidate),
            // A candidate that cannot be read is one that cannot be
            // reassembled, and it is already counted as attempted.
            Err(_) => {}
        }
    }

    // The walk does consult the claimed set, so every thread is given the same
    // one — what was claimed when this phase began — and a result is taken only
    // if it still holds once the results before it are in. A walk that a
    // neighbour's recovery invalidated is run again here against everything
    // claimed by then, which is what makes the region's outcome the sequential
    // one for any number of threads. On measured media it is rare enough to
    // cost nothing: five recoveries in three hundred and ninety-nine attempts.
    let claimed_before = claimed_here.clone();
    let walks = in_parallel(&unresolved, region, ctx, |view, scratch, &candidate| {
        reassemble::parallel_unique_path(
            view,
            std::slice::from_ref(&candidate),
            region.blocks(),
            &claimed_before,
            searchable,
            limits,
            scratch,
        )
        .map(|walk| walk.assembled)
    });

    for (&candidate, outcome) in unresolved.iter().zip(&walks) {
        let Some(Ok(assembled)) = outcome else {
            continue;
        };
        let stale = assembled.iter().any(|(_, reassembly)| {
            reassembly
                .extents
                .iter()
                .any(|extent| claimed_here.iter().any(|claim| overlaps(*extent, *claim)))
        });
        let assembled = if stale {
            reassemble::parallel_unique_path(
                &mut region.view(),
                std::slice::from_ref(&candidate),
                region.blocks(),
                &claimed_here,
                searchable,
                limits,
                ctx.scratch,
            )
            .map(|walk| walk.assembled)
            .unwrap_or_default()
        } else {
            assembled.clone()
        };
        for (header, reassembly) in assembled {
            ctx.report.reassembled = ctx.report.reassembled.saturating_add(1);
            claimed_here.extend_from_slice(&reassembly.extents);
            found.push(finding_from_reassembly(header, &reassembly));
        }
    }
    found
}

/// The allocation grid a region's fragments fall on: unit size, and where it
/// is counted from.
///
/// A filesystem allocates in clusters counted from its own start, so those are
/// the only offsets a fragment can begin at. Stepping on them instead of on
/// every 4 KiB is exact — no real boundary is skipped, because every one of
/// them is a cluster boundary — and on a volume of 32 KiB clusters it is eight
/// times fewer hypotheses for the same reach, which the budget spends
/// elsewhere.
///
/// Falls back to the smallest cluster any filesystem here uses when no located
/// volume contains the region, or when two do and disagree: a finer grid tries
/// more than it needs to, while a wrong coarser one would step over the
/// boundary it was looking for.
fn allocation_grid(volumes: &[Volume], region: ByteRange) -> (u64, u64) {
    let default = (reassemble::BLOCK_BYTES, 0);
    let mut found: Option<(u64, u64)> = None;
    for volume in volumes {
        let (start, end) = (
            volume.range.start.get(),
            volume.range.end_saturating().get(),
        );
        if volume.allocation_bytes < reassemble::BLOCK_BYTES
            || !volume
                .allocation_bytes
                .is_multiple_of(reassemble::BLOCK_BYTES)
            || region.start.get() < start
            || region.end_saturating().get() > end
        {
            continue;
        }
        let grid = (volume.allocation_bytes, start);
        match found {
            Some(earlier) if earlier != grid => return default,
            _ => found = Some(grid),
        }
    }
    found.unwrap_or(default)
}

/// Which candidates the search takes, and the order it takes their regions in.
///
/// Two decisions, both made before a byte is read.
///
/// A frame declares its size before its data, so what a candidate claims to be
/// is known for the cost of a header. Searching only photograph-sized frames is
/// what stops a used disk's thumbnail cache — which outnumbers its photographs
/// by two orders of magnitude — from spending the budget: the entries are whole
/// small files, and no reassembly of one could produce anything but the small
/// file it already is (`docs/defects/02-thumbnail-provenance.md`).
///
/// Then regions go in the order of how many units the candidates inside each
/// decoded — MCUs for JPEG, counted absolutely rather than as a share of the
/// frame. A frame the decoder walked thousands of MCUs into is a photograph
/// whose first fragment survived; one it walked three into is a signature that
/// happened to land on plausible bytes. A share does not separate those,
/// because it measures the frame and not the evidence: a cache entry three
/// quarters decoded has walked fewer units than a photograph a tenth decoded,
/// and on a medium whose photographs are the large frames a share orders the
/// budget against them. A format that counts no units contributes zero, so its
/// regions sort last. When the clock runs out, it runs out having spent itself
/// on the first kind.
fn plan_search(
    broken: &[Broken],
    min_long_side: u32,
    medium_len: u64,
    report: &mut ScanReport,
) -> (Vec<Broken>, Vec<Plan>) {
    let taken: Vec<Broken> = broken
        .iter()
        .copied()
        .filter(|candidate| candidate.clears(min_long_side))
        .collect();
    report.reassembly_skipped_small = broken
        .len()
        .saturating_sub(taken.len())
        .try_into()
        .unwrap_or(u64::MAX);

    let mut plans: Vec<_> =
        plan_regions(&taken, medium_len, REGION_BYTES, reassemble::MAX_GAP_BYTES)
            .into_iter()
            .map(|plan| {
                let best = taken[plan.headers.clone()]
                    .iter()
                    .map(|candidate| candidate.decoded)
                    .max()
                    .unwrap_or(0);
                (plan, best)
            })
            .collect();
    // Ties keep medium order, so two runs over one medium agree.
    plans.sort_by_key(|(plan, best)| (std::cmp::Reverse(*best), plan.range.start));
    (taken, plans.into_iter().map(|(plan, _)| plan).collect())
}

/// Smallest share of a frame that has to decode before its prefix is reported.
///
/// Below this there is not enough picture to be worth a file: a frame the
/// decoder walked a twentieth of is a few rows at the top and grey beneath. The
/// bytes are still accounted for — the candidate is in the manifest either way
/// — so this decides whether a file is written, not whether the evidence is
/// recorded (`M-DOCUMENTED-MAGIC`).
const MIN_PARTIAL_PROGRESS: f64 = 0.05;

/// Whether the part of `candidate` that decoded is worth reporting as a file.
///
/// The two formats do not measure the same thing, and only one of them can
/// answer this at all.
///
/// A JPEG's entropy decoder accounts for the frame one MCU at a time, so every
/// byte up to [`Broken::decoded_end`] is *proven* to be this image and the
/// share of the frame they draw is known. That is a prefix worth a file.
///
/// A PNG is verified per chunk: a chunk's CRC32 confirms all of it or says
/// nothing about any of it. A file whose tail is gone has a truncated `IDAT`,
/// which cannot verify, so the confirmed prefix stops at the `IHDR` — thirty
/// three bytes, a description of a picture with none of the picture in it.
/// Writing that as a recovery would be writing a header and calling it a
/// photograph.
///
/// This is a property of the format rather than a gap here, and the way past
/// it is not a lower threshold: it is an incremental inflate of the `IDAT`
/// stream, which is self-describing and would confirm payload the way
/// [`crate::pipeline`]'s JPEG counterpart confirms MCUs. Until that exists,
/// a broken PNG is reported as a fragmentation point and nothing more
/// (`A-CONFIDENCE-HONEST`).
fn worth_reporting(candidate: &Broken) -> bool {
    match candidate.format {
        Format::Jpeg => candidate.progress() >= MIN_PARTIAL_PROGRESS,
        Format::Png => false,
    }
}

/// Reports what decoded of the images reassembly could not complete.
///
/// A photograph whose remainder was overwritten is not recoverable, but its
/// beginning is *on the medium* and decodes: a 3072x2304 frame the decoder
/// walked 58% of is the top thirteen hundred rows of the picture, which is the
/// difference between recognising a photograph and having only the EXIF
/// thumbnail its header happened to carry, at a size too small to make out.
///
/// The bytes reported are the medium's own, from the header to where the
/// decoder stopped; no `EOI` is appended and nothing is padded, so the digest
/// stays the digest of what was there (`A-PROVENANCE`). What the file is
/// missing is stated rather than hidden: the tier is the weakest there is, and
/// the record carries how much of the frame decoded.
pub(super) fn partial_prefixes(
    broken: &[Broken],
    whole_again: &HashSet<ByteOffset>,
    min_long_side: u32,
) -> Vec<Finding> {
    broken
        .iter()
        .filter(|candidate| !whole_again.contains(&candidate.header))
        .filter(|candidate| candidate.clears(min_long_side))
        .filter(|candidate| worth_reporting(candidate))
        .filter_map(|candidate| {
            // To the last whole unit, not to where the stream stopped being
            // this file: between the two are the bytes the decoder read on its
            // way to finding out, and they belong to whatever followed on the
            // medium rather than to this picture.
            let length = candidate
                .decoded_end
                .get()
                .checked_sub(candidate.header.get())
                .filter(|length| *length > 0)?;
            Some(Finding {
                format: candidate.format,
                stage: Stage::Carve,
                confidence: Confidence::PartialOrThumbnail,
                extents: Box::from([ByteRange::new(candidate.header, length)]),
                declared_size: None,
                timestamps: argos_core::Timestamps::default(),
                deleted: None,
                name: None,
                source_object: None,
                parent: None,
            })
        })
        .collect()
}

/// Turns a confirmed reassembly into a finding.
///
/// The tier is [`Confidence::Reassembled`], below a contiguous carve: the
/// bytes are the image — the entropy decoder settled that — but which bytes
/// belong together is a reconstruction, and the extent list is what makes it
/// reproducible (A-PROVENANCE).
fn finding_from_reassembly(broken: Broken, reassembly: &reassemble::Reassembly) -> Finding {
    Finding {
        format: broken.format,
        stage: Stage::Reassembly,
        confidence: Confidence::Reassembled,
        extents: reassembly.extents.clone().into_boxed_slice(),
        declared_size: None,
        timestamps: argos_core::Timestamps::default(),
        deleted: None,
        name: None,
        source_object: None,
        parent: None,
    }
}

/// Bytes of the medium held in memory while its headers are searched.
///
/// The region has to hold a header's whole reach either side of it, so it is
/// sized against [`reassemble::MAX_GAP_BYTES`]: at four times the reach, a
/// region covers headers spanning twice it, and consecutive regions advance by
/// half a region, which caps the medium being read at twice over.
///
/// This is the scan's resident memory during reassembly, and the largest
/// single allocation the engine makes (`A-BOUNDED-ALLOC`). Earlier stages have
/// finished by the time it is taken.
const REGION_BYTES: u64 = 4 * reassemble::MAX_GAP_BYTES;

/// Bytes read per request while filling a region.
///
/// Large enough to keep the reader in sequential requests, small enough that
/// one unreadable span costs a piece rather than the region.
const READ_BYTES: usize = 4 * 1024 * 1024;

/// Which headers search which stretch of the medium.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Plan {
    /// The stretch to hold in memory.
    pub range: ByteRange,
    /// Indices into the medium-ordered broken slice this region serves.
    pub headers: Range<usize>,
}

/// Groups fragmentation points into regions, in medium order.
///
/// A header is only put in a region that holds its whole reach, so the search
/// is never quietly shortened by where a region boundary fell; the one
/// exception is a header whose reach is wider than a whole region, which is
/// taken alone and searched over as much as fits, because a region that took
/// no header would not advance.
fn plan_regions(broken: &[Broken], medium_len: u64, region_bytes: u64, reach: u64) -> Vec<Plan> {
    // A header's region has to hold its first fragment as well as its search:
    // the splices the gap search tries run from the header to the break point,
    // so a region ending between the two would offer the decoder a prefix it
    // cannot read.
    let needs = |candidate: &Broken| {
        candidate
            .break_at
            .get()
            .max(candidate.header.get())
            .saturating_add(reach)
            .min(medium_len)
    };

    let mut plans: Vec<Plan> = Vec::new();
    let mut at = 0_usize;
    while at < broken.len() {
        let start = broken[at].header.get().saturating_sub(reach);
        let limit = start.saturating_add(region_bytes).min(medium_len);

        // The region must satisfy the furthest-reaching header it takes, which
        // is not always the last one: the slice is ordered by header, and a
        // candidate that broke early can follow one that broke late.
        let mut end = at;
        let mut wanted = 0_u64;
        while end < broken.len() {
            let next = needs(&broken[end]);
            if next > limit {
                break;
            }
            wanted = wanted.max(next);
            end += 1;
        }
        // At least one, or the walk stalls on a header whose reach exceeds a
        // whole region.
        let end = end.max(at + 1);
        let wanted = wanted.max(needs(&broken[end - 1]));

        let region_end = wanted.min(limit);
        if region_end > start {
            plans.push(Plan {
                range: ByteRange::new(ByteOffset::new(start), region_end - start),
                headers: at..end,
            });
        }
        at = end;
    }
    plans
}

/// A stretch of the medium in memory, with the blocks in it that could hold
/// image data.
struct Region {
    start: u64,
    bytes: Vec<u8>,
    blocks: Vec<reassemble::Candidate>,
}

impl Region {
    /// Reads `range` and classifies it, reusing `bytes` between regions.
    ///
    /// A span the medium refuses is left as it was and contributes no
    /// candidate blocks: nothing is fabricated for it, and the sweep has
    /// already recorded it as unreadable.
    fn load<V: Read + Seek>(view: &mut V, range: ByteRange, bytes: Vec<u8>) -> Self {
        let start = range.start.get();
        let len = usize::try_from(range.len).unwrap_or(usize::MAX);
        let mut bytes = bytes;
        bytes.clear();
        bytes.resize(len, 0);

        let mut filled = 0_usize;
        while filled < len {
            let take = READ_BYTES.min(len - filled);
            let at = start.saturating_add(filled as u64);
            if read_exact_at(view, at, &mut bytes[filled..filled + take]).is_err() {
                // Unreadable here. Zeroes stay, and they classify as low
                // entropy, so no candidate block comes out of them.
                bytes[filled..filled + take].fill(0);
            }
            filled += take;
        }

        let block = classify::BLOCK_BYTES;
        let blocks = bytes
            .chunks_exact(block)
            .enumerate()
            .filter_map(|(index, chunk)| {
                let profile = classify::classify(chunk);
                profile
                    .class
                    .can_hold_image_data()
                    .then(|| reassemble::Candidate {
                        start: ByteOffset::new(start.saturating_add((index * block) as u64)),
                        profile,
                    })
            })
            .collect();

        Self {
            start,
            bytes,
            blocks,
        }
    }

    /// The region as a seekable source in medium coordinates.
    fn view(&self) -> RegionView<'_> {
        RegionView {
            start: self.start,
            bytes: &self.bytes,
            pos: self.start,
        }
    }

    /// Blocks that could hold image data, in medium order.
    fn blocks(&self) -> &[reassemble::Candidate] {
        &self.blocks
    }

    /// The searchable range, which bounds every hypothesis over this region.
    fn range(&self) -> ByteRange {
        ByteRange::new(ByteOffset::new(self.start), self.bytes.len() as u64)
    }

    /// Hands the buffer back so the next region can reuse it (`M-MEM-REUSE`).
    fn into_buffer(self) -> Vec<u8> {
        self.bytes
    }
}

/// A held region addressed the way the medium is.
///
/// Offsets outside it read as end-of-input rather than as anything else: a
/// hypothesis that reaches past what was held fails, and failing is the only
/// honest answer about bytes this view does not have.
struct RegionView<'a> {
    start: u64,
    bytes: &'a [u8],
    pos: u64,
}

impl Read for RegionView<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(from) = self.pos.checked_sub(self.start) else {
            return Ok(0);
        };
        let from = usize::try_from(from).unwrap_or(usize::MAX);
        let Some(available) = self.bytes.len().checked_sub(from) else {
            return Ok(0);
        };
        let take = available.min(buf.len());
        if take == 0 {
            return Ok(0);
        }
        buf[..take].copy_from_slice(&self.bytes[from..from + take]);
        self.pos = self.pos.saturating_add(take as u64);
        Ok(take)
    }
}

impl Seek for RegionView<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let end = self.start.saturating_add(self.bytes.len() as u64);
        let target = match pos {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(delta) => end.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        let Some(target) = target else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before the start of the medium",
            ));
        };
        self.pos = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_carve::reassemble::MAX_GAP_BYTES;

    fn broken(header: u64, declared: (u32, u32), decoded: u32, required: u32) -> Broken {
        Broken {
            header: ByteOffset::new(header),
            break_at: ByteOffset::new(header + 4096),
            format: Format::Jpeg,
            declared: Some(declared),
            decoded,
            required,
            decoded_end: ByteOffset::new(header + 2048),
        }
    }

    /// The floor decides what is searched at all; the order decides what a
    /// clock reaches. Both these frames clear a 300 px floor, so only the order
    /// separates them: 594 of 625 MCUs is most of a 400x400 derived image,
    /// 2,300 of 23,232 is a tenth of a 2816x2112 photograph. The share prefers
    /// the small frame, the units prefer the photograph, and the photograph is
    /// what a clock should buy.
    #[test]
    fn a_photograph_a_tenth_decoded_is_searched_before_a_small_frame_nearly_through() {
        let cache = broken(1 << 30, (400, 400), 594, 625);
        let photo = broken(8 << 30, (2816, 2112), 2_300, 23_232);
        assert!(
            cache.progress() > photo.progress(),
            "the premise: a share orders these the other way round"
        );

        let mut report = ScanReport::default();
        let (taken, plans) = plan_search(&[cache, photo], 300, 16 << 30, &mut report);

        assert_eq!(taken.len(), 2, "both frames clear the floor");
        assert_eq!(
            taken[plans[0].headers.clone()][0].header,
            photo.header,
            "the region holding the photograph is searched first"
        );
    }

    /// A format whose parser counts no units reports zero decoded, which must
    /// order its regions last rather than tie them with everything else.
    #[test]
    fn a_candidate_that_counts_no_units_sorts_behind_one_that_does() {
        let uncounted = Broken {
            format: Format::Png,
            declared: None,
            decoded: 0,
            required: 0,
            ..broken(1 << 30, (0, 0), 0, 0)
        };
        let counted = broken(8 << 30, (2816, 2112), 12, 23_232);

        let mut report = ScanReport::default();
        let (taken, plans) = plan_search(&[uncounted, counted], 300, 16 << 30, &mut report);

        assert_eq!(
            taken[plans[0].headers.clone()][0].header,
            counted.header,
            "twelve MCUs is still evidence; no count at all is not"
        );
    }

    fn broken_at(offset: u64) -> Broken {
        Broken {
            header: ByteOffset::new(offset),
            break_at: ByteOffset::new(offset + 4096),
            format: Format::Jpeg,
            declared: Some((1600, 1200)),
            decoded: 100,
            required: 300,
            decoded_end: ByteOffset::new(offset + 4000),
        }
    }

    fn plans(broken: &[Broken], medium: u64) -> Vec<Plan> {
        plan_regions(broken, medium, REGION_BYTES, MAX_GAP_BYTES)
    }

    #[test]
    fn clustered_headers_are_read_once_instead_of_once_each() {
        // The measured shape of a real medium: 203 fragmentation points inside
        // eight megabytes. A region each would read the same bytes 203 times,
        // which is what turned this stage into an overnight job
        // (docs/defects/01-reassembly-unbounded.md).
        let medium = 1 << 40;
        let base = 100 * 1024 * 1024;
        let broken: Vec<_> = (0..203)
            .map(|index| broken_at(base + index * 40 * 1024))
            .collect();

        let plans = plans(&broken, medium);

        assert_eq!(
            plans.len(),
            1,
            "one cluster is one sequential read: {plans:?}"
        );
        assert_eq!(plans[0].headers, 0..203, "and it serves every header");
        let read: u64 = plans.iter().map(|plan| plan.range.len).sum();
        let separate = broken.len() as u64 * 2 * MAX_GAP_BYTES;
        assert!(
            read * 50 < separate,
            "merging must collapse the work, not restate it: {read} vs {separate}"
        );
    }

    #[test]
    fn every_header_gets_its_whole_reach_either_side() {
        // The property that makes a region boundary invisible to recall: a
        // header assigned to a region can search as far inside it as it could
        // have searched on the medium.
        let medium = 1 << 40;
        let broken: Vec<_> = (0..40)
            .map(|index| broken_at(1_000_000_000 + index * 7 * 1024 * 1024))
            .collect();

        for plan in plans(&broken, medium) {
            let (from, to) = (plan.range.start.get(), plan.range.end_saturating().get());
            for header in &broken[plan.headers.clone()] {
                let at = header.header.get();
                assert!(
                    at.saturating_sub(MAX_GAP_BYTES) >= from || from == 0,
                    "a header cannot search behind the region it was put in"
                );
                assert!(
                    at.saturating_add(MAX_GAP_BYTES).min(medium) <= to,
                    "a header cannot search ahead of the region it was put in"
                );
            }
        }
    }

    #[test]
    fn a_region_holds_the_first_fragment_of_every_header_it_takes() {
        // The gap search offers the decoder everything from the header to the
        // break point. A region that ended between the two would hand it a
        // prefix it cannot read, and the candidate would be lost rather than
        // searched.
        let medium = 1 << 40;
        let base = 2_000_000_000;
        // Break points deliberately out of step with the headers: a candidate
        // that broke far follows ones that broke near.
        let broken: Vec<_> = (0..60)
            .map(|index| Broken {
                header: ByteOffset::new(base + index * 2 * 1024 * 1024),
                break_at: ByteOffset::new(
                    base + index * 2 * 1024 * 1024
                        + if index % 7 == 0 { 90 } else { 3 } * 1024 * 1024,
                ),
                format: Format::Jpeg,
                declared: Some((1600, 1200)),
                decoded: 100,
                required: 300,
                decoded_end: ByteOffset::new(base + index * 2 * 1024 * 1024),
            })
            .collect();

        for plan in plans(&broken, medium) {
            let to = plan.range.end_saturating().get();
            for candidate in &broken[plan.headers.clone()] {
                assert!(
                    candidate.break_at.get() <= to,
                    "the region ends at {to}, before the break at {:?}",
                    candidate.break_at
                );
            }
        }
    }

    #[test]
    fn headers_further_apart_than_a_region_stay_separate() {
        let medium = 1 << 40;
        let far = 8 * REGION_BYTES;
        let plans = plans(&[broken_at(far), broken_at(far * 4)], medium);
        assert_eq!(plans.len(), 2, "{plans:?}");
        assert!(
            plans[0].range.end_saturating().get() <= plans[1].range.start.get(),
            "regions come out in medium order: {plans:?}"
        );
    }

    #[test]
    fn the_medium_is_read_at_most_twice_over() {
        // Regions overlap on purpose — that is what gives every header its
        // reach — so the guarantee is a bound on the overlap, not its absence.
        let medium = 1 << 40;
        let broken: Vec<_> = (0..500)
            .map(|index| broken_at(1_000_000 + index * 3 * 1024 * 1024))
            .collect();

        let plans = plans(&broken, medium);
        let read: u64 = plans.iter().map(|plan| plan.range.len).sum();
        let spanned = plans
            .last()
            .map(|last| last.range.end_saturating().get())
            .unwrap_or_default()
            - plans[0].range.start.get();

        assert!(
            read <= spanned * 2,
            "reading {read} of a {spanned}-byte span is more than twice over"
        );
        for pair in plans.windows(2) {
            assert!(
                pair[0].range.start.get() < pair[1].range.start.get(),
                "regions must advance: {pair:?}"
            );
        }
    }

    #[test]
    fn a_region_never_runs_past_the_medium() {
        let medium = 32 * 1024 * 1024;
        for plan in plans(&[broken_at(medium - 4096)], medium) {
            assert!(
                plan.range.end_saturating().get() <= medium,
                "a read past the end is a read that fails: {plan:?}"
            );
        }
    }

    #[test]
    fn every_header_belongs_to_exactly_one_region() {
        let medium = 1 << 40;
        let broken: Vec<_> = (0..137)
            .map(|index| broken_at(500_000 + index * 11 * 1024 * 1024))
            .collect();

        let plans = plans(&broken, medium);
        let covered: Vec<usize> = plans.iter().flat_map(|plan| plan.headers.clone()).collect();
        assert_eq!(
            covered,
            (0..broken.len()).collect::<Vec<_>>(),
            "every header is searched, once"
        );
    }
}
