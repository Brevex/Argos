//! Structure-validated signature carving over `impl Read + Seek`.
//!
//! [`Carver`] sweeps a source for image signatures and drives every candidate
//! through a full format state machine ([`jpeg`], [`png`]) — a magic-byte hit
//! alone never becomes a finding. Everything here is sans-IO and contains no
//! `unsafe`; disk content is untrusted input, so all lengths and offsets read
//! from the medium are bounds-checked before use.
//!
//! Performance note: the signature sweep in [`Carver::scan`] is the project's
//! hot path (bytes/second over whole devices). It reads large windows, reuses
//! its buffers across windows and candidates, and multi-pattern-matches
//! signatures; keep allocations out of that loop.
//!
//! What carving cannot do: a candidate whose stream breaks mid-file is counted
//! as rejected, not recovered — fragmented images need the reassembly stage,
//! and overwritten bytes are gone. Overstating that would be a correctness bug.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

use argos_core::Confidence;
use argos_core::geometry::ByteOffset;
use memchr::memmem::Finder;

pub mod assemble;
pub mod classify;
pub mod decode;
pub mod exif;
pub mod jpeg;
pub mod mcu;
pub mod png;
pub mod reassemble;

mod stream;

#[cfg(feature = "test-util")]
pub mod fixture;

/// The format vocabulary is shared workspace-wide; carving is where a format
/// is *decided*, not where it is defined.
pub use argos_core::Format;

/// Upper bound on a single carved image, in bytes.
///
/// Chosen far above any real photograph (a 200-megapixel uncompressed TIFF
/// stays under 1 GiB; JPEG/PNG photos are orders of magnitude smaller) so a
/// corrupt length field or a missing terminator cannot make a validator walk
/// the whole device.
pub const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// Bytes per signature-scan window. 4 MiB keeps the sweep in large sequential
/// reads (the throughput driver on HDDs) while the window buffer stays small
/// enough to live comfortably per worker.
const SCAN_WINDOW_BYTES: usize = 4 * 1024 * 1024;

/// JPEG start-of-image marker followed by the first marker-introducing byte.
///
/// Every JFIF/EXIF file begins with these bytes. Changing this breaks
/// detection of all JPEG variants. Source: ITU-T T.81, Annex B.1.
const JPEG_SOI: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// PNG file signature. Source: RFC 2083 / ISO 15948 §5.2.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Overlap carried between scan windows so a signature straddling a window
/// boundary is still found: longest signature length minus one.
const WINDOW_OVERLAP: usize = PNG_SIGNATURE.len() - 1;

/// Bytes a caller must carry between consecutive buffers handed to
/// [`Detector::hits_in`], so a signature straddling the boundary is still
/// found. Feeding disjoint buffers silently loses those candidates.
pub const SIGNATURE_OVERLAP_BYTES: usize = WINDOW_OVERLAP;

/// An unvalidated signature hit: a position that *might* start an image.
///
/// A candidate becomes a [`Finding`] only by passing [`validate`]; a
/// magic-byte hit on its own is never evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Candidate {
    /// Absolute byte position of the signature in the medium.
    pub offset: ByteOffset,
    /// Format whose signature matched.
    pub format: Format,
}

/// Multi-pattern signature matcher over in-memory buffers.
///
/// This is the surface-rate half of carving: it runs over every byte of the
/// medium, so it does no I/O, allocates nothing per hit beyond the caller's
/// output vector, and matches all signatures in one pass.
#[derive(Debug)]
pub struct Detector {
    jpeg_header: Finder<'static>,
    png_header: Finder<'static>,
}

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector {
    /// A detector for every format Argos carves.
    #[must_use]
    pub fn new() -> Self {
        Self {
            jpeg_header: Finder::new(&JPEG_SOI),
            png_header: Finder::new(&PNG_SIGNATURE),
        }
    }

    /// Appends every signature hit in `window` to `out`, ordered by offset.
    ///
    /// `start` is the absolute position of `window[0]`; consecutive windows
    /// must overlap by [`SIGNATURE_OVERLAP_BYTES`].
    ///
    /// The cap matters: a medium whose free space is a repeated signature —
    /// trivial to produce deliberately — yields a hit every few bytes, and an
    /// uncapped collector would exhaust memory before the scan could report
    /// anything at all (A-BOUNDED-ALLOC). Returns whether every hit fit.
    pub fn hits_in(
        &self,
        window: &[u8],
        start: ByteOffset,
        cap: usize,
        out: &mut Vec<Candidate>,
    ) -> bool {
        let mark = out.len();
        let at = |i: usize, format| Candidate {
            offset: ByteOffset::new(start.get().saturating_add(i as u64)),
            format,
        };
        out.extend(
            self.jpeg_header
                .find_iter(window)
                .map(|i| at(i, Format::Jpeg)),
        );
        out.extend(
            self.png_header
                .find_iter(window)
                .map(|i| at(i, Format::Png)),
        );
        out[mark..].sort_unstable();
        if out.len() <= cap {
            return true;
        }
        out.truncate(cap);
        false
    }
}

/// Bytes of a buffer [`identify`] needs to reach a verdict.
pub const MAX_SIGNATURE_BYTES: usize = PNG_SIGNATURE.len();

/// The format `prefix` signals, if any.
///
/// This is a signature test, not validation: it answers "could these bytes
/// begin an image?" and nothing more. A caller that needs an answer it can
/// report must run [`validate`].
#[must_use]
pub fn identify(prefix: &[u8]) -> Option<Format> {
    if prefix.starts_with(&PNG_SIGNATURE) {
        Some(Format::Png)
    } else if prefix.starts_with(&JPEG_SOI) {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// Re-validates an embedded thumbnail as a standalone JPEG, returning its
/// confirmed length.
///
/// `None` when the thumbnail does not itself validate: a corrupt thumbnail is
/// not evidence, even when the EXIF metadata pointing at it parsed cleanly.
/// `end` is the medium end, which caps the read.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn validate_thumbnail<R: Read + Seek>(
    src: &mut R,
    thumbnail: Thumbnail,
    end: u64,
    scratch: &mut Scratch,
) -> Result<Option<u64>, CarveError> {
    let limit = end.min(thumbnail.offset.get().saturating_add(thumbnail.length));
    match jpeg::validate(src, thumbnail.offset, limit, scratch)? {
        Verdict::Complete { length, .. } => Ok(Some(length)),
        Verdict::Corrupt { .. } => Ok(None),
    }
}

/// Validates the candidate at `start` as `format`.
///
/// Consumes at most `limit - start` bytes of `src`; callers cap `limit` at the
/// medium end and at [`MAX_IMAGE_BYTES`].
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails. A structural violation is a
/// [`Verdict::Corrupt`], not an error — corruption is the expected input.
pub fn validate<R: Read + Seek>(
    format: Format,
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    scratch: &mut Scratch,
) -> Result<Verdict, CarveError> {
    match format {
        Format::Jpeg => jpeg::validate(src, start, limit, scratch),
        Format::Png => png::validate(src, start, limit, scratch),
    }
}

/// A validated image located in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Image format the candidate validated as.
    pub format: Format,
    /// Absolute byte position of the first byte in the source.
    pub offset: ByteOffset,
    /// Length in bytes.
    pub length: u64,
    /// Evidence tier: `ContiguousCarve` for full images, `PartialOrThumbnail`
    /// for embedded thumbnails.
    pub confidence: Confidence,
    /// For an embedded thumbnail, the offset of the candidate it was found in.
    pub parent: Option<ByteOffset>,
}

/// Outcome of one [`Carver::scan`] pass.
#[derive(Clone, Debug)]
pub struct Scan {
    /// Validated findings, ordered by offset.
    pub findings: Vec<Finding>,
    /// Signature hits that failed structural validation. Rejected candidates
    /// are not recoverable by contiguous carving; fragment reassembly may
    /// still recover them in a later stage.
    pub rejected: u64,
}

/// Verdict of a format validator on one candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The candidate validated end-to-end as one contiguous image.
    Complete {
        /// Total image length in bytes from the candidate start.
        length: u64,
        /// Embedded EXIF thumbnail, when present and in range.
        thumbnail: Option<Thumbnail>,
    },
    /// The structure broke; `at` is the fragmentation point.
    Corrupt {
        /// Earliest absolute offset at which the corruption can begin: exact
        /// for structural violations, the chunk data start for a PNG CRC
        /// mismatch (a CRC cannot localize damage within its chunk).
        at: ByteOffset,
        /// Embedded EXIF thumbnail parsed before the break, when any — a
        /// thumbnail may survive when its parent does not.
        thumbnail: Option<Thumbnail>,
    },
}

/// Location of an embedded EXIF thumbnail in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    /// Absolute byte position of the thumbnail's first byte.
    pub offset: ByteOffset,
    /// Thumbnail length in bytes.
    pub length: u64,
}

/// Reusable validator working memory, so scanning many candidates does not
/// allocate per candidate.
#[derive(Debug, Default)]
pub struct Scratch {
    pub(crate) stream: Vec<u8>,
    pub(crate) seg: Vec<u8>,
    pub(crate) inflate_out: Vec<u8>,
}

impl Scratch {
    /// Fresh working memory; buffers grow on first use and are reused after.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Reading the source failed while carving.
#[derive(Debug)]
pub struct CarveError {
    offset: ByteOffset,
    source: io::Error,
    backtrace: Backtrace,
}

impl CarveError {
    pub(crate) fn io(offset: ByteOffset, source: io::Error) -> Self {
        Self {
            offset,
            source,
            backtrace: Backtrace::capture(),
        }
    }

    /// Byte position the failed read concerned.
    #[must_use]
    pub fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for CarveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot read source at byte {}: {}",
            self.offset, self.source
        )?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for CarveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Signature sweeper plus per-candidate validation.
///
/// One `Carver` owns its buffers and is reused across scans; create one per
/// worker thread.
#[derive(Debug)]
pub struct Carver {
    detector: Detector,
    window: Vec<u8>,
    hits: Vec<Candidate>,
    scratch: Scratch,
}

impl Default for Carver {
    fn default() -> Self {
        Self::new()
    }
}

impl Carver {
    /// A carver with empty reusable buffers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            detector: Detector::new(),
            window: Vec::new(),
            hits: Vec::new(),
            scratch: Scratch::new(),
        }
    }

    /// Sweeps `src` end to end and returns every image that validates.
    ///
    /// Signature hits inside an already-validated image are skipped (embedded
    /// thumbnails are reported through their parent instead), so one validated
    /// file is never double-reported.
    ///
    /// # Errors
    ///
    /// Fails only when reading or seeking `src` fails; structurally invalid
    /// candidates are counted in [`Scan::rejected`], not raised.
    pub fn scan<R: Read + Seek>(&mut self, src: &mut R) -> Result<Scan, CarveError> {
        let end = src
            .seek(SeekFrom::End(0))
            .map_err(|source| CarveError::io(ByteOffset::new(0), source))?;

        let mut findings = Vec::new();
        let mut rejected = 0_u64;
        // First byte not covered by an already-validated image.
        let mut resume = 0_u64;
        let mut pos = 0_u64;

        while pos < end {
            let want = usize::try_from((end - pos).min(SCAN_WINDOW_BYTES as u64))
                .unwrap_or(SCAN_WINDOW_BYTES);
            self.window.resize(want, 0);
            src.seek(SeekFrom::Start(pos))
                .and_then(|_| src.read_exact(&mut self.window))
                .map_err(|source| CarveError::io(ByteOffset::new(pos), source))?;

            self.hits.clear();
            // One window's worth of hits is already bounded by the window.
            let cap = self.window.len();
            self.detector
                .hits_in(&self.window, ByteOffset::new(pos), cap, &mut self.hits);
            for i in 0..self.hits.len() {
                let Candidate { offset, format } = self.hits[i];
                let abs = offset.get();
                if abs < resume {
                    continue;
                }
                let limit = end.min(abs.saturating_add(MAX_IMAGE_BYTES));
                let verdict = validate(format, src, offset, limit, &mut self.scratch)?;
                match verdict {
                    Verdict::Complete { length, thumbnail } => {
                        findings.push(Finding {
                            format,
                            offset: ByteOffset::new(abs),
                            length,
                            confidence: Confidence::ContiguousCarve,
                            parent: None,
                        });
                        self.push_thumbnail(src, thumbnail, abs, end, &mut findings)?;
                        resume = abs + length;
                    }
                    Verdict::Corrupt { thumbnail, .. } => {
                        rejected += 1;
                        self.push_thumbnail(src, thumbnail, abs, end, &mut findings)?;
                    }
                }
            }

            // Step so a signature straddling the window edge lands whole in
            // the next window.
            if pos + want as u64 >= end {
                break;
            }
            pos += (want - WINDOW_OVERLAP) as u64;
        }

        merge_duplicate_extents(&mut findings);
        findings.shrink_to_fit();
        Ok(Scan { findings, rejected })
    }

    /// Re-validates a thumbnail range as a standalone JPEG and records it only
    /// if it is itself intact — a corrupt thumbnail is not evidence.
    fn push_thumbnail<R: Read + Seek>(
        &mut self,
        src: &mut R,
        thumbnail: Option<Thumbnail>,
        parent: u64,
        end: u64,
        findings: &mut Vec<Finding>,
    ) -> Result<(), CarveError> {
        let Some(thumb) = thumbnail else {
            return Ok(());
        };
        if let Some(length) = validate_thumbnail(src, thumb, end, &mut self.scratch)? {
            findings.push(Finding {
                format: Format::Jpeg,
                offset: thumb.offset,
                length,
                confidence: Confidence::PartialOrThumbnail,
                parent: Some(ByteOffset::new(parent)),
            });
        }
        Ok(())
    }
}

/// Deduplicates findings covering identical extents, keeping the highest
/// evidence tier and attaching the other's parent metadata — the same image
/// can be reached both through its parent's EXIF and by direct carving.
fn merge_duplicate_extents(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        (a.offset, a.length)
            .cmp(&(b.offset, b.length))
            .then(b.confidence.cmp(&a.confidence))
    });
    findings.dedup_by(|dropped, kept| {
        let same = dropped.offset == kept.offset && dropped.length == kept.length;
        if same && kept.parent.is_none() {
            kept.parent = dropped.parent;
        }
        same
    });
}
