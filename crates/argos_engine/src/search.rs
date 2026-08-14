//! What reassembly searches: one region of the medium at a time, in memory.
//!
//! The search is thousands of hypotheses per candidate, each reading a few
//! dozen bytes from a different offset. Served from the medium that is a seek
//! per hypothesis, and on rotational media a seek costs more than the decode it
//! feeds by three orders of magnitude — which is the difference between a
//! hypothesis costing microseconds and costing milliseconds.
//!
//! So the region a candidate can reach is read once, sequentially, and held.
//! Every hypothesis then reads memory. Regions are built to give each header
//! its whole search reach either side, which makes consecutive regions overlap
//! by at most that reach; the medium is therefore read at most twice over,
//! against once per header before
//! (`docs/defects/01-reassembly-unbounded.md`).

use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use argos_carve::classify;
use argos_carve::reassemble::{self, Broken};
use argos_core::geometry::{ByteOffset, ByteRange};

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
pub(crate) const REGION_BYTES: u64 = 4 * reassemble::MAX_GAP_BYTES;

/// Bytes read per request while filling a region.
///
/// Large enough to keep the reader in sequential requests, small enough that
/// one unreadable span costs a piece rather than the region.
const READ_BYTES: usize = 4 * 1024 * 1024;

/// Which headers search which stretch of the medium.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
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
pub(crate) fn plan_regions(
    broken: &[Broken],
    medium_len: u64,
    region_bytes: u64,
    reach: u64,
) -> Vec<Plan> {
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
pub(crate) struct Region {
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
    pub(crate) fn load<V: Read + Seek>(view: &mut V, range: ByteRange, bytes: Vec<u8>) -> Self {
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
    pub(crate) fn view(&self) -> RegionView<'_> {
        RegionView {
            start: self.start,
            bytes: &self.bytes,
            pos: self.start,
        }
    }

    /// Blocks that could hold image data, in medium order.
    pub(crate) fn blocks(&self) -> &[reassemble::Candidate] {
        &self.blocks
    }

    /// The searchable range, which bounds every hypothesis over this region.
    pub(crate) fn range(&self) -> ByteRange {
        ByteRange::new(ByteOffset::new(self.start), self.bytes.len() as u64)
    }

    /// Hands the buffer back so the next region can reuse it (`M-MEM-REUSE`).
    pub(crate) fn into_buffer(self) -> Vec<u8> {
        self.bytes
    }
}

/// A held region addressed the way the medium is.
///
/// Offsets outside it read as end-of-input rather than as anything else: a
/// hypothesis that reaches past what was held fails, and failing is the only
/// honest answer about bytes this view does not have.
pub(crate) struct RegionView<'a> {
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

/// Reads exactly `buf.len()` bytes from `at`.
fn read_exact_at<V: Read + Seek>(view: &mut V, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
    view.seek(SeekFrom::Start(at))?;
    view.read_exact(buf)
}

#[cfg(test)]
mod tests {
    use argos_core::Format;

    use super::{Plan, REGION_BYTES, plan_regions};
    use argos_carve::reassemble::{Broken, MAX_GAP_BYTES};
    use argos_core::geometry::ByteOffset;

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
