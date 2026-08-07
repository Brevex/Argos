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
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Name of the manifest file inside the output directory.
const MANIFEST_FILE: &str = "manifest.json";

/// Bytes copied per streaming step while hashing and writing an artifact.
/// 64 KiB balances syscall count against buffer size.
const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// The scan manifest: tool identity, source description and every artifact.
#[derive(Debug, Serialize)]
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
    /// One record per recovered artifact.
    pub artifacts: Vec<ArtifactRecord>,
}

/// One contiguous source range, as recorded in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ExtentRecord {
    /// Absolute byte offset of the range's first byte in the source.
    pub offset: u64,
    /// Range length in bytes.
    pub length: u64,
}

/// Provenance record of one recovered artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArtifactRecord {
    /// File name of the artifact inside the output directory.
    pub name: String,
    /// Recovery stage that produced the artifact.
    pub stage: String,
    /// Image format the artifact validated as.
    pub format: String,
    /// Absolute byte offset of the artifact's first byte in the source.
    pub source_offset: u64,
    /// Artifact length in bytes: what was actually recovered and stored.
    pub length: u64,
    /// Length the source metadata claimed, when it said one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_length: Option<u64>,
    /// Bytes the metadata expected that were not recovered. Absent when the
    /// recovery is whole; present and non-zero states the truncation plainly
    /// (A-CONFIDENCE-HONEST).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_bytes: Option<u64>,
    /// Evidence tier, as its canonical display name.
    pub confidence: String,
    /// Every source extent the artifact was assembled from, in file order.
    pub extents: Vec<ExtentRecord>,
    /// Creation time recovered from metadata, in seconds since the Unix
    /// epoch. Never inferred; absent when the filesystem did not record one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_unix: Option<i64>,
    /// Last modification time recovered from metadata, in seconds since the
    /// Unix epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_unix: Option<i64>,
    /// File name recovered from filesystem metadata, when one survived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovered_name: Option<String>,
    /// Filesystem object the metadata came from — MFT record number, inode
    /// number or first cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_object: Option<u64>,
    /// For embedded thumbnails, the source offset of the parent candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_offset: Option<u64>,
    /// SHA-256 of the artifact bytes, computed while writing them.
    pub sha256: String,
}

/// Writes artifacts and the manifest into one output directory.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    records: Vec<ArtifactRecord>,
    /// Reused streaming buffer so saving many artifacts does not allocate per
    /// artifact.
    copy_buf: Vec<u8>,
}

impl Store {
    /// Creates `dir` (and parents) and an empty store over it.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created.
    pub fn create(dir: impl AsRef<Path>) -> Result<Self, ReportError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).map_err(|source| ReportError::new(dir, source))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            records: Vec::new(),
            copy_buf: Vec::new(),
        })
    }

    /// The directory this store writes into.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
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

        self.records.push(ArtifactRecord {
            name,
            stage: artifact.stage.to_string(),
            format: artifact.format.to_string(),
            source_offset: artifact
                .extents
                .first()
                .map_or(0, |first| first.start.get()),
            length: written,
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
        });
        Ok(self
            .records
            .last()
            .unwrap_or_else(|| unreachable!("a record was just pushed")))
    }

    /// Writes `manifest.json` and returns its path.
    ///
    /// # Errors
    ///
    /// Fails when the manifest cannot be serialized or written.
    pub fn finish(self, summary: Summary<'_>) -> Result<PathBuf, ReportError> {
        let manifest = Manifest {
            tool_version: summary.tool_version.to_owned(),
            source: summary.source.to_owned(),
            scan_state: summary.state.to_owned(),
            rejected_candidates: summary.rejected_candidates,
            unreadable: summary.unreadable.to_vec(),
            artifacts: self.records,
        };
        let path = self.dir.join(MANIFEST_FILE);
        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|source| ReportError::new(&path, io::Error::other(source)))?;
        fs::write(&path, json).map_err(|source| ReportError::new(&path, source))?;
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
