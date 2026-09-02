//! The four ports of the hexagon, and the vocabulary each one carries.
//!
//! Every other crate in the workspace either implements one of these or calls
//! through one. Where `std` already provides the abstraction, the `std` trait
//! *is* the port — parsers and carvers consume `impl Read + Seek` — so these
//! four exist only for what that cannot express.
//!
//! **[`BlockSource`] — reading the medium.** The only way any Argos crate
//! reaches bytes on a device or image. It deliberately has no write, discard or
//! passthrough method: a write path to the source medium must not exist
//! anywhere in the workspace. Implementations report unreadable regions as
//! [`ReadError`]s — corruption and bad sectors are the expected operating
//! condition, never a reason to panic — and must never fabricate data for
//! sectors they could not read.
//!
//! **[`ArtifactSink`] — delivering results.** An [`Artifact`] is the provenance
//! record of one recovery: where its bytes came from, how they were obtained,
//! and their hash at recovery time (A-PROVENANCE). The bytes themselves travel
//! separately, as a reader, so a sink streams them wherever it wants without
//! the engine ever holding a whole image in memory.
//!
//! **[`Classifier`] — scoring recovered images.** Triage exists to save the
//! examiner's time, never to decide what the examiner sees. A classifier hands
//! back an opinion *about* an artifact; it has no way to remove one, because it
//! is only ever consulted after the artifact is persisted, hashed and recorded,
//! and its whole output is an annotation (A-TRIAGE-NOT-VERDICT). The port has
//! no filtering method to misuse.
//!
//! **[`ProgressSink`] — reporting what a run is doing.** Events are
//! *structured*: named variants with named fields, never pre-formatted strings,
//! so a renderer decides how — and whether — to show them
//! (`M-LOG-STRUCTURED`). They are also *batched*: the pipeline emits one event
//! per chunk or per stage, never one per sector or per candidate, so a sink can
//! never become the bottleneck (`M-LOG-OVERHEAD`). No event carries recovered
//! content, a recovered filename or an identifying path
//! (A-NO-CONTENT-IN-LOGS); offsets, sizes and counts only.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

use crate::{ByteOffset, ByteRange, Confidence, Format, Lba, SectorSize, Stage, Timestamps};

/// Sector-addressed, read-only access to a medium under analysis.
///
/// Implementations report unreadable regions as [`ReadError`]s — corruption and bad
/// sectors are the expected operating condition, never a reason to panic — and must
/// never fabricate data for sectors they could not read.
pub trait BlockSource: Send {
    /// The medium's geometry, queried once at open time.
    fn geometry(&self) -> Geometry;

    /// Reads exactly `buf.len()` bytes starting at the first byte of sector `lba`.
    ///
    /// # Errors
    ///
    /// Fails when the range lies outside the medium, when the underlying medium
    /// reports unreadable sectors, or on an I/O fault. On error the contents of
    /// `buf` are unspecified and must not be used.
    ///
    /// # Panics
    ///
    /// Implementations panic if `buf.len()` is zero or not a multiple of the sector
    /// size — that is a caller bug, not a property of the medium.
    fn read_at(&mut self, lba: Lba, buf: &mut [u8]) -> Result<(), ReadError>;
}

/// Geometry of a medium: sector size, extent and device class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Logical sector size used for all addressing on this medium.
    pub sector_size: SectorSize,
    /// Total number of addressable sectors.
    pub sector_count: u64,
    /// What kind of medium this is.
    pub class: DeviceClass,
}

impl Geometry {
    /// Geometry of a `class` medium with `sector_count` sectors of `sector_size`.
    #[must_use]
    pub const fn new(sector_size: SectorSize, sector_count: u64, class: DeviceClass) -> Self {
        Self {
            sector_size,
            sector_count,
            class,
        }
    }

    /// Total capacity in bytes, or `None` on overflow.
    #[must_use]
    pub const fn capacity_bytes(&self) -> Option<u64> {
        self.sector_count.checked_mul(self.sector_size.as_u64())
    }

    /// Whether `range` (starting at `lba`, `sectors` long) lies within the medium.
    #[must_use]
    pub const fn contains(&self, lba: Lba, sectors: u64) -> bool {
        match lba.get().checked_add(sectors) {
            Some(end) => end <= self.sector_count,
            None => false,
        }
    }
}

/// Kind of medium behind a [`BlockSource`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceClass {
    /// Rotational disk: sequential access dominates throughput.
    Hdd,
    /// Solid-state device: TRIM may have erased deleted data.
    Ssd,
    /// A file holding a raw image of a medium.
    ImageFile,
    /// The class could not be determined.
    Unknown,
}

impl fmt::Display for DeviceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Hdd => "hdd",
            Self::Ssd => "ssd",
            Self::ImageFile => "image file",
            Self::Unknown => "unknown",
        };
        f.write_str(name)
    }
}

/// A read from a [`BlockSource`] failed.
///
/// Carries the failed range so reports can map damage precisely; accessors expose
/// what a caller can act on without making internal failure modes public API.
#[derive(Debug)]
pub struct ReadError {
    lba: Lba,
    sectors: u64,
    kind: ReadErrorKind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum ReadErrorKind {
    BadSector,
    OutOfRange { sector_count: u64 },
    Io(io::Error),
}

impl ReadError {
    /// The medium reported the range starting at `lba` unreadable.
    #[must_use]
    pub fn bad_sector(lba: Lba, sectors: u64) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::BadSector)
    }

    /// The requested range does not fit a medium of `sector_count` sectors.
    #[must_use]
    pub fn out_of_range(lba: Lba, sectors: u64, sector_count: u64) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::OutOfRange { sector_count })
    }

    /// An I/O fault occurred while reading the range.
    #[must_use]
    pub fn io(lba: Lba, sectors: u64, source: io::Error) -> Self {
        Self::with_kind(lba, sectors, ReadErrorKind::Io(source))
    }

    fn with_kind(lba: Lba, sectors: u64, kind: ReadErrorKind) -> Self {
        Self {
            lba,
            sectors,
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// First sector of the failed range.
    #[must_use]
    pub fn lba(&self) -> Lba {
        self.lba
    }

    /// Length of the failed range in sectors.
    #[must_use]
    pub fn sectors(&self) -> u64 {
        self.sectors
    }

    /// Whether the medium itself reported the sectors unreadable.
    #[must_use]
    pub fn is_bad_sector(&self) -> bool {
        matches!(self.kind, ReadErrorKind::BadSector)
    }

    /// Whether the request lay outside the medium.
    #[must_use]
    pub fn is_out_of_range(&self) -> bool {
        matches!(self.kind, ReadErrorKind::OutOfRange { .. })
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "read of sectors {}..+{} failed: ",
            self.lba, self.sectors
        )?;
        match &self.kind {
            ReadErrorKind::BadSector => f.write_str("medium reports bad sector"),
            ReadErrorKind::OutOfRange { sector_count } => {
                write!(f, "range exceeds medium of {sector_count} sectors")
            }
            ReadErrorKind::Io(source) => write!(f, "i/o fault: {source}"),
        }?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for ReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ReadErrorKind::Io(source) => Some(source),
            ReadErrorKind::BadSector | ReadErrorKind::OutOfRange { .. } => None,
        }
    }
}
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
    /// When a change journal recorded the file being deleted, when one did.
    ///
    /// The only timestamp about the *removal* rather than about the file: a
    /// `FILE` record keeps when its file was made and last written, not when
    /// it stopped existing. A run of artifacts sharing this moment is a batch
    /// deletion — files removed in one action.
    pub deleted: Option<std::time::SystemTime>,
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
    /// Width and height of the picture, as its own frame header declares them.
    ///
    /// The one property that tells a photograph from the derived images a used
    /// disk is full of — cache entries, icons, avatars — and the one a reader
    /// of the manifest cannot work out from a byte count. Absent when no frame
    /// header was found or the frame it declares is outside the bounds this
    /// tool works within, which is a statement about the header and not about
    /// the bytes.
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
/// A decoded image, RGBA8 row-major, as handed to a [`Classifier`].
///
/// `Debug` prints dimensions only: decoded pixels are recovered content and
/// must never reach a log or a panic message (A-NO-CONTENT-IN-LOGS).
#[derive(Clone, PartialEq, Eq)]
pub struct PixelImage {
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

impl PixelImage {
    /// Bytes per pixel: red, green, blue, alpha.
    pub const BYTES_PER_PIXEL: usize = 4;

    /// An image of `width` by `height` pixels over `pixels`.
    ///
    /// # Panics
    ///
    /// Panics when `pixels` is not exactly `width * height * 4` bytes — the
    /// buffer and the dimensions come from the same decoder, so a mismatch is
    /// a bug in the adapter that produced them, not a property of the medium.
    #[must_use]
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(Self::BYTES_PER_PIXEL));
        assert_eq!(
            Some(pixels.len()),
            expected,
            "pixel buffer of {} bytes does not match {width}x{height} RGBA",
            pixels.len(),
        );
        Self {
            width,
            height,
            pixels: pixels.into_boxed_slice(),
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Number of pixels.
    #[must_use]
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The RGBA8 pixel data, row-major, four bytes per pixel.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.pixels
    }
}

impl fmt::Debug for PixelImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PixelImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"<redacted>")
            .finish()
    }
}

/// What triage concluded an image most likely is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TriageLabel {
    /// A user photograph — what a recovery of personal images is looking for.
    Photograph,
    /// A synthetic asset: icon, sprite, UI chrome, web-cache graphic.
    SyntheticAsset,
    /// Neither signal was strong enough to say. Presented, never hidden.
    Ambiguous,
}

impl fmt::Display for TriageLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Photograph => "photograph",
            Self::SyntheticAsset => "synthetic-asset",
            Self::Ambiguous => "ambiguous",
        };
        f.write_str(name)
    }
}

/// What settled a label.
///
/// Named rather than scored. A deterministic rule either fired or it did not,
/// and attaching a probability to that would be a number with nothing behind
/// it — the classifier does not estimate a likelihood, so it must not report
/// one. What an examiner can act on is *which* property decided, because that
/// is checkable against the image in front of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decision {
    /// Meaningful transparency. Photographs are opaque; alpha is authoring.
    Transparency,
    /// Too few distinct colours and luminance levels for a sensor's output.
    Palette,
    /// Long runs of byte-identical neighbours: flat fills and drawn edges.
    FlatFill,
    /// A high-frequency floor over the whole frame, which is what a sensor and
    /// a quantizer leave behind and drawn art does not.
    SensorTexture,
    /// No rule fired either way.
    Inconclusive,
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Transparency => "transparency",
            Self::Palette => "palette",
            Self::FlatFill => "flat-fill",
            Self::SensorTexture => "sensor-texture",
            Self::Inconclusive => "inconclusive",
        };
        f.write_str(name)
    }
}

/// A classifier's opinion of one image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriageScore {
    /// What the image most likely is.
    pub label: TriageLabel,
    /// The property that settled it.
    pub decided_by: Decision,
}

/// Identity of the decision procedure behind a classifier, for the manifest.
///
/// A scan records exactly what labelled it, so a result can be reproduced with
/// the same rules and nothing else (A-MODEL-PINNED). With no model file there
/// is no file hash to pin; what is pinned instead is the version of the rules,
/// which lives in the source tree the binary was built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelIdentity {
    /// Human-readable version of the decision procedure.
    pub version: &'static str,
}

/// Scores decoded images: photograph vs synthetic asset.
///
/// The engine consults a classifier only after artifacts are persisted and
/// hashed; everything a classifier returns is an annotation on an existing
/// record. `None` means "no opinion" and leaves the artifact unscored — it is
/// never a reason to drop one (A-TRIAGE-NOT-VERDICT).
pub trait Classifier {
    /// What scoring fails with.
    type Error: Error + Send + Sync + 'static;

    /// Identity of the decision procedure behind this classifier, when there
    /// is one.
    fn model(&self) -> Option<ModelIdentity>;

    /// Scores a batch of images, one answer per image in order.
    ///
    /// The batch is the unit because the engine scores a whole scan's
    /// artifacts at once, after every one of them is stored and hashed.
    ///
    /// # Errors
    ///
    /// Fails when the classifier itself breaks, not on a property of any
    /// image. Per-image "no opinion" is `Ok(None)`.
    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error>;
}

/// The null classifier: no opinion, every artifact left as it is.
///
/// It is what names the classifier type of a scan that has none, so a run with
/// triage disabled goes through the same path as one with it and reports
/// everything, unscored (A-TRIAGE-NOT-VERDICT).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcceptAll;

impl Classifier for AcceptAll {
    type Error = Infallible;

    fn model(&self) -> Option<ModelIdentity> {
        None
    }

    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        Ok(vec![None; images.len()])
    }
}
/// Lifecycle state of a scan run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RunState {
    /// Work is progressing.
    #[default]
    Running,
    /// Work is suspended and will resume where it stopped.
    Paused,
    /// Work was stopped early; results so far are still reported.
    Cancelled,
    /// The run reached the end of its work.
    Finished,
}

impl RunState {
    /// Whether no further work will happen in this state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Finished)
    }
}

impl std::fmt::Display for RunState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Finished => "finished",
        };
        f.write_str(name)
    }
}

/// What a stage counts its work in.
///
/// A stage that reads the medium measures itself in bytes; one that examines
/// candidates or labels artifacts measures itself in those; one that ends on a
/// clock measures itself in seconds. Naming the unit is what lets a display say
/// `43%` for one without claiming a candidate count is a byte count — and what
/// lets it decline to say it for a unit that cannot support one.
///
/// Only [`Unit::Bytes`], [`Unit::Items`] and [`Unit::Seconds`] support a
/// percentage. [`Unit::Steps`] does not, and a display must not compute one
/// from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Unit {
    /// Bytes of the medium.
    #[default]
    Bytes,
    /// Whatever the stage handles one at a time: a candidate to validate, a
    /// fragment set to reassemble, an artifact to label.
    Items,
    /// Units of work, where one item costs more than one of them.
    ///
    /// A stage whose item is searched in several passes cannot report items
    /// without either standing still through a pass or counting an item twice.
    /// It counts steps instead, and says so: `3706 of 16321 steps` invites no
    /// arithmetic about how many items are left, which `3706 of 16321 items`
    /// does — wrongly, when an item costs three.
    ///
    /// Steps are not equal, and where the queue is ordered by how promising an
    /// item is rather than by what it costs, they are not even ordered: the
    /// field run behind `docs/defects/09` measured 3.68 s and 131.65 s per
    /// header in two regions of the same queue, with the expensive ones first.
    /// A fraction of the steps is therefore not a fraction of the time, and
    /// [`Unit::supports_percentage`] answers `false` here for that reason.
    Steps,
    /// Seconds of wall clock, for a stage that ends on a deadline rather than
    /// on an amount of work.
    ///
    /// Reassembly is the one. Its budget is wall-clock because a decode's cost
    /// is not a constant and the stage cannot tell which case it is in until it
    /// is there, so how far it has got through its queue says nothing about how
    /// long it has left. Elapsed against that budget says exactly that, and
    /// reaches its end when the stage does.
    Seconds,
}

impl Unit {
    /// Whether `done` out of `total` of this unit is a fraction a display may
    /// show as a percentage.
    ///
    /// False for [`Unit::Steps`], whose units cost different amounts and are
    /// not handed out cheapest-first. A display that shows one anyway reports a
    /// run doing its heaviest work as barely started, which is what the run in
    /// `docs/defects/09` was cancelled for.
    #[must_use]
    pub const fn supports_percentage(self) -> bool {
        match self {
            Self::Bytes | Self::Items | Self::Seconds => true,
            Self::Steps => false,
        }
    }
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bytes => "bytes",
            Self::Items => "items",
            Self::Steps => "steps",
            Self::Seconds => "seconds",
        })
    }
}

/// One structured progress event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanEvent {
    /// A stage began.
    ///
    /// Emitted by every stage, including one that will never report progress,
    /// because a stage that cannot say how far along it is must at least be
    /// able to say that it is the one running. Silence between two stages is
    /// indistinguishable from a stall.
    StageStarted {
        /// Stage that began.
        stage: Stage,
        /// What `total` counts.
        unit: Unit,
        /// Work the stage expects to cover, zero when not known ahead.
        total: u64,
    },
    /// Cumulative progress within a stage, emitted once per chunk of work.
    StageProgress {
        /// Stage reporting progress.
        stage: Stage,
        /// What `done` and `total` count.
        unit: Unit,
        /// Work processed so far.
        done: u64,
        /// Work the stage expects to cover, zero when not known ahead.
        total: u64,
    },
    /// A stage ended, having produced `findings` results.
    StageFinished {
        /// Stage that ended.
        stage: Stage,
        /// Findings the stage contributed.
        findings: u64,
    },
    /// The run changed lifecycle state.
    StateChanged {
        /// State entered.
        state: RunState,
    },
    /// A region of the medium could not be read and is reported unrecovered,
    /// never fabricated (A-CONFIDENCE-HONEST).
    RegionUnreadable {
        /// Byte range that failed to read.
        range: ByteRange,
    },
    /// An artifact reached the sink. Counts are cumulative for the run.
    ///
    /// Emitted once per artifact, which is the granularity a recovery happens
    /// at — not per sector and not per candidate (`M-LOG-OVERHEAD`). It exists
    /// so a display can say what has actually been recovered while it is
    /// happening, rather than only once the stage ends.
    ///
    /// Both figures describe artifacts **stored**, never candidates seen: a
    /// signature hit that has not passed its format's state machine is not a
    /// recovery, and counting one as such would overstate the result
    /// (A-CONFIDENCE-HONEST).
    ArtifactStored {
        /// Artifacts handed to the sink so far.
        artifacts: u64,
        /// Sum of their lengths in bytes.
        bytes: u64,
    },
}

/// Receives [`ScanEvent`]s from a running scan.
///
/// Sinks are shared across the pipeline's threads, so `emit` takes `&self` and
/// implementations must be cheap: a slow sink stalls the scan.
pub trait ProgressSink: Send + Sync {
    /// Handles one event.
    fn emit(&self, event: ScanEvent);
}

/// A [`ProgressSink`] that drops every event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Discard;

impl ProgressSink for Discard {
    fn emit(&self, _event: ScanEvent) {}
}

impl<T: ProgressSink + ?Sized> ProgressSink for &T {
    fn emit(&self, event: ScanEvent) {
        (**self).emit(event);
    }
}
