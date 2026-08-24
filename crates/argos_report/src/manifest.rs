//! The scan manifest: what a session recovered, in machine-readable form.
//!
//! Anything other than the scan that produced a session reads its recovery from
//! here — the export command, the report command and the engine's IPC surface
//! all parse this rather than keeping a second account of the same facts.
//!
//! The manifest is deliberate report output and may carry a recovered file
//! name; log and error messages never do (A-NO-CONTENT-IN-LOGS).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ReportError;

/// Name of the manifest file inside the output directory.
pub(crate) const MANIFEST_FILE: &str = "manifest.json";

/// The scan manifest: tool identity, source description and every artifact.
#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Version of the tool that produced this manifest.
    pub tool_version: String,
    /// User-supplied description of the scanned source (path or label).
    pub source: String,
    /// How the run ended: `finished`, `cancelled`, or `failed`. A manifest is
    /// written for every outcome, so artifacts already on disk are never left
    /// unattributed (A-PROVENANCE).
    pub scan_state: String,
    /// Signature hits that failed validation and were not recovered.
    pub rejected_candidates: u64,
    /// Byte ranges the medium could not read; their contents are unknown and
    /// were never fabricated.
    pub unreadable: Vec<ExtentRecord>,
    /// How triage ran over this scan, when the caller reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageRecord>,
    /// What the run reached and what it left behind, when the caller reported
    /// it. This is what separates "the medium held nothing more" from "the run
    /// did not look" (A-CONFIDENCE-HONEST).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageRecord>,
    /// Volumes the run located, current and residual.
    ///
    /// A medium re-formatted more than once carries the anchors of the
    /// filesystems that came before, and which of them were found is what
    /// decides whether their metadata could be read at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<VolumeRecord>,
    /// One record per fragmentation point carving localized.
    ///
    /// Where the search can be picked up without sweeping the medium again.
    /// Locating these is what a scan's expensive stages produce; searching from
    /// them is minutes where the scan was hours, which is what lets a search be
    /// run again with a longer budget or a lower floor.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragmentation: Vec<FragmentRecord>,
    /// Files a surviving metadata record names, whose content the run could
    /// not place.
    ///
    /// Held apart from `artifacts` and never counted among them: nothing here
    /// was read from the medium and no extent is claimed. A re-format destroys
    /// the boot sector that says where a volume began, and without it a run
    /// list — which counts clusters of that volume — locates nothing. The
    /// record's own name, size and times survive it, and they are the only
    /// evidence left that a particular file was ever there
    /// (`A-CONFIDENCE-HONEST`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lost_files: Vec<LostFileRecord>,
    /// One record per recovered artifact.
    pub artifacts: Vec<ArtifactRecord>,
}

/// What a run reached, and what it stopped short of.
///
/// Every field here is a count the run already keeps; recording them is what
/// makes the difference between a recovery that failed and one that was never
/// attempted answerable after the fact, from the manifest alone. Without them
/// the only account of a scan's own reach is its console output, which a
/// window discards.
///
/// Plain numbers rather than the engine's own vocabulary, like every other
/// record here: this crate writes the manifest and depends on nothing that
/// recovers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageRecord {
    /// Bytes of the medium the sweep covered.
    pub bytes_swept: u64,
    /// Findings dropped because an identical content hash was already stored.
    pub duplicates: u64,
    /// Findings whose claimed bytes could not be read back, so they were
    /// dropped rather than reported from whatever was there.
    pub unrecoverable: u64,
    /// Findings dropped because their bytes lie in a range the medium refused.
    ///
    /// Their content is unknown and nothing was fabricated for it, but the
    /// signature that started them was real: a high count here means damage
    /// cost recoveries, not that the medium held nothing.
    pub dropped_unreadable: u64,
    /// Artifacts recognised, recorded and deliberately not written because
    /// they fell under the run's size floor. Each is in `artifacts` with its
    /// extents and dimensions, so a rerun with a lower floor produces them.
    pub omitted_assets: u64,
    /// Images reported as the part of themselves that decodes.
    pub partial_prefixes: u64,
    /// Broken candidates reassembly was offered.
    pub reassembly_attempted: u64,
    /// Images recovered by reassembling fragments.
    pub reassembled: u64,
    /// Broken candidates the search left alone because the frame declares a
    /// picture below the size floor. Not lost — a run with a lower floor
    /// searches them.
    pub reassembly_skipped_small: u64,
    /// Deletion events read from the volumes' change journals.
    ///
    /// Names and moments, never extents. A run of artifacts sharing one
    /// `deleted_unix` is a batch deletion — files removed in one action, which
    /// nothing else on an NTFS volume records.
    pub journal_deletions: u64,
    /// Residual `FILE`-record regions that could not be attributed to a
    /// located volume, so their extents could not be resolved.
    ///
    /// These are run lists — the exact map of a deleted file's fragments —
    /// that survived a re-format and could not be read for want of the volume
    /// geometry they are counted against. They are counted, never guessed at.
    pub unattributed_residue: u64,
    /// Ceilings the run reached, named. Each means it looked at less than it
    /// set out to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ceilings: Vec<String>,
}

/// One filesystem volume a run located.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeRecord {
    /// Filesystem family detected at the anchor, as its canonical name.
    pub kind: String,
    /// `current` when the partition table lists it, `residual` when only the
    /// residue sweep found it — the anchor of a filesystem an earlier format
    /// left behind.
    pub origin: String,
    /// Where the volume starts on the medium.
    pub offset: u64,
    /// Length the anchor claims, capped at the medium.
    pub length: u64,
    /// Bytes in the unit this filesystem allocates in. Zero when the anchor
    /// did not state a usable one.
    pub allocation_bytes: u64,
}

/// One image that started decoding on the medium and stopped.
///
/// Plain numbers rather than the engine's own vocabulary: this crate is what
/// writes the manifest and depends on nothing that recovers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRecord {
    /// Where the image starts on the medium.
    pub offset: u64,
    /// Where the stream stopped being this image.
    pub break_at: u64,
    /// First byte past the last part of the picture that decoded whole.
    pub decoded_end: u64,
    /// Image format, as its canonical display name.
    pub format: String,
    /// Pixel width the frame header declares, when it declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_width: Option<u32>,
    /// Pixel height the frame header declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_height: Option<u32>,
    /// Units of the picture that decoded, and how many it needs.
    pub decoded: u32,
    /// Units the whole picture requires.
    pub required: u32,
}

/// A file a surviving metadata record still names, and nothing more.
///
/// Deliberately without an extents field: there is no honest value for one.
/// What is here is what a `FILE` record states about itself, none of which
/// needs the volume's geometry — plus the run list in the volume's own units,
/// which is what lets a later run test a candidate geometry against it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostFileRecord {
    /// Name the record carries, when one survived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Content length in bytes the record claims.
    pub size: u64,
    /// Where the record itself lay — where the lost `$MFT` was.
    pub record_at: u64,
    /// Creation time, as Unix seconds, when the record carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_unix: Option<i64>,
    /// Last modification time, as Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix: Option<i64>,
    /// First logical cluster of the content, in the lost volume's units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_cluster: Option<u64>,
    /// Clusters the run list accounts for, in the lost volume's units.
    ///
    /// With `size`, this is a test of any candidate cluster size: the right
    /// one is the one whose clusters account for the size stated.
    pub clusters: u64,
}

impl Manifest {
    /// Reads the manifest of the session directory `dir`.
    ///
    /// This is how anything other than the scan that produced it learns what a
    /// session recovered — the export command, the report command, and the
    /// engine's IPC surface all read it rather than keeping a second account
    /// of the same facts.
    ///
    /// # Errors
    ///
    /// Fails when the manifest is missing, unreadable, or not the JSON this
    /// tool writes.
    pub fn read(dir: impl AsRef<Path>) -> Result<Self, ReportError> {
        let path = dir.as_ref().join(MANIFEST_FILE);
        let bytes = fs::read(&path).map_err(|source| ReportError::new(&path, source))?;
        serde_json::from_slice(&bytes)
            .map_err(|source| ReportError::new(&path, io::Error::other(source)))
    }

    /// Writes this manifest into the directory `dir`, returning its path.
    ///
    /// Used where a manifest is assembled from records rather than from a
    /// scan — an export describing exactly the artifacts that landed in its
    /// destination, for instance. The format is the one [`Store::finish`](crate::Store::finish)
    /// writes, because there is only one.
    ///
    /// # Errors
    ///
    /// Fails when the manifest cannot be serialized or written.
    pub fn write(&self, dir: impl AsRef<Path>) -> Result<PathBuf, ReportError> {
        let path = dir.as_ref().join(MANIFEST_FILE);
        let json = serde_json::to_vec_pretty(self)
            .map_err(|source| ReportError::new(&path, io::Error::other(source)))?;
        fs::write(&path, json).map_err(|source| ReportError::new(&path, source))?;
        Ok(path)
    }
}

/// How ML triage ran over a scan.
///
/// Recorded whatever happened: a disabled triage is stated with its reason,
/// never silently absent, so the absence of scores is attributable
/// (A-MODEL-PINNED).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TriageRecord {
    /// `scored` when triage ran, `disabled` when it did not.
    pub status: String,
    /// Why triage did not run, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    /// Version of the decision procedure that labelled the scan, which is what
    /// makes a label reproducible (A-MODEL-PINNED).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
    /// Artifacts that received a score.
    pub scored: u64,
    /// Artifacts triage saw but could not score.
    pub unscored: u64,
    /// Whether the classifier failed mid-run, leaving artifacts unscored that
    /// a healthy one would have scored.
    pub degraded: bool,
}

/// Triage annotation for one stored artifact, matched by content hash.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageAnnotation {
    /// SHA-256 (lowercase hex) of the artifact the annotation belongs to.
    pub sha256: String,
    /// Perceptual hash of the decoded image, as 16 hex digits.
    pub perceptual_hash: Option<String>,
    /// SHA-256 of the artifact this one is a near-duplicate of.
    pub near_duplicate_of: Option<String>,
    /// Triage label.
    pub label: Option<String>,
    /// The property that settled the label.
    pub decided_by: Option<String>,
}

/// One contiguous source range, as recorded in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExtentRecord {
    /// Absolute byte offset of the range's first byte in the source.
    pub offset: u64,
    /// Range length in bytes.
    pub length: u64,
}

/// Provenance record of one recovered artifact.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ArtifactRecord {
    /// File name of the artifact inside the output directory, when its bytes
    /// were stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Recovery stage that produced the artifact.
    pub stage: String,
    /// Image format the artifact validated as.
    pub format: String,
    /// Absolute byte offset of the artifact's first byte in the source.
    pub source_offset: u64,
    /// Artifact length in bytes: what was actually recovered and stored.
    pub length: u64,
    /// Width of the decoded picture, in pixels, when it decoded.
    ///
    /// The property that separates a photograph from the derived images a
    /// used medium is full of, and the one a byte count cannot stand in for.
    /// Absent means the artifact did not decode here — a statement about the
    /// decoder, not about the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Height of the decoded picture, on the same terms as [`width`].
    ///
    /// [`width`]: ArtifactRecord::width
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Camera manufacturer the picture records about itself.
    ///
    /// This and the three fields below are how a person finds their own
    /// photographs among a used disk's hundreds of thousands of recovered
    /// images: an offset and a byte count separate nothing, while a camera and
    /// a date separate one afternoon from ten years of everything else. They
    /// survive a picture that does not, because they sit ahead of its data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_make: Option<String>,
    /// Camera model the picture records about itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_model: Option<String>,
    /// When the picture was taken, as stored: `YYYY:MM:DD HH:MM:SS`. Verbatim,
    /// because it carries no zone and turning it into an instant would be
    /// inventing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken: Option<String>,
    /// Pixel width the picture's own metadata claims, which may differ from
    /// the decoded [`width`] and survives when the picture does not.
    ///
    /// [`width`]: ArtifactRecord::width
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_width: Option<u32>,
    /// Pixel height the picture's own metadata claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_height: Option<u32>,
    /// Length the source metadata claimed, when it said one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_length: Option<u64>,
    /// Bytes the metadata expected that were not recovered. Absent when the
    /// recovery is whole; present and non-zero states the truncation plainly
    /// (A-CONFIDENCE-HONEST).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_bytes: Option<u64>,
    /// Evidence tier, as its canonical display name.
    pub confidence: String,
    /// Every source extent the artifact was assembled from, in file order.
    pub extents: Vec<ExtentRecord>,
    /// Creation time recovered from metadata, in seconds since the Unix
    /// epoch. Never inferred; absent when the filesystem did not record one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_unix: Option<i64>,
    /// Last modification time recovered from metadata, in seconds since the
    /// Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix: Option<i64>,
    /// When a change journal recorded this file being deleted, in seconds
    /// since the Unix epoch. Absent when no journal named it.
    ///
    /// The only timestamp about the *removal* rather than about the file. A
    /// run of artifacts sharing this moment is a batch deletion — files
    /// removed in one action, which nothing else on a volume records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_unix: Option<i64>,
    /// File name recovered from filesystem metadata, when one survived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovered_name: Option<String>,
    /// Filesystem object the metadata came from — MFT record number, inode
    /// number or first cluster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_object: Option<u64>,
    /// For embedded thumbnails, the source offset of the parent candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_offset: Option<u64>,
    /// SHA-256 of the artifact bytes, computed while writing them.
    pub sha256: String,
    /// Triage label, when the artifact was scored. A label orders and groups;
    /// every artifact stays in this manifest whatever it says
    /// (A-TRIAGE-NOT-VERDICT).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_label: Option<String>,
    /// The property that settled the label, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_decided_by: Option<String>,
    /// Whether the artifact's bytes were stored in the output directory.
    ///
    /// False when the artifact did not clear the run's size floor. The record
    /// still describes it completely — extents, digest, dimensions — so the
    /// account of what the medium held stays whole either way, and the extents
    /// locate the bytes exactly for a rerun with a lower floor.
    ///
    /// `argos export` reads the session directory, so it cannot produce these:
    /// they have no file there. Getting them is a rerun of the scan.
    #[serde(default = "stored_by_default")]
    pub written: bool,
    /// Why the bytes were not stored, when they were not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_because: Option<String>,
    /// How many artifacts of identical dimensions this one was found among,
    /// consecutively, when it was found among any.
    ///
    /// The signature of a thumbnail cache: a cache writes one size and writes
    /// it in one place, so its entries share dimensions to the pixel. A large
    /// number here says the artifact is a preview of a picture, which may or
    /// may not itself have survived — and saying that is what stops a report
    /// presenting the preview as the picture (A-CONFIDENCE-HONEST). It is a
    /// count of neighbours, never a verdict about this artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_size_neighbours: Option<u32>,
    /// Where this artifact stands in a list, named by the strongest fact that
    /// put it there.
    ///
    /// A sort key, and only that. A recovery of a used disk writes hundreds of
    /// thousands of artifacts and a few hundred photographs; without an order
    /// the photographs are present and unreachable. Every stored artifact
    /// carries one, the weakest is still one, and nothing is removed or hidden
    /// by it (A-TRIAGE-NOT-VERDICT).
    ///
    /// Derived from fields recorded beside it — dimensions, camera, capture
    /// date, same-size neighbours — so it can be recomputed from this record
    /// alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<String>,
    /// Perceptual hash of the decoded image, 16 hex digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perceptual_hash: Option<String>,
    /// SHA-256 of the artifact this one is a near-duplicate of; both stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_duplicate_of: Option<String>,
    /// Path of this artifact's preview, relative to the output directory.
    ///
    /// Derived presentation, reproducible from the artifact at any time.
    /// Absent when previews were not requested, or when this artifact did not
    /// decode into one — which says nothing about the recovery itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// A manifest written before this field existed described stored artifacts
/// only, so an absent flag means the bytes are there.
const fn stored_by_default() -> bool {
    true
}
