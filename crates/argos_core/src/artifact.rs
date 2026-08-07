//! The port through which recovered artifacts leave the engine.
//!
//! An [`Artifact`] is the provenance record of one recovery — where its bytes
//! came from, how they were obtained, and their hash at recovery time
//! (A-PROVENANCE). The bytes themselves travel separately, as a reader, so a
//! sink streams them wherever it wants without the engine ever holding a whole
//! image in memory.

use std::error::Error;
use std::fmt;
use std::io::Read;

use crate::geometry::{ByteOffset, ByteRange};
use crate::recovery::{Confidence, Format, Stage, Timestamps};

/// A SHA-256 digest, displayed as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; Self::LEN]);

impl Digest {
    /// Length of a SHA-256 digest in bytes.
    pub const LEN: usize = 32;

    /// The digest with the given bytes.
    #[must_use]
    pub const fn new(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

/// Provenance of one recovered artifact, handed to an [`ArtifactSink`].
///
/// `Debug` redacts [`Artifact::recovered_name`]: a name read off a medium is
/// identifying content and must never reach a log or a panic message
/// (A-NO-CONTENT-IN-LOGS). It reaches the user only through a sink that was
/// asked for it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Artifact<'a> {
    /// Image format the artifact validated as.
    pub format: Format,
    /// Pipeline stage that produced it.
    pub stage: Stage,
    /// Evidence tier — the tier of the evidence, never higher
    /// (A-CONFIDENCE-HONEST).
    pub confidence: Confidence,
    /// Source extents, absolute in the medium, in file order. Concatenating
    /// them yields exactly the bytes the sink receives.
    pub extents: &'a [ByteRange],
    /// Total artifact length in bytes: the sum of the extent lengths, and
    /// exactly the number of bytes the sink will receive.
    pub length: u64,
    /// Length the source metadata claimed the file had, when it said.
    ///
    /// A value larger than [`Artifact::length`] means part of the file was not
    /// recovered — a hole in the extent list, or a run the medium could not
    /// give back. Reporting the pair is what states a truncation plainly
    /// instead of presenting a short file as whole (A-CONFIDENCE-HONEST).
    pub expected_length: Option<u64>,
    /// SHA-256 of the artifact bytes, computed at recovery time.
    ///
    /// It describes exactly the byte stream handed to the sink.
    pub sha256: Digest,
    /// Timestamps recovered from the source metadata, never invented.
    pub timestamps: Timestamps,
    /// Name recovered from filesystem metadata, when one survived.
    ///
    /// It belongs to the object named by [`Artifact::source_object`]; the two
    /// always describe the same filesystem object or are both absent.
    pub recovered_name: Option<&'a str>,
    /// Identity of the filesystem object the metadata came from — MFT record
    /// number, inode number or first cluster.
    pub source_object: Option<u64>,
    /// For an embedded thumbnail, the offset of the candidate it was found in.
    pub parent: Option<ByteOffset>,
}

impl fmt::Debug for Artifact<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Artifact")
            .field("format", &self.format)
            .field("stage", &self.stage)
            .field("confidence", &self.confidence)
            .field("extents", &self.extents)
            .field("length", &self.length)
            .field("expected_length", &self.expected_length)
            .field("sha256", &self.sha256)
            .field("timestamps", &self.timestamps)
            .field("recovered_name", &self.recovered_name.map(|_| "<redacted>"))
            .field("source_object", &self.source_object)
            .field("parent", &self.parent)
            .finish()
    }
}

/// Receives validated artifacts and owns the output layout.
///
/// Implementations decide where bytes land and how they are named. The engine
/// guarantees that `bytes` yields exactly [`Artifact::length`] bytes and that
/// they hash to [`Artifact::sha256`]: the digest is taken from this very
/// stream, so a sink that re-hashes what it stored will always agree, and a
/// scan whose medium changed underneath it fails rather than recording a
/// digest that describes different bytes.
pub trait ArtifactSink {
    /// What this sink fails with.
    type Error: Error + Send + Sync + 'static;

    /// Accepts one artifact, reading its bytes from `bytes`.
    ///
    /// # Errors
    ///
    /// Fails when the artifact cannot be stored, or when reading `bytes`
    /// fails; either aborts the scan's report stage.
    fn accept<R: Read + ?Sized>(
        &mut self,
        artifact: &Artifact<'_>,
        bytes: &mut R,
    ) -> Result<(), Self::Error>;
}
