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
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use argos_core::ports::{Artifact, ArtifactSink, Digest, PixelImage};

use sha2::{Digest as _, Sha256};

pub mod manifest;

use manifest::MANIFEST_FILE;

pub use manifest::{
    ArtifactRecord, CoverageRecord, ExtentRecord, FragmentRecord, LostFileRecord, Manifest,
    ResidueCensusRecord, RunRecord, TriageAnnotation, TriageRecord, VolumeRecord,
};

/// Subdirectory of the output holding preview images.
///
/// Separate from the artifacts on purpose: it is the one part of a session
/// directory that holds derived rather than recovered bytes, so a viewer can
/// be given access to it without being given access to the evidence
/// (`A-DTO-VERSIONED`).
pub const PREVIEW_DIR: &str = "previews";

/// Bytes of write buffer the manifest is serialized through.
const MANIFEST_BUFFER_BYTES: usize = 1024 * 1024;

/// Bytes copied per streaming step while hashing and writing an artifact.
///
/// Written straight to the file: a `BufWriter` passes through any write at
/// least as large as its own capacity, so one sitting under a chunk this size
/// would absorb nothing but the last short piece of each artifact. One megabyte
/// puts a three-megabyte photograph on disk in three writes rather than
/// forty-eight, and is the same buffer for every artifact of a run
/// (`M-MEM-REUSE`).
const COPY_CHUNK_BYTES: usize = 1024 * 1024;

/// Writes artifacts and the manifest into one output directory.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    records: Vec<ArtifactRecord>,
    /// Where each digest's record sits in `records`, so an annotation reaches
    /// its record in one lookup rather than by walking every record before it.
    /// A whole-disk recovery writes hundreds of thousands of them, and each is
    /// annotated three times.
    ///
    /// Keyed by the 32-byte digest rather than by the hex string the record
    /// serialises, so a join costs no allocation. The two cannot disagree:
    /// both are derived from the same [`Artifact::sha256`], which [`Store::save`]
    /// has already checked against the bytes it wrote.
    by_digest: HashMap<Digest, usize>,
    /// Who every file written here is given to, once it is written. `None`
    /// when nobody was named or the directory could not be handed over.
    owner: Option<Owner>,
    /// What happened when the directory itself was handed over, for the caller
    /// to report.
    handback: Handback,
    /// Reused streaming buffer so saving many artifacts does not allocate per
    /// artifact.
    copy_buf: Vec<u8>,
    /// Whether the preview directory has been created and handed over.
    ///
    /// Lazily, and once: a session that rendered no previews should not be left
    /// with an empty `previews/` explaining nothing, and a session that
    /// rendered two hundred thousand should not pay a `mkdir` and a `chown` of
    /// the directory for each of them.
    previews_ready: bool,
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
            by_digest: HashMap::new(),
            owner: handback.owner(owner),
            handback,
            copy_buf: Vec::new(),
            previews_ready: false,
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

    /// Appends one record and indexes it by the digest it describes.
    ///
    /// The index keeps the *first* record for a digest, which is the record
    /// a scan of every record in order would have found. Two records can carry
    /// one digest — the engine's own deduplication prevents it, but this store
    /// is public and does not — and an annotation must land on the same one
    /// either way.
    fn push_record(&mut self, digest: Digest, record: ArtifactRecord) {
        self.by_digest.entry(digest).or_insert(self.records.len());
        self.records.push(record);
    }

    /// Where `digest`'s record sits, if this store holds one for it.
    fn index_of(&self, digest: Digest) -> Option<usize> {
        self.by_digest.get(&digest).copied()
    }

    /// The record `digest` was stored under, if this store stored it.
    fn record_of(&mut self, digest: Digest) -> Option<&mut ArtifactRecord> {
        let index = self.index_of(digest)?;
        self.records.get_mut(index)
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

        let mut out = File::create(&path).map_err(into_err)?;
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
        let sha256 = Digest::new(hasher.finalize().into()).to_string();
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

        self.push_record(
            artifact.sha256,
            ArtifactRecord {
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
                deleted_unix: artifact.deleted.map(unix_seconds),
                recovered_name: artifact.recovered_name.map(str::to_owned),
                source_object: artifact.source_object,
                parent_offset: artifact.parent.map(argos_core::ByteOffset::get),
                sha256,
                standing: None,
                same_size_neighbours: None,
                triage_label: None,
                triage_decided_by: None,
                written: true,
                omitted_because: None,
                perceptual_hash: None,
                near_duplicate_of: None,
                preview: None,
            },
        );
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
            let Some(record) = self.record_of(annotation.sha256) else {
                continue;
            };
            record.triage_label.clone_from(&annotation.label);
            record.triage_decided_by.clone_from(&annotation.decided_by);
            record
                .perceptual_hash
                .clone_from(&annotation.perceptual_hash);
            record.near_duplicate_of = annotation.near_duplicate_of.map(|of| of.to_string());
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
    fn record_only(&mut self, artifact: &Artifact<'_>, reason: &str) {
        self.push_record(
            artifact.sha256,
            ArtifactRecord {
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
                deleted_unix: artifact.deleted.map(unix_seconds),
                recovered_name: artifact.recovered_name.map(str::to_owned),
                source_object: artifact.source_object,
                parent_offset: artifact.parent.map(argos_core::ByteOffset::get),
                sha256: artifact.sha256.to_string(),
                written: false,
                triage_label: None,
                triage_decided_by: None,
                standing: None,
                same_size_neighbours: None,
                perceptual_hash: None,
                near_duplicate_of: None,
                preview: None,
                omitted_because: Some(reason.to_owned()),
            },
        );
    }

    /// Records where each named artifact stands in a list.
    ///
    /// Annotation only, like the triage labels and the neighbour counts: it
    /// adds a sort key to records already written and can remove nothing
    /// (A-TRIAGE-NOT-VERDICT). A standing for an artifact this store never
    /// saved is dropped rather than creating a record.
    pub fn annotate_standings(&mut self, standings: &[(Digest, &str)]) {
        for (sha256, standing) in standings {
            if let Some(record) = self.record_of(*sha256) {
                record.standing = Some((*standing).to_owned());
            }
        }
    }

    /// Records how many same-sized neighbours each named artifact was found
    /// among.
    ///
    /// Annotation only, like the triage labels: it adds a fact about the
    /// medium's layout to records already written and can remove nothing.
    pub fn annotate_same_size_runs(&mut self, runs: &[(Digest, u32)]) {
        for (sha256, neighbours) in runs {
            if let Some(record) = self.record_of(*sha256) {
                record.same_size_neighbours = Some(*neighbours);
            }
        }
    }

    /// The preview directory, created and handed over on the first call.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created.
    fn preview_dir(&mut self) -> Result<PathBuf, ReportError> {
        let dir = self.dir.join(PREVIEW_DIR);
        if !self.previews_ready {
            fs::create_dir_all(&dir).map_err(|source| ReportError::new(&dir, source))?;
            self.give(&dir);
            self.previews_ready = true;
        }
        Ok(dir)
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
    fn save_preview(
        &mut self,
        sha256: &argos_core::ports::Digest,
        image: &PixelImage,
    ) -> Result<(), ReportError> {
        let Some(index) = self.index_of(*sha256) else {
            return Ok(());
        };
        // An image the encoder declines is one with no pixels to show. That is
        // not a failure of the output directory, and reporting it as one would
        // make a caller count it against the medium.
        let Some(encoded) = encode(image) else {
            return Ok(());
        };
        // Spelled out only now: both ways out above are taken often enough on
        // a whole-disk recovery that formatting the hex first would be paid
        // for previews that are never written.
        let hash = sha256.to_string();

        let dir = self.preview_dir()?;
        // Named by content hash, so the manifest joins previews to records the
        // same way it joins triage annotations, and a rerun overwrites rather
        // than accumulating.
        let name = format!("{hash}.jpg");
        let path = dir.join(&name);
        fs::write(&path, encoded).map_err(|source| ReportError::new(&path, source))?;
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
            coverage: summary.coverage.cloned(),
            volumes: summary.volumes.to_vec(),
            fragmentation: summary.fragmentation.to_vec(),
            lost_files: summary.lost_files.to_vec(),
            artifacts: std::mem::take(&mut self.records),
        };
        let path = self.dir.join(MANIFEST_FILE);
        // Streamed rather than built in memory first. A scan of a large medium
        // produces hundreds of thousands of records — one real run wrote a
        // manifest of 194 MB — and serializing that to a `Vec` doubles it,
        // on top of the records themselves, at the very end of a run that has
        // already been holding them all (A-BOUNDED-ALLOC).
        let file = File::create(&path).map_err(|source| ReportError::new(&path, source))?;
        // A buffer of its own, because the serializer writes a field at a time:
        // the default eight kibibytes turns a manifest of that size into tens of
        // thousands of writes.
        let mut out = BufWriter::with_capacity(MANIFEST_BUFFER_BYTES, file);
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
        sha256: &argos_core::ports::Digest,
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
    /// What the run reached and what it stopped short of.
    pub coverage: Option<&'a CoverageRecord>,
    /// Volumes located, current and residual.
    pub volumes: &'a [VolumeRecord],
    /// Fragmentation points, so a later run can start from them.
    pub fragmentation: &'a [FragmentRecord],
    /// Files a metadata record names that the run could not place.
    pub lost_files: &'a [LostFileRecord],
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
///
/// Public because the record types here state their times this way, so anything
/// building one needs the same conversion — and a caller writing its own would
/// have to get the pre-epoch case right to agree with the records this crate
/// builds itself.
#[must_use]
pub fn unix_seconds(at: std::time::SystemTime) -> i64 {
    match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |seconds| -seconds)
        }
    }
}

/// The account that recovered files belong to once written.
///
/// Constructed from the identity a privileged process was started on behalf
/// of, which the process running the scan resolves; this crate is given the
/// answer rather than looking for it, so what it does is visible in its
/// signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    uid: u32,
    gid: Option<u32>,
}

impl Owner {
    /// The account with this user id, and optionally this group.
    ///
    /// The group is optional because not every elevation path reports one:
    /// `pkexec` publishes the caller's user id alone, and leaving the group
    /// untouched is better than guessing at one.
    #[must_use]
    pub fn new(uid: u32, gid: Option<u32>) -> Self {
        Self { uid, gid }
    }

    /// Gives `path` to this owner.
    ///
    /// # Errors
    ///
    /// Fails when the filesystem cannot represent the change — no ownership at
    /// all on FAT and exFAT — or when this process is not allowed to make it.
    #[cfg(unix)]
    pub fn give(self, path: &Path) -> Result<(), std::io::Error> {
        std::os::unix::fs::chown(path, Some(self.uid), self.gid)
    }

    /// Gives `path` to this owner.
    ///
    /// Windows has nothing to do here: a file created by an elevated process
    /// inherits the destination folder's access rules, so the person who chose
    /// the folder keeps the access they already had to it.
    ///
    /// # Errors
    ///
    /// Never on this platform. The signature is the Unix one so that the caller
    /// stays free of `cfg`.
    #[cfg(not(unix))]
    pub fn give(self, path: &Path) -> Result<(), std::io::Error> {
        let _ = path;
        Ok(())
    }
}

/// Whether an output directory could be handed to its [`Owner`].
///
/// Reported rather than hidden: files left belonging to the administrator are
/// something the person reading the result has to know, and it is the kind of
/// thing that is discovered hours later, at the end of a long scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handback {
    /// Nothing to do: no owner was named, or the platform has no notion of one.
    NotNeeded,
    /// The output directory now belongs to the owner, and its files will too.
    Done,
    /// The output directory could not be handed over, and neither will its
    /// files be. Carries what to tell the user.
    Refused(String),
}

impl Handback {
    /// Attempts to give `dir` to `owner`, and says what happened.
    pub(crate) fn attempt(dir: &Path, owner: Option<Owner>) -> Self {
        let Some(owner) = owner else {
            return Self::NotNeeded;
        };
        match owner.give(dir) {
            Ok(()) => Self::Done,
            // The message names no recovered content and no filename, only the
            // directory the user themselves chose (A-NO-CONTENT-IN-LOGS).
            Err(err) => Self::Refused(format!(
                "recovered files will belong to the administrator account rather than to you, \
                 because the destination does not allow changing ownership ({err}); copy them \
                 elsewhere or take ownership of {} afterwards",
                dir.display()
            )),
        }
    }

    /// The owner to apply to each file written, if any.
    pub(crate) fn owner(&self, owner: Option<Owner>) -> Option<Owner> {
        matches!(self, Self::Done).then_some(owner).flatten()
    }
}

/// Longest edge of a preview, in pixels.
///
/// Large enough to recognise a photograph in a gallery, small enough that a
/// directory of thousands costs less than one recovered image.
pub(crate) const MAX_EDGE: u32 = 256;

/// JPEG quality of a preview. Previews are looked at, never analysed, and past
/// this the extra bytes stop being visible at this size.
const QUALITY: u8 = 80;

/// Channels in the encoder's input.
const RGB_CHANNELS: usize = 3;

/// Full opacity, and the divisor of the alpha composite below.
const OPAQUE: u32 = 255;

/// A downscaled JPEG of `image`, or `None` when it has no pixels to show.
///
/// Transparency is composited over white, so an icon with an alpha channel
/// looks in a gallery the way it looks in a file manager rather than as a
/// black square.
pub(crate) fn encode(image: &PixelImage) -> Option<Vec<u8>> {
    let (width, height) = (image.width(), image.height());
    let (target_width, target_height) = fit(width, height)?;

    let mut rgb = vec![
        0_u8;
        usize::try_from(target_width)
            .ok()?
            .checked_mul(usize::try_from(target_height).ok()?)?
            .checked_mul(RGB_CHANNELS)?
    ];
    resample(image, target_width, target_height, &mut rgb);

    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, QUALITY)
        .encode(
            &rgb,
            u16::try_from(target_width).ok()?,
            u16::try_from(target_height).ok()?,
            jpeg_encoder::ColorType::Rgb,
        )
        .ok()?;
    Some(out)
}

/// The preview's dimensions: `width`×`height` scaled to fit [`MAX_EDGE`],
/// never enlarged, never collapsed to zero.
///
/// `None` for an image with no pixels — there is nothing to show, and the
/// division below would have no defined answer.
fn fit(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let longest = width.max(height);
    if longest <= MAX_EDGE {
        return Some((width, height));
    }
    // In `u64`: the products below exceed `u32` for any image wider than
    // 16 megapixels on one edge, and the medium decides these numbers
    // (A-UNTRUSTED-ONDISK).
    let scale = |edge: u32| -> u32 {
        let scaled = u64::from(edge) * u64::from(MAX_EDGE) / u64::from(longest);
        u32::try_from(scaled).unwrap_or(MAX_EDGE).max(1)
    };
    Some((scale(width), scale(height)))
}

/// Averages each target pixel over the source box it covers.
///
/// Box sampling rather than point sampling: a photograph point-sampled to a
/// sixteenth of its width aliases into something that can look like a
/// synthetic asset, and a preview that misrepresents what was recovered is
/// worse than no preview.
fn resample(image: &PixelImage, target_width: u32, target_height: u32, out: &mut [u8]) {
    let (width, height) = (image.width(), image.height());
    let pixels = image.rgba();
    let stride = usize::try_from(width).unwrap_or(0) * PixelImage::BYTES_PER_PIXEL;

    for ty in 0..target_height {
        let (top, bottom) = box_edges(ty, target_height, height);
        for tx in 0..target_width {
            let (left, right) = box_edges(tx, target_width, width);

            let (mut red, mut green, mut blue, mut count) = (0_u64, 0_u64, 0_u64, 0_u64);
            for y in top..bottom {
                let row = usize::try_from(y).unwrap_or(0) * stride;
                for x in left..right {
                    let at = row + usize::try_from(x).unwrap_or(0) * PixelImage::BYTES_PER_PIXEL;
                    let Some(pixel) = pixels.get(at..at + PixelImage::BYTES_PER_PIXEL) else {
                        continue;
                    };
                    let alpha = u32::from(pixel[3]);
                    red += u64::from(over_white(u32::from(pixel[0]), alpha));
                    green += u64::from(over_white(u32::from(pixel[1]), alpha));
                    blue += u64::from(over_white(u32::from(pixel[2]), alpha));
                    count += 1;
                }
            }

            let at = (usize::try_from(ty).unwrap_or(0)
                * usize::try_from(target_width).unwrap_or(0)
                + usize::try_from(tx).unwrap_or(0))
                * RGB_CHANNELS;
            let Some(target) = out.get_mut(at..at + RGB_CHANNELS) else {
                continue;
            };
            // A box with no samples cannot happen — `box_edges` never returns
            // an empty span — but a preview must not panic over a thumbnail.
            let mean = |sum: u64| u8::try_from(sum / count.max(1)).unwrap_or(u8::MAX);
            target[0] = mean(red);
            target[1] = mean(green);
            target[2] = mean(blue);
        }
    }
}

/// The half-open source span target index `at` of `target_len` covers in a
/// source of `source_len` pixels. Never empty, never past the source.
fn box_edges(at: u32, target_len: u32, source_len: u32) -> (u32, u32) {
    let span = |index: u64| -> u32 {
        let scaled = index * u64::from(source_len) / u64::from(target_len.max(1));
        u32::try_from(scaled).unwrap_or(source_len).min(source_len)
    };
    let start = span(u64::from(at));
    let end = span(u64::from(at) + 1)
        .max(start.saturating_add(1))
        .min(source_len);
    (start, end)
}

/// One channel composited over an opaque white background.
fn over_white(channel: u32, alpha: u32) -> u8 {
    let blended = (channel * alpha + OPAQUE * (OPAQUE - alpha)) / OPAQUE;
    u8::try_from(blended).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use argos_core::ports::PixelImage;

    #[cfg(unix)]
    use super::Owner;
    use super::{Handback, MAX_EDGE, encode, fit};

    #[test]
    fn no_owner_means_nothing_to_do() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert_eq!(Handback::attempt(dir.path(), None), Handback::NotNeeded);
    }

    #[cfg(unix)]
    #[test]
    fn handing_a_directory_to_its_current_owner_succeeds() {
        // Every account may give a file it owns to itself, so this exercises
        // the real syscall without needing privileges.
        let dir = tempfile::tempdir().expect("temporary directory");
        let owner = current();
        assert_eq!(Handback::attempt(dir.path(), Some(owner)), Handback::Done);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_handback_stops_the_per_file_attempts() {
        // Once the directory could not be handed over, its files are on the
        // same filesystem and will not be either; retrying per artifact would
        // fail once per recovered image.
        let refused = Handback::Refused("no".to_owned());
        assert_eq!(refused.owner(Some(current())), None);
        assert_eq!(Handback::Done.owner(Some(current())), Some(current()));
        assert_eq!(Handback::Done.owner(None), None);
    }

    #[cfg(unix)]
    fn current() -> Owner {
        use std::os::unix::fs::MetadataExt;

        let me = std::fs::metadata("/proc/self")
            .or_else(|_| std::fs::metadata("."))
            .expect("this process can stat something it owns");
        Owner::new(me.uid(), Some(me.gid()))
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> PixelImage {
        let count = (width as usize) * (height as usize);
        PixelImage::new(width, height, rgba.repeat(count))
    }

    #[test]
    fn a_preview_never_enlarges_a_small_image() {
        // Upscaling a 32×32 icon to 256 would put more pixels in the preview
        // than the artifact has, which says something false about it.
        assert_eq!(fit(32, 32), Some((32, 32)));
        assert_eq!(fit(MAX_EDGE, MAX_EDGE), Some((MAX_EDGE, MAX_EDGE)));
    }

    #[test]
    fn a_large_image_keeps_its_aspect_ratio() {
        assert_eq!(fit(4000, 3000), Some((256, 192)));
        assert_eq!(fit(3000, 4000), Some((192, 256)));
        // An extreme panorama still has a visible edge rather than none.
        let (width, height) = fit(100_000, 10).expect("a panorama has pixels");
        assert_eq!(width, MAX_EDGE);
        assert!(height >= 1, "an edge must not round away to nothing");
    }

    #[test]
    fn an_empty_image_has_no_preview() {
        assert_eq!(fit(0, 100), None);
        assert_eq!(fit(100, 0), None);
        assert!(encode(&solid(0, 0, [0; 4])).is_none());
    }

    #[test]
    fn a_preview_is_a_decodable_jpeg() {
        let bytes = encode(&solid(300, 200, [200, 40, 40, 255])).expect("a solid image encodes");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG starts with SOI");
        assert_eq!(
            &bytes[bytes.len() - 2..],
            &[0xFF, 0xD9],
            "and ends with EOI"
        );
    }

    #[test]
    fn transparency_is_composited_over_white_not_over_black() {
        // A fully transparent asset must not preview as a black rectangle:
        // that is what an overwritten region looks like, and confusing the two
        // in a gallery is exactly the wrong mistake for this tool to make.
        let clear = encode(&solid(64, 64, [0, 0, 0, 0])).expect("a clear image encodes");
        let black = encode(&solid(64, 64, [0, 0, 0, 255])).expect("a black image encodes");
        assert_ne!(clear, black);
    }
}
