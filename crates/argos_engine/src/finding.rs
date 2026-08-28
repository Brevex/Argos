//! What the pipeline produces before it becomes an artifact.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::fmt;

use argos_core::ports::{Digest, ModelIdentity, RunState};
use argos_core::{ByteOffset, ByteRange, Confidence, Format, Stage, Timestamps};
use argos_fs::Volume;

use crate::annotate::TriageOutcome;

/// One recoverable image located by some stage.
///
/// A finding is a claim about bytes on the medium: these extents, in this
/// order, are an image of this format, recovered by this stage with this much
/// evidence behind it. Nothing here is inferred — [`Finding::confidence`] is
/// the tier of the evidence that produced it and is never raised later
/// (A-CONFIDENCE-HONEST).
///
/// `Debug` redacts [`Finding::name`]: names read off a medium are identifying
/// content and must not reach a log or a panic message (A-NO-CONTENT-IN-LOGS).
#[derive(Clone, PartialEq, Eq)]
pub struct Finding {
    /// Image format.
    pub format: Format,
    /// Stage that produced the finding.
    pub stage: Stage,
    /// Evidence tier.
    pub confidence: Confidence,
    /// Content extents, absolute in the medium, in file order.
    pub extents: Box<[ByteRange]>,
    /// Length the source metadata claimed, when it said one. Larger than
    /// [`Finding::length`] means part of the file was not recovered.
    pub declared_size: Option<u64>,
    /// Timestamps recovered from the source metadata, never invented.
    pub timestamps: Timestamps,
    /// When the change journal says the file was deleted, when it said.
    ///
    /// The only timestamp on an NTFS volume that is about the *removal* rather
    /// than the file. A run of findings sharing one is a batch deletion.
    pub deleted: Option<std::time::SystemTime>,
    /// Name recovered from filesystem metadata, when one survived. It always
    /// belongs to [`Finding::source_object`].
    pub name: Option<Box<str>>,
    /// Filesystem object the metadata came from — MFT record number, inode
    /// number or first cluster.
    pub source_object: Option<u64>,
    /// For an embedded thumbnail, the offset of the candidate it was found in.
    pub parent: Option<ByteOffset>,
}

impl Finding {
    /// Total length in bytes: the sum of the extent lengths.
    #[must_use]
    pub fn length(&self) -> u64 {
        self.extents
            .iter()
            .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
    }

    /// Bytes the metadata expected but the extents do not cover, if any.
    ///
    /// Non-zero means the recovery is partial: a hole in the extent list, or a
    /// run that did not survive. It is stated, never quietly closed up
    /// (A-CONFIDENCE-HONEST).
    #[must_use]
    pub fn missing_bytes(&self) -> u64 {
        self.declared_size
            .unwrap_or(0)
            .saturating_sub(self.length())
    }

    /// Offset of the first byte, or zero for a finding with no extents.
    #[must_use]
    pub fn start(&self) -> ByteOffset {
        self.extents
            .first()
            .map_or(ByteOffset::new(0), |extent| extent.start)
    }

    /// Whether every extent lies inside `other`'s span — the test for a
    /// candidate that is really a piece of something already recovered.
    #[must_use]
    pub(crate) fn is_covered_by(&self, other: &Self) -> bool {
        let Some((low, high)) = other.span() else {
            return false;
        };
        !self.extents.is_empty()
            && self
                .extents
                .iter()
                .all(|extent| extent.start.get() >= low && extent.end_saturating().get() <= high)
    }

    /// Whether any extent overlaps `range` — the test for a finding whose
    /// bytes lie in a region the medium could not read.
    #[must_use]
    pub(crate) fn intersects(&self, range: ByteRange) -> bool {
        let range_end = range.end_saturating().get();
        self.extents.iter().any(|extent| {
            let end = extent.end_saturating().get();
            extent.start.get() < range_end && range.start.get() < end
        })
    }

    /// Lowest and highest byte the finding touches.
    fn span(&self) -> Option<(u64, u64)> {
        let low = self.extents.iter().map(|extent| extent.start.get()).min()?;
        let high = self
            .extents
            .iter()
            .map(|extent| extent.end_saturating().get())
            .max()?;
        Some((low, high))
    }
}

impl fmt::Debug for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Finding")
            .field("format", &self.format)
            .field("stage", &self.stage)
            .field("confidence", &self.confidence)
            .field("extents", &self.extents)
            .field("declared_size", &self.declared_size)
            .field("timestamps", &self.timestamps)
            .field("deleted", &self.deleted)
            .field("name", &self.name.as_ref().map(|_| "<redacted>"))
            .field("source_object", &self.source_object)
            .field("parent", &self.parent)
            .finish()
    }
}

/// The bounds a run reached, each meaning it looked at less than it set out to.
///
/// Every one of these is a deliberate limit on how long a stage may take, and
/// every one of them is reported rather than applied quietly: a scan that
/// stopped early and said nothing would be telling its reader the medium held
/// no more, which is a different claim entirely (A-CONFIDENCE-HONEST).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ceilings {
    /// Reassembly ran out of its decode budget, so candidates were left
    /// untried.
    pub reassembly_decodes: bool,
    /// The block search stopped at its byte ceiling before it had looked at
    /// every region around a fragmentation point. About how much was *read*,
    /// where the decode budget is about how much was tried.
    pub reassembly_search: bool,
    /// A detector hit its cap and stopped collecting, so the sweep covered the
    /// surface but did not report everything it saw. A medium that does this
    /// is patterned, deliberately or otherwise.
    pub detection: bool,
}

impl Ceilings {
    /// Each ceiling reached, named for a reader.
    pub fn reached(self) -> impl Iterator<Item = &'static str> {
        [
            (self.reassembly_decodes, "reassembly decode budget"),
            (self.reassembly_search, "reassembly search ceiling"),
            (self.detection, "detection cap"),
        ]
        .into_iter()
        .filter_map(|(hit, name)| hit.then_some(name))
    }
}

/// What a completed — or cancelled — scan found.
///
/// Everything here is countable evidence about the run itself; recovered
/// content leaves through the [`ArtifactSink`](argos_core::ports::ArtifactSink).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanReport {
    /// How the run ended.
    pub state: RunState,
    /// Artifacts handed to the sink.
    pub artifacts: u64,
    /// Signature hits that failed structural validation.
    pub rejected_candidates: u64,
    /// Artifacts dropped because an identical content hash was already
    /// reported.
    pub duplicates: u64,
    /// Findings whose claimed bytes could not be read back from the medium,
    /// so they were dropped rather than reported from whatever was there.
    pub unrecoverable: u64,
    /// Findings dropped because their bytes overlap a range the medium refused.
    ///
    /// Distinct from [`ScanReport::unrecoverable`], which is about a read that
    /// failed at report time; this is about bytes the sweep already knew it
    /// never got. Counted because the signature that started each of them was
    /// real: damage that costs recoveries must not read as a medium that held
    /// nothing (A-CONFIDENCE-HONEST).
    pub dropped_unreadable: u64,
    /// Artifacts recognised, recorded and deliberately not written, because
    /// the run was asked to leave synthetic assets out of the directory. They
    /// are in the manifest with their extents: the account stays complete.
    pub omitted_assets: u64,
    /// Bytes of the medium the sweep covered.
    pub bytes_swept: u64,
    /// Ranges the medium refused to read. Their content is unknown; nothing
    /// was fabricated for them and no artifact overlaps one.
    pub unreadable: Vec<ByteRange>,
    /// Volumes located, current and residual.
    pub volumes: Vec<Volume>,
    /// Every fragmentation point carving localized, in medium order.
    ///
    /// Recorded so the search can be run again without sweeping the medium
    /// again: locating these is what the sweep and the validation stage cost,
    /// and on a terabyte that is hours, while searching from them is minutes.
    /// A later run reads them back and starts at stage E.
    pub fragmentation: Vec<crate::Broken>,
    /// Images recovered by reassembling fragments.
    pub reassembled: u64,
    /// Broken candidates reassembly was offered.
    pub reassembly_attempted: u64,
    /// Images reported as the part of themselves that decodes, because their
    /// remainder is not on the medium to be found.
    pub partial_prefixes: u64,
    /// Broken candidates the search left alone because the frame declares a
    /// picture below the size floor.
    ///
    /// They are not lost: what carving established about each is still
    /// reported, and a run with a lower floor searches them. Counting them
    /// separately is what keeps "the search found nothing here" distinct from
    /// "the search did not look here" (`A-CONFIDENCE-HONEST`).
    pub reassembly_skipped_small: u64,
    /// Which of the run's ceilings were reached, if any.
    pub ceilings: Ceilings,
    /// Deletion events the change journals recorded.
    ///
    /// Names and moments, never extents: an event says a file was removed,
    /// which is not evidence its bytes survived (A-CONFIDENCE-HONEST).
    pub journal_deletions: u64,
    /// Residual `FILE`-record regions that could not be attributed to an NTFS
    /// volume, so their extents could not be resolved. They are counted, not
    /// guessed at.
    pub unattributed_residue: u64,
    /// Residual `FILE`-record regions a confirmed NTFS volume did cover, whose
    /// run lists were therefore resolved against a real geometry.
    ///
    /// The denominator [`ScanReport::unattributed_residue`] never had: without
    /// it, a run that resolved every region it found and a run that found
    /// almost none report the same way.
    pub attributed_residue: u64,
    /// What the passes over orphaned `FILE` records counted, attributed
    /// regions and unattributed alike.
    pub orphan_census: argos_fs::ntfs::Census,
    /// Filesystem-metadata claims dropped because the first extent carried no
    /// recognisable signature.
    ///
    /// The metadata said a file was here; the bytes here are not that file.
    /// Counting them is what separates "the clusters were reused" from "the
    /// geometry these were resolved against is wrong" — two conclusions the
    /// same silence used to serve (`A-CONFIDENCE-HONEST`).
    pub metadata_unconfirmed: u64,
    /// Files an orphaned `FILE` record still names, whose content this run
    /// could not place.
    ///
    /// **Not recoveries, and never counted as artifacts**: no bytes were read
    /// for them and no extent is claimed. What they are is evidence. A record
    /// keeps a file's name, size and times long after the boot sector that
    /// would locate its content is gone, and those three survive without any
    /// geometry at all — so a run that discards them reports a medium as
    /// emptier than the medium says it is (`A-CONFIDENCE-HONEST`).
    ///
    /// Each carries its run list whole, in the volume's own units, which is
    /// what makes a candidate geometry testable afterwards: the right cluster
    /// size is the one whose runs resolve to the file the record names.
    pub lost_files: Vec<argos_fs::ntfs::LostFile>,
    /// One triage annotation per persisted artifact, in emit order. Empty when
    /// triage did not run. Annotations only: nothing here can remove an
    /// artifact (A-TRIAGE-NOT-VERDICT).
    pub triage: Vec<TriageOutcome>,
    /// Identity of the model that scored this scan, when one did
    /// (A-MODEL-PINNED).
    pub triage_model: Option<ModelIdentity>,
    /// Artifacts that received a score.
    pub triage_scored: u64,
    /// Artifacts triage saw but could not score — too large to decode within
    /// bounds, undecodable, or left over after a classifier failure.
    pub triage_unscored: u64,
    /// Whether the classifier failed mid-run, leaving artifacts unscored that
    /// a healthy one would have scored.
    pub triage_degraded: bool,
    /// Where each written artifact stands in a list, keyed by content hash.
    ///
    /// A sort key, not a verdict: a recovery of a used disk writes hundreds of
    /// thousands of artifacts and a few hundred photographs, and this is what
    /// lets a reader put the photographs first. Every written artifact has one
    /// and nothing is removed by it (A-TRIAGE-NOT-VERDICT).
    pub standings: Vec<(argos_core::ports::Digest, argos_classify::rank::Standing)>,
    /// Artifacts found among same-sized neighbours, with how many there were.
    ///
    /// The signature of a thumbnail cache, which a used medium holds far more
    /// of than photographs. It is a count of neighbours and nothing more: no
    /// artifact is removed, reclassified or ranked by it
    /// (A-CONFIDENCE-HONEST).
    pub cache_runs: Vec<CacheRun>,
    /// Preview images rendered. Zero when previews were not requested.
    pub previews_written: u64,
    /// Artifacts whose preview could not be written. Each one is a thumbnail
    /// that is missing from the output directory and nothing more: the
    /// artifact itself was stored, hashed and recorded before this was tried.
    pub previews_failed: u64,
}

impl ScanReport {
    /// Whether the run covered everything it was asked to cover.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.state == RunState::Finished
            && self.unreadable.is_empty()
            && self.ceilings.reached().next().is_none()
    }
}

/// Drops what cannot honestly be reported and orders what remains.
///
/// Two stages routinely find the same image: filesystem metadata names it and
/// carving re-derives it from its signature. Merging keeps the strongest
/// evidence for each set of bytes and never invents a stronger tier than the
/// evidence that produced it (A-CONFIDENCE-HONEST). Content-hash deduplication
/// happens later, at emit time, because it needs the bytes.
///
/// In order: findings with no bytes, findings whose bytes lie in a region the
/// medium could not read, exact-extent duplicates (strongest tier wins), and
/// findings entirely covered by another of at least equal tier. Embedded
/// thumbnails survive coverage — a thumbnail always lies inside its parent and
/// is a separate artifact.
///
/// Returns how many were dropped for touching an unreadable range. That one is
/// counted and the others are not, because it is the only drop here that is
/// about the *medium* rather than about the findings: the signature that
/// started each of them was real, and a run that lost recoveries to damage must
/// not read as one that found the medium empty (A-CONFIDENCE-HONEST).
///
/// The resulting order is a total order over the findings' own fields, so two
/// runs over the same medium produce the same manifest.
pub(crate) fn consolidate(findings: &mut Vec<Finding>, unreadable: &[ByteRange]) -> u64 {
    let mut dropped_unreadable = 0_u64;
    let mut truncated = Vec::new();
    findings.retain_mut(|finding| {
        if finding.extents.is_empty() || finding.length() == 0 {
            return false;
        }
        if !unreadable.iter().any(|range| finding.intersects(*range)) {
            return true;
        }
        // The bytes before the damage are still the medium's own, and they are
        // still the start of this image. Damage is recorded at retry-span
        // granularity, so a bad sector condemns a whole span around it and the
        // photograph that reached into it loses everything — including the
        // part that read cleanly.
        dropped_unreadable += 1;
        if let Some(head) = head_before_damage(finding, unreadable) {
            truncated.push(head);
        }
        false
    });
    findings.append(&mut truncated);

    findings.sort_by(|a, b| {
        a.start()
            .cmp(&b.start())
            // Longest first, so a container is seen before what it contains.
            .then(b.length().cmp(&a.length()))
            // Strongest evidence first, so it is the one kept.
            .then(b.confidence.cmp(&a.confidence))
            .then(a.stage.cmp(&b.stage))
            .then(a.format.cmp(&b.format))
            .then(a.extents.cmp(&b.extents))
    });

    // Exact-extent duplicates: keep the strongest, but carry over the metadata
    // the weaker one had and the stronger one lacks.
    findings.dedup_by(|dropped, kept| {
        if dropped.extents != kept.extents {
            return false;
        }
        // A name and the object it was read from move together or not at all.
        // Splicing one finding's name onto another's object would assert an
        // association that exists nowhere on the medium (A-PROVENANCE).
        if kept.name.is_none() && kept.source_object.is_none() {
            kept.name = dropped.name.take();
            kept.source_object = dropped.source_object;
        } else if kept.name.is_none() && kept.source_object == dropped.source_object {
            kept.name = dropped.name.take();
        } else if kept.source_object.is_none() && dropped.name.is_none() {
            kept.source_object = dropped.source_object;
        }
        if kept.timestamps.is_empty() && kept.source_object == dropped.source_object {
            kept.timestamps = dropped.timestamps;
        }
        if kept.declared_size.is_none() && kept.source_object == dropped.source_object {
            kept.declared_size = dropped.declared_size;
        }
        if kept.parent.is_none() && dropped.parent.is_some() {
            kept.parent = dropped.parent;
            // Being an embedded thumbnail is a fact about what these bytes
            // *are*, and it outranks how they were found: a thumbnail that
            // also carves cleanly on its own is still a thumbnail. Metadata
            // evidence is never overridden this way — only carving is.
            if kept.stage == Stage::Carve {
                kept.confidence = kept.confidence.min(dropped.confidence);
            }
        }
        true
    });

    let mut kept: Vec<Finding> = Vec::with_capacity(findings.len());
    // Indices into `kept`, ordered by the byte each one reaches, furthest
    // first.
    //
    // A container has to reach at least as far as what it contains, so a
    // search can stop at the first candidate that does not reach far enough.
    // Without that order the search is against everything kept so far, and
    // there is a shape of medium that makes that quadratic: one weakly
    // evidenced carve spanning thousands of files the filesystem also named.
    // It cannot cover any of them — its evidence is weaker — so every one of
    // them is kept, and every one of them is then compared against every one
    // before it. That is a real disk, not a contrived one, and it costs hours
    // in a stage that says nothing while it runs.
    let mut by_reach: BTreeSet<(Reverse<u64>, usize)> = BTreeSet::new();
    let mut furthest_end = 0_u64;
    for finding in findings.drain(..) {
        let end = finding
            .extents
            .iter()
            .map(|extent| extent.end_saturating().get())
            .max()
            .unwrap_or(0);
        // Reaching past everything kept so far cannot be covered by it: the
        // common case, and the cheapest possible answer.
        let covered = end <= furthest_end
            && finding.parent.is_none()
            && by_reach
                .iter()
                .take_while(|(Reverse(reach), _)| *reach >= end)
                .any(|&(_, index)| {
                    let container = &kept[index];
                    container.confidence >= finding.confidence
                        && container.extents != finding.extents
                        && finding.is_covered_by(container)
                });
        if !covered {
            furthest_end = furthest_end.max(end);
            by_reach.insert((Reverse(end), kept.len()));
            kept.push(finding);
        }
    }
    *findings = kept;
    dropped_unreadable
}

/// The part of `finding` that lies before the first damage it meets.
///
/// Damage is recorded at the granularity a failed read is retried at, so one
/// bad sector condemns a whole span around it. A photograph reaching into that
/// span still has extents that read cleanly and are, byte for byte, the start
/// of the picture.
///
/// So the head is kept and the rest is not. What comes back is the medium's
/// own bytes up to the damage, at the weakest tier and with the length the
/// metadata expected recorded beside it, exactly as a decoder-truncated
/// recovery is. Nothing is padded and no zero is ever presented as data
/// (`A-CONFIDENCE-HONEST`).
///
/// `None` when the damage starts at or before the first extent: there is no
/// head, only a finding whose bytes are unknown.
fn head_before_damage(finding: &Finding, unreadable: &[ByteRange]) -> Option<Finding> {
    let first_damage = unreadable
        .iter()
        .filter(|range| finding.intersects(**range))
        .map(|range| range.start.get())
        .min()?;

    let mut extents = Vec::new();
    for extent in &finding.extents {
        let start = extent.start.get();
        if start >= first_damage {
            break;
        }
        let len = extent.len.min(first_damage.saturating_sub(start));
        if len == 0 {
            break;
        }
        extents.push(ByteRange::new(extent.start, len));
        // A later extent may sit before the damage on the medium while a
        // earlier one already reached it; stopping here keeps the head a
        // prefix of the *file* rather than of the disk.
        if len < extent.len {
            break;
        }
    }
    if extents.is_empty() {
        return None;
    }

    Some(Finding {
        format: finding.format,
        stage: finding.stage,
        // Never above the weakest tier: what this describes is a file that
        // stops, and how it was found does not change that.
        confidence: argos_core::Confidence::PartialOrThumbnail,
        extents: extents.into_boxed_slice(),
        // What the file was supposed to be, so the shortfall is stated rather
        // than the head presented as whole.
        declared_size: finding.declared_size.or_else(|| Some(finding.length())),
        timestamps: finding.timestamps,
        deleted: finding.deleted,
        name: finding.name.clone(),
        source_object: finding.source_object,
        parent: finding.parent,
    })
}

/// Artifacts that must share dimensions in a row before the run is a cache.
///
/// Two pictures of one size are a coincidence and three are a set; a cache
/// holds hundreds. The threshold sits low enough to catch a small one and high
/// enough that a burst of photographs from one camera — which vary in
/// orientation, and whose sizes therefore alternate — never reaches it.
const MIN_RUN: usize = 8;

/// How far apart two entries of one cache may sit and still be one run.
///
/// Entries of a cache file are consecutive; the slack is for the records
/// between them and for entries a scan could not recover.
const MAX_GAP_BYTES: u64 = 4 * 1024 * 1024;

/// One artifact, as this pass needs to see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Where the artifact starts on the medium.
    pub offset: u64,
    /// Its decoded dimensions, when it decoded.
    pub pixels: Option<(u32, u32)>,
    /// Content hash, which is how the manifest is told about it.
    pub sha256: Digest,
}

/// One artifact found among same-sized neighbours, and how many there were.
///
/// A desktop or a phone keeps previews of every picture it has shown, in one
/// file, written once. That file survives long after the photographs it
/// describes are overwritten, so a recovery of a used disk turns up far more
/// cache entries than photographs — and each one looks, on its own, exactly
/// like a small photograph, because that is what it is a copy of.
///
/// What gives a cache away is not any one entry but the run: a cache writes
/// one size, so its entries share dimensions to the pixel and sit next to each
/// other. Measured on a 1 TB disk of ten years' use, 51 of 60 artifacts within
/// four megabytes of one offset were exactly 256x192.
///
/// Naming it is what stops the report from presenting a preview of a lost
/// photograph as the photograph (`A-CONFIDENCE-HONEST`). Nothing here removes
/// or reclassifies anything: it counts neighbours and says how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheRun {
    /// Content hash of the artifact, which is how a manifest is told.
    pub sha256: Digest,
    /// How many artifacts of identical dimensions the run held, this one
    /// included. A large number is a thumbnail cache; there is no size at
    /// which it becomes a verdict about any single picture.
    pub neighbours: u32,
}

/// Every artifact that belongs to a run of same-sized neighbours, with the
/// size of the run it belongs to.
///
/// `entries` are expected in medium order, which is the order the report stage
/// produces them in.
pub(crate) fn runs(entries: &[Entry]) -> Vec<CacheRun> {
    let mut found = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let Some(pixels) = entries[start].pixels else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < entries.len()
            && entries[end].pixels == Some(pixels)
            && entries[end].offset.saturating_sub(entries[end - 1].offset) <= MAX_GAP_BYTES
        {
            end += 1;
        }
        let length = end - start;
        if length >= MIN_RUN {
            let size = u32::try_from(length).unwrap_or(u32::MAX);
            found.extend(entries[start..end].iter().map(|entry| CacheRun {
                sha256: entry.sha256,
                neighbours: size,
            }));
        }
        start = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{Entry, MIN_RUN, runs};
    use argos_core::ports::Digest;

    fn entry(offset: u64, pixels: Option<(u32, u32)>, tag: u64) -> Entry {
        Entry {
            offset,
            pixels,
            sha256: Digest::new([u8::try_from(tag % 256).unwrap_or(0); Digest::LEN]),
        }
    }

    #[test]
    fn a_run_of_identical_sizes_is_named_with_its_length() {
        // The shape measured on a real disk: a stretch of one size, packed.
        let entries: Vec<_> = (0..40)
            .map(|index| entry(1_000_000 + index * 8_000, Some((256, 192)), index))
            .collect();
        let found = runs(&entries);
        assert_eq!(found.len(), 40, "every entry of the run is named");
        assert!(found.iter().all(|run| run.neighbours == 40));
    }

    #[test]
    fn a_handful_of_one_size_is_not_a_cache() {
        let entries: Vec<_> = (0..MIN_RUN as u64 - 1)
            .map(|index| entry(index * 5_000, Some((640, 480)), index))
            .collect();
        assert!(
            runs(&entries).is_empty(),
            "a few photographs of one size are a coincidence, not a cache"
        );
    }

    #[test]
    fn photographs_between_two_caches_are_left_alone() {
        let mut entries: Vec<_> = (0..12)
            .map(|index| entry(index * 4_000, Some((258, 258)), index))
            .collect();
        entries.push(entry(60_000, Some((4128, 3096)), 200));
        entries.extend(
            (0..10).map(|index| entry(80_000 + index * 4_000, Some((258, 258)), 100 + index)),
        );

        let found = runs(&entries);
        let named: Vec<_> = found.iter().map(|run| run.sha256).collect();
        assert_eq!(
            found.len(),
            22,
            "both runs are named, the photograph is not"
        );
        assert!(
            !named.contains(&Digest::new([200; Digest::LEN])),
            "the camera frame between two caches is not part of either"
        );
    }

    #[test]
    fn a_distant_neighbour_starts_a_new_run() {
        let mut entries: Vec<_> = (0..10)
            .map(|index| entry(index * 1_000, Some((96, 96)), index))
            .collect();
        // Far enough away to be another file entirely.
        entries.extend((0..3).map(|index| {
            entry(
                500 * 1024 * 1024 + index * 1_000,
                Some((96, 96)),
                50 + index,
            )
        }));
        let found = runs(&entries);
        assert_eq!(
            found.len(),
            10,
            "the three on their own are not a run: {found:?}"
        );
    }

    #[test]
    fn an_artifact_that_did_not_decode_belongs_to_no_run() {
        let mut entries: Vec<_> = (0..10)
            .map(|index| entry(index * 1_000, Some((64, 64)), index))
            .collect();
        entries.insert(5, entry(4_500, None, 99));
        let found = runs(&entries);
        assert!(
            !found
                .iter()
                .any(|run| run.sha256 == Digest::new([99; Digest::LEN])),
            "a size nobody measured cannot match a size"
        );
    }
}
