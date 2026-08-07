//! Recovery output: artifact files, SHA-256 hashes and the scan manifest.
//!
//! [`Store`] owns one output directory. Every artifact saved through it is
//! streamed to disk while its SHA-256 is computed, and everything ends up in a
//! machine-readable `manifest.json` carrying provenance: where each artifact's
//! bytes came from, how it was recovered, and its hash at recovery time.
//! Neither the manifest nor any error message contains recovered content or
//! names read from the medium — offsets, sizes, hashes and counts only.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

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
    /// Signature hits that failed validation and were not recovered.
    pub rejected_candidates: u64,
    /// One record per recovered artifact.
    pub artifacts: Vec<ArtifactRecord>,
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
    /// Artifact length in bytes.
    pub length: u64,
    /// Evidence tier, as its canonical display name.
    pub confidence: String,
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

    /// Streams `bytes` into `<dir>/<name>`, hashing while writing, and records
    /// the artifact with the provided provenance.
    ///
    /// # Errors
    ///
    /// Fails when the artifact file cannot be created or written, or when
    /// reading `bytes` fails.
    pub fn save(
        &mut self,
        name: &str,
        mut bytes: impl Read,
        provenance: Provenance<'_>,
    ) -> Result<&ArtifactRecord, ReportError> {
        let path = self.dir.join(name);
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
        if written != provenance.length {
            return Err(into_err(io::Error::other(format!(
                "short artifact: expected {} bytes from the source, got {written}",
                provenance.length
            ))));
        }

        self.records.push(ArtifactRecord {
            name: name.to_owned(),
            stage: provenance.stage.to_owned(),
            format: provenance.format.to_owned(),
            source_offset: provenance.source_offset,
            length: written,
            confidence: provenance.confidence.to_owned(),
            parent_offset: provenance.parent_offset,
            sha256: hex(&hasher.finalize()),
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
    pub fn finish(
        self,
        tool_version: &str,
        source: &str,
        rejected_candidates: u64,
    ) -> Result<PathBuf, ReportError> {
        let manifest = Manifest {
            tool_version: tool_version.to_owned(),
            source: source.to_owned(),
            rejected_candidates,
            artifacts: self.records,
        };
        let path = self.dir.join(MANIFEST_FILE);
        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|source| ReportError::new(&path, io::Error::other(source)))?;
        fs::write(&path, json).map_err(|source| ReportError::new(&path, source))?;
        Ok(path)
    }
}

/// Provenance of an artifact being saved, borrowed from the caller.
#[derive(Clone, Copy, Debug)]
pub struct Provenance<'a> {
    /// Recovery stage that produced the artifact (e.g. `carve`).
    pub stage: &'a str,
    /// Image format display name.
    pub format: &'a str,
    /// Absolute byte offset in the source.
    pub source_offset: u64,
    /// Length in bytes the recovery stage validated; a save yielding fewer or
    /// more bytes is refused rather than misrecorded.
    pub length: u64,
    /// Evidence tier display name.
    pub confidence: &'a str,
    /// Parent candidate offset, for embedded thumbnails.
    pub parent_offset: Option<u64>,
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
