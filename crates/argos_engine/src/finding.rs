//! What the pipeline produces before it becomes an artifact.

use std::fmt;

use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::progress::RunState;
use argos_core::{Confidence, Format, Stage, Timestamps};
use argos_fs::Volume;

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
            .field("name", &self.name.as_ref().map(|_| "<redacted>"))
            .field("source_object", &self.source_object)
            .field("parent", &self.parent)
            .finish()
    }
}

/// What a completed — or cancelled — scan found.
///
/// Everything here is countable evidence about the run itself; recovered
/// content leaves through the [`ArtifactSink`](argos_core::artifact::ArtifactSink).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// Bytes of the medium the sweep covered.
    pub bytes_swept: u64,
    /// Ranges the medium refused to read. Their content is unknown; nothing
    /// was fabricated for them and no artifact overlaps one.
    pub unreadable: Vec<ByteRange>,
    /// Volumes located, current and residual.
    pub volumes: Vec<Volume>,
    /// Whether a detector hit its cap and stopped collecting, so the scan
    /// covered the surface but did not report everything it saw. A medium that
    /// does this is patterned, deliberately or otherwise.
    pub detection_truncated: bool,
    /// Residual `FILE`-record regions that could not be attributed to an NTFS
    /// volume, so their extents could not be resolved. They are counted, not
    /// guessed at.
    pub unattributed_residue: u64,
}

impl ScanReport {
    /// Whether the run covered everything it was asked to cover.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.state == RunState::Finished && self.unreadable.is_empty() && !self.detection_truncated
    }
}
