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

use crate::classify::PixelImage;
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
    /// Width and height of the decoded picture, when it decoded.
    ///
    /// The one property that tells a photograph from the derived images a used
    /// disk is full of — cache entries, icons, avatars — and the one a reader
    /// of the manifest cannot work out from a byte count. Absent when the
    /// artifact did not decode, which is a statement about the decoder and not
    /// about the bytes.
    pub pixels: Option<(u32, u32)>,
    /// What the picture records about itself and the camera that made it.
    pub capture: &'a Capture,
}

/// What a recovered picture records about itself and the camera that made it.
///
/// Every field is optional because every one is absent from some real file, and
/// nothing read from a medium is trusted to be there.
///
/// This is how a person finds their own photographs among a used disk's
/// hundreds of thousands of recovered images: a byte count and an offset
/// separate nothing, while a camera model and a date separate one afternoon
/// from ten years of everything else. It survives a frame that does not —
/// the metadata sits ahead of the picture data, so a photograph whose picture
/// is half overwritten still says when it was taken.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Capture {
    /// Camera manufacturer.
    pub make: Option<String>,
    /// Camera model.
    pub model: Option<String>,
    /// When the picture was taken, as stored: `YYYY:MM:DD HH:MM:SS`. Kept
    /// verbatim rather than parsed — it carries no zone, so turning it into an
    /// instant would be inventing one.
    pub taken: Option<String>,
    /// When the file was last changed, in the same form.
    pub modified: Option<String>,
    /// Pixel dimensions the metadata records, which the picture's own header
    /// may contradict and which survive when that header does not.
    pub pixels: Option<(u32, u32)>,
}

impl Capture {
    /// Whether anything at all was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// `Debug` reports which fields were recorded, never what they say.
///
/// A camera model and the moment a picture was taken are read off the medium
/// and describe a person, so they belong in a manifest the user asked for and
/// nowhere else — not in a log, a panic message or a test failure
/// (A-NO-CONTENT-IN-LOGS).
impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = |value: &Option<String>| {
            if value.is_some() {
                "<recorded>"
            } else {
                "<absent>"
            }
        };
        f.debug_struct("Capture")
            .field("make", &held(&self.make))
            .field("model", &held(&self.model))
            .field("taken", &held(&self.taken))
            .field("modified", &held(&self.modified))
            .field("pixels", &self.pixels)
            .finish()
    }
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
            .field("capture", &self.capture)
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

    /// Records an artifact that was recognised but whose bytes were not
    /// stored, and why.
    ///
    /// The one way an artifact can exist in the account without existing in
    /// the output directory. The record it produces carries the same extents,
    /// digest, format, confidence and dimensions as a stored one, so the
    /// manifest still describes everything the medium held and its extents
    /// locate the bytes on the medium exactly.
    ///
    /// `reason` is copied in as given. Nothing here decides anything: a sink
    /// records what it is told (A-TRIAGE-NOT-VERDICT).
    ///
    /// The default records nothing, which is what a sink that stores
    /// everything wants.
    ///
    /// # Errors
    ///
    /// Fails when the record cannot be kept.
    fn omit(&mut self, artifact: &Artifact<'_>, reason: &str) -> Result<(), Self::Error> {
        let _ = (artifact, reason);
        Ok(())
    }

    /// Accepts a preview of an artifact already handed to
    /// [`accept`](ArtifactSink::accept), named by its content hash.
    ///
    /// A preview is derived presentation, not evidence: it lets a viewer show
    /// what was recovered without opening a full-resolution image, and it is
    /// reproducible from the artifact at any time. A sink with no use for one
    /// ignores it, which is what the default does.
    ///
    /// Called at most once per artifact, only for artifacts that decoded, and
    /// only when the caller asked for previews. An artifact whose preview
    /// fails stays recovered and recorded — losing a thumbnail must never cost
    /// evidence, so the caller counts the failure instead of propagating it.
    ///
    /// # Errors
    ///
    /// Fails when the preview cannot be stored.
    fn preview(&mut self, sha256: &Digest, image: &PixelImage) -> Result<(), Self::Error> {
        let _ = (sha256, image);
        Ok(())
    }
}
