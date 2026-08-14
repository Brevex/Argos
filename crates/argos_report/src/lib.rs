//! Recovery output: artifact files, SHA-256 hashes and the scan manifest.
//!
//! [`Store`] is the [`ArtifactSink`] adapter that owns one output directory.
//! Every artifact saved through it is streamed to disk while its SHA-256 is
//! recomputed and checked against the digest the engine recorded at recovery
//! time, and everything ends up in a machine-readable `manifest.json` carrying
//! full provenance: the exact source extents, the stage that recovered them,
//! the evidence tier and the hash (A-PROVENANCE).
//!
//! The manifest is deliberate report output and may carry a recovered file
//! name; log and error messages never do (A-NO-CONTENT-IN-LOGS).

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use argos_core::artifact::{Artifact, ArtifactSink};
use argos_core::classify::PixelImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod handback;
mod preview;

pub use handback::{Handback, Owner};

/// Name of the manifest file inside the output directory.
const MANIFEST_FILE: &str = "manifest.json";

/// Subdirectory of the output holding preview images.
///
/// Separate from the artifacts on purpose: it is the one part of a session
/// directory that holds derived rather than recovered bytes, so a viewer can
/// be given access to it without being given access to the evidence
/// (`A-DTO-VERSIONED`).
pub const PREVIEW_DIR: &str = "previews";

/// Bytes copied per streaming step while hashing and writing an artifact.
/// 64 KiB balances syscall count against buffer size.
const COPY_CHUNK_BYTES: usize = 64 * 1024;

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
    /// One record per fragmentation point carving localized.
    ///
    /// Where the search can be picked up without sweeping the medium again.
    /// Locating these is what a scan's expensive stages produce; searching from
    /// them is minutes where the scan was hours, which is what lets a search be
    /// run again with a longer budget or a lower floor.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragmentation: Vec<FragmentRecord>,
    /// One record per recovered artifact.
    pub artifacts: Vec<ArtifactRecord>,
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
    /// destination, for instance. The format is the one [`Store::finish`]
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

/// Writes artifacts and the manifest into one output directory.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    records: Vec<ArtifactRecord>,
    /// Who every file written here is given to, once it is written. `None`
    /// when nobody was named or the directory could not be handed over.
    owner: Option<Owner>,
    /// What happened when the directory itself was handed over, for the caller
    /// to report.
    handback: Handback,
    /// Reused streaming buffer so saving many artifacts does not allocate per
    /// artifact.
    copy_buf: Vec<u8>,
}

impl Store {
    /// Creates `dir` (and parents) and an empty store over it.
    ///
    /// `owner` is the account the recovered files belong to. A scan of a raw
    /// device runs elevated, so without it every file would be created by the
    /// administrator and the person who asked for the recovery could not use
    /// what came back. Whether that worked is [`Store::handback`], and a
    /// refusal is a warning rather than a failure: the bytes are recovered
    /// either way.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created.
    pub fn create(dir: impl AsRef<Path>, owner: Option<Owner>) -> Result<Self, ReportError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).map_err(|source| ReportError::new(dir, source))?;
        let handback = Handback::attempt(dir, owner);
        Ok(Self {
            dir: dir.to_path_buf(),
            records: Vec::new(),
            owner: handback.owner(owner),
            handback,
            copy_buf: Vec::new(),
        })
    }

    /// The directory this store writes into.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the output directory could be given to the account that asked.
    #[must_use]
    pub fn handback(&self) -> &Handback {
        &self.handback
    }

    /// Gives one file this store just created to the account that asked.
    ///
    /// Failures are dropped on purpose: the directory was handed over a moment
    /// ago on the same filesystem, so a file that refuses is a surprise with
    /// nowhere useful to be reported — and losing a recovered artifact over
    /// its ownership would be the wrong trade in a recovery tool.
    fn give(&self, path: &Path) {
        if let Some(owner) = self.owner {
            let _ = owner.give(path);
        }
    }

    /// Records saved so far, in save order.
    #[must_use]
    pub fn records(&self) -> &[ArtifactRecord] {
        &self.records
    }

    /// Streams `bytes` into the output directory and records the artifact.
    ///
    /// The file is named `<index>.<extension>` from the artifact's format and
    /// its position in the store. While writing, the SHA-256 is recomputed and
    /// compared with [`Artifact::sha256`], and the byte count with
    /// [`Artifact::length`]: an artifact that does not reproduce both is
    /// refused rather than recorded as recovered.
    ///
    /// # Errors
    ///
    /// Fails when the artifact file cannot be created or written, when reading
    /// `bytes` fails, or when the bytes do not match the recorded length or
    /// digest.
    pub fn save<R: Read + ?Sized>(
        &mut self,
        artifact: &Artifact<'_>,
        bytes: &mut R,
    ) -> Result<&ArtifactRecord, ReportError> {
        let name = format!("{:06}.{}", self.records.len(), artifact.format.extension());
        let path = self.dir.join(&name);
        let into_err = |source: io::Error| ReportError::new(&path, source);

        let mut out = BufWriter::new(File::create(&path).map_err(into_err)?);
        let mut hasher = Sha256::new();
        self.copy_buf.resize(COPY_CHUNK_BYTES, 0);
        let mut written = 0_u64;
        loop {
            let n = bytes.read(&mut self.copy_buf).map_err(into_err)?;
            if n == 0 {
                break;
            }
            hasher.update(&self.copy_buf[..n]);
            out.write_all(&self.copy_buf[..n]).map_err(into_err)?;
            written += n as u64;
        }
        out.flush().map_err(into_err)?;
        if written != artifact.length {
            // The file on disk is not the artifact that was validated. Leaving
            // it would put bytes in the output directory that no manifest
            // record describes (A-PROVENANCE).
            drop(out);
            let _ = fs::remove_file(&path);
            return Err(into_err(io::Error::other(format!(
                "short artifact: expected {} bytes from the source, got {written}",
                artifact.length
            ))));
        }
        let sha256 = hex(&hasher.finalize());
        let recorded = artifact.sha256.to_string();
        if sha256 != recorded {
            drop(out);
            let _ = fs::remove_file(&path);
            return Err(into_err(io::Error::other(format!(
                "artifact hash changed between recovery and storage: recovered {recorded}, \
                 stored {sha256}"
            ))));
        }

        drop(out);
        self.give(&path);

        self.records.push(ArtifactRecord {
            name: Some(name),
            stage: artifact.stage.to_string(),
            format: artifact.format.to_string(),
            source_offset: artifact
                .extents
                .first()
                .map_or(0, |first| first.start.get()),
            length: written,
            width: artifact.pixels.map(|(width, _)| width),
            height: artifact.pixels.map(|(_, height)| height),
            camera_make: artifact.capture.make.clone(),
            camera_model: artifact.capture.model.clone(),
            taken: artifact.capture.taken.clone(),
            declared_width: artifact.capture.pixels.map(|(width, _)| width),
            declared_height: artifact.capture.pixels.map(|(_, height)| height),
            expected_length: artifact.expected_length,
            missing_bytes: artifact
                .expected_length
                .map(|expected| expected.saturating_sub(written))
                .filter(|missing| *missing > 0),
            confidence: artifact.confidence.to_string(),
            extents: artifact
                .extents
                .iter()
                .map(|extent| ExtentRecord {
                    offset: extent.start.get(),
                    length: extent.len,
                })
                .collect(),
            created_unix: artifact.timestamps.created.map(unix_seconds),
            modified_unix: artifact.timestamps.modified.map(unix_seconds),
            recovered_name: artifact.recovered_name.map(str::to_owned),
            source_object: artifact.source_object,
            parent_offset: artifact.parent.map(argos_core::geometry::ByteOffset::get),
            sha256,
            same_size_neighbours: None,
            triage_label: None,
            triage_decided_by: None,
            written: true,
            omitted_because: None,
            perceptual_hash: None,
            near_duplicate_of: None,
            preview: None,
        });
        Ok(self
            .records
            .last()
            .unwrap_or_else(|| unreachable!("a record was just pushed")))
    }

    /// Joins triage annotations onto the records already saved, by content
    /// hash.
    ///
    /// Strictly an annotation: a record without an annotation stays as it is,
    /// an annotation without a record is dropped, and nothing here can remove
    /// or reorder a record. Triage output has no other way into the manifest
    /// (A-TRIAGE-NOT-VERDICT).
    pub fn annotate_triage(&mut self, annotations: &[TriageAnnotation]) {
        for annotation in annotations {
            // Linear match: artifact counts are the size of a photo library,
            // and this runs once per scan.
            let Some(record) = self
                .records
                .iter_mut()
                .find(|record| record.sha256 == annotation.sha256)
            else {
                continue;
            };
            record.triage_label.clone_from(&annotation.label);
            // Rounded, because the score comes from a softmax over a
            // transcendental and `exp` is a libm implementation detail: the
            // last digits of a probability differ between operating systems
            // for no substantive reason, and a manifest that differs between
            // machines undermines the reproducibility the rest of it exists
            // for. Four decimals is far finer than the label thresholds.
            record.triage_decided_by.clone_from(&annotation.decided_by);
            record
                .perceptual_hash
                .clone_from(&annotation.perceptual_hash);
            record
                .near_duplicate_of
                .clone_from(&annotation.near_duplicate_of);
        }
    }

    /// Records an artifact whose bytes were deliberately not stored.
    ///
    /// The record is the same in every respect a stored one is — extents,
    /// digest, format, confidence, provenance — except that it names no file
    /// and says so. That is what keeps the manifest a complete account of the
    /// medium while the directory holds only what the caller asked for: the
    /// bytes are still at the extents recorded here, which locate them on the
    /// source exactly.
    pub fn record_only(&mut self, artifact: &Artifact<'_>, reason: &str) {
        self.records.push(ArtifactRecord {
            name: None,
            stage: artifact.stage.to_string(),
            format: artifact.format.to_string(),
            source_offset: artifact
                .extents
                .first()
                .map_or(0, |first| first.start.get()),
            length: artifact.length,
            width: artifact.pixels.map(|(width, _)| width),
            height: artifact.pixels.map(|(_, height)| height),
            camera_make: artifact.capture.make.clone(),
            camera_model: artifact.capture.model.clone(),
            taken: artifact.capture.taken.clone(),
            declared_width: artifact.capture.pixels.map(|(width, _)| width),
            declared_height: artifact.capture.pixels.map(|(_, height)| height),
            expected_length: artifact.expected_length,
            missing_bytes: artifact
                .expected_length
                .map(|expected| expected.saturating_sub(artifact.length))
                .filter(|missing| *missing > 0),
            confidence: artifact.confidence.to_string(),
            extents: artifact
                .extents
                .iter()
                .map(|extent| ExtentRecord {
                    offset: extent.start.get(),
                    length: extent.len,
                })
                .collect(),
            created_unix: artifact.timestamps.created.map(unix_seconds),
            modified_unix: artifact.timestamps.modified.map(unix_seconds),
            recovered_name: artifact.recovered_name.map(str::to_owned),
            source_object: artifact.source_object,
            parent_offset: artifact.parent.map(argos_core::geometry::ByteOffset::get),
            sha256: artifact.sha256.to_string(),
            written: false,
            triage_label: None,
            triage_decided_by: None,
            same_size_neighbours: None,
            perceptual_hash: None,
            near_duplicate_of: None,
            preview: None,
            omitted_because: Some(reason.to_owned()),
        });
    }

    /// Records how many same-sized neighbours each named artifact was found
    /// among.
    ///
    /// Annotation only, like the triage labels: it adds a fact about the
    /// medium's layout to records already written and can remove nothing.
    pub fn annotate_same_size_runs(&mut self, runs: &[(String, u32)]) {
        for (sha256, neighbours) in runs {
            if let Some(record) = self
                .records
                .iter_mut()
                .find(|record| record.sha256 == *sha256)
            {
                record.same_size_neighbours = Some(*neighbours);
            }
        }
    }

    /// Writes a preview of an artifact already saved, named by its hash.
    ///
    /// Strictly additive, exactly like [`annotate_triage`](Store::annotate_triage):
    /// a preview for an artifact this store never saved is dropped, and no
    /// record is created, removed or reordered by one. An artifact with no
    /// preview is an artifact with no thumbnail — nothing more.
    ///
    /// # Errors
    ///
    /// Fails when the preview directory or the preview file cannot be written.
    /// The artifact itself is already stored and recorded by then, so a caller
    /// that counts the failure and continues loses a thumbnail rather than
    /// evidence.
    pub fn save_preview(
        &mut self,
        sha256: &argos_core::artifact::Digest,
        image: &PixelImage,
    ) -> Result<(), ReportError> {
        let hash = sha256.to_string();
        let Some(index) = self.records.iter().position(|record| record.sha256 == hash) else {
            return Ok(());
        };
        // An image the encoder declines is one with no pixels to show. That is
        // not a failure of the output directory, and reporting it as one would
        // make a caller count it against the medium.
        let Some(encoded) = preview::encode(image) else {
            return Ok(());
        };

        let dir = self.dir.join(PREVIEW_DIR);
        fs::create_dir_all(&dir).map_err(|source| ReportError::new(&dir, source))?;
        // Named by content hash, so the manifest joins previews to records the
        // same way it joins triage annotations, and a rerun overwrites rather
        // than accumulating.
        let name = format!("{hash}.jpg");
        let path = dir.join(&name);
        fs::write(&path, encoded).map_err(|source| ReportError::new(&path, source))?;
        self.give(&dir);
        self.give(&path);

        if let Some(record) = self.records.get_mut(index) {
            record.preview = Some(format!("{PREVIEW_DIR}/{name}"));
        }
        Ok(())
    }

    /// Writes `manifest.json` and returns its path.
    ///
    /// # Errors
    ///
    /// Fails when the manifest cannot be serialized or written.
    pub fn finish(mut self, summary: Summary<'_>) -> Result<PathBuf, ReportError> {
        let manifest = Manifest {
            tool_version: summary.tool_version.to_owned(),
            source: summary.source.to_owned(),
            scan_state: summary.state.to_owned(),
            rejected_candidates: summary.rejected_candidates,
            unreadable: summary.unreadable.to_vec(),
            triage: summary.triage.cloned(),
            fragmentation: summary.fragmentation.to_vec(),
            artifacts: std::mem::take(&mut self.records),
        };
        let path = self.dir.join(MANIFEST_FILE);
        // Streamed rather than built in memory first. A scan of a large medium
        // produces hundreds of thousands of records — one real run wrote a
        // manifest of 194 MB — and serializing that to a `Vec` doubles it,
        // on top of the records themselves, at the very end of a run that has
        // already been holding them all (A-BOUNDED-ALLOC).
        let file = File::create(&path).map_err(|source| ReportError::new(&path, source))?;
        let mut out = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut out, &manifest)
            .map_err(|source| ReportError::new(&path, io::Error::other(source)))?;
        out.flush()
            .map_err(|source| ReportError::new(&path, source))?;
        drop(out);
        self.give(&path);
        Ok(path)
    }
}

impl ArtifactSink for Store {
    type Error = ReportError;

    fn accept<R: Read + ?Sized>(
        &mut self,
        artifact: &Artifact<'_>,
        bytes: &mut R,
    ) -> Result<(), Self::Error> {
        Self::save(self, artifact, bytes).map(|_| ())
    }

    fn omit(&mut self, artifact: &Artifact<'_>, reason: &str) -> Result<(), Self::Error> {
        Self::record_only(self, artifact, reason);
        Ok(())
    }

    fn preview(
        &mut self,
        sha256: &argos_core::artifact::Digest,
        image: &PixelImage,
    ) -> Result<(), Self::Error> {
        Self::save_preview(self, sha256, image)
    }
}

/// What the scan as a whole found, for the manifest header.
#[derive(Clone, Copy, Debug)]
pub struct Summary<'a> {
    /// Version of the tool that produced the manifest.
    pub tool_version: &'a str,
    /// Description of the scanned source (path or label).
    pub source: &'a str,
    /// How the run ended: `finished`, `cancelled` or `failed`.
    pub state: &'a str,
    /// Signature hits that failed validation and were not recovered.
    pub rejected_candidates: u64,
    /// Byte ranges the medium could not read.
    pub unreadable: &'a [ExtentRecord],
    /// How triage ran, when the caller has anything to say about it.
    pub triage: Option<&'a TriageRecord>,
    /// Fragmentation points, so a later run can start from them.
    pub fragmentation: &'a [FragmentRecord],
}

/// Writing recovery output failed.
#[derive(Debug)]
pub struct ReportError {
    path: PathBuf,
    source: io::Error,
    backtrace: Backtrace,
}

impl ReportError {
    fn new(path: &Path, source: io::Error) -> Self {
        Self {
            path: path.to_path_buf(),
            source,
            backtrace: Backtrace::capture(),
        }
    }

    /// Output path the failure concerns.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot write recovery output {}: {}",
            self.path.display(),
            self.source
        )?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for ReportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Seconds since the Unix epoch, negative for times before it.
fn unix_seconds(at: std::time::SystemTime) -> i64 {
    match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |seconds| -seconds)
        }
    }
}

/// Rounds a classifier score to four decimals.
/// A manifest written before this field existed described stored artifacts
/// only, so an absent flag means the bytes are there.
const fn stored_by_default() -> bool {
    true
}

/// Lowercase hex of a digest.
fn hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use fmt::Write as _;
        write!(out, "{byte:02x}").unwrap_or_else(|err| {
            unreachable!("writing hex into a String cannot fail: {err}");
        });
    }
    out
}
