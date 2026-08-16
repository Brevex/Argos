//! What the pipeline produces before it becomes an artifact.

use std::fmt;

use argos_core::classify::ModelIdentity;
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::progress::RunState;
use argos_core::{Confidence, Format, Stage, Timestamps};
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
    pub fn is_covered_by(&self, other: &Self) -> bool {
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
    pub fn intersects(&self, range: ByteRange) -> bool {
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
/// content leaves through the [`ArtifactSink`](argos_core::artifact::ArtifactSink).
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
    /// Each also carries its run list in the volume's own units, which is what
    /// makes a candidate geometry testable afterwards: the right cluster size
    /// is the one whose clusters account for the size the record states.
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
    pub standings: Vec<(argos_core::artifact::Digest, argos_classify::rank::Standing)>,
    /// Artifacts found among same-sized neighbours, with how many there were.
    ///
    /// The signature of a thumbnail cache, which a used medium holds far more
    /// of than photographs. It is a count of neighbours and nothing more: no
    /// artifact is removed, reclassified or ranked by it
    /// (A-CONFIDENCE-HONEST).
    pub cache_runs: Vec<crate::cache_run::CacheRun>,
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
