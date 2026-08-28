//! A surviving image used as the header its lost siblings were written with.
//!
//! An entropy-coded fragment with no header is undecodable in principle: the
//! Huffman tables, the sampling factors and the frame geometry live in the
//! header and nothing in the fragment states them. The published attack
//! estimates them from a corpus wide enough to cover the camera in question
//! (Uzun & Sencar, IEEE TIFS 10(8), 2015). When a file from the same batch
//! survives, they are not estimated but known: one camera at one setting writes
//! one header, so a sibling's header *is* the lost file's header.
//!
//! What this module produces is pixels, never a file. See [`Reference::graft`].

use std::fmt;

use argos_core::ByteOffset;

/// Start of image.
const MARKER_SOI: u8 = 0xD8;
/// End of image.
const MARKER_EOI: u8 = 0xD9;
/// Start of scan: the last segment before the entropy-coded data.
const MARKER_SOS: u8 = 0xDA;
/// Baseline sequential frame.
const MARKER_SOF0: u8 = 0xC0;
/// Extended sequential frame, Huffman-coded.
const MARKER_SOF1: u8 = 0xC1;

/// Markers that stand alone: no length field follows them.
///
/// `RST0..RST7`, `SOI`, `EOI` and `TEM`. Source: ITU-T T.81 Table B.1.
const STANDALONE: std::ops::RangeInclusive<u8> = 0xD0..=0xD9;

/// Largest reference header accepted, in bytes.
///
/// A JPEG header is tables and a frame declaration: four quantization tables,
/// eight Huffman tables and a scan header come to a few kilobytes, and EXIF
/// with an embedded thumbnail rarely passes 64 KiB. The bound is what stops a
/// crafted file from being read into memory in full before it is rejected
/// (`A-BOUNDED-ALLOC`), and is independent of any length the file states.
pub(crate) const MAX_HEADER_BYTES: usize = 1 << 20;

/// Why a candidate cannot serve as a reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The bytes do not begin with `SOI`.
    NotJpeg,
    /// The markers ran out before a scan began.
    NoScan,
    /// The frame is progressive, arithmetic-coded, lossless or hierarchical.
    ///
    /// Each scan of a progressive frame carries its own parameters, so a prefix
    /// of one does not decode the data of another. Offering one would produce
    /// confident nonsense.
    NotSequential,
    /// A segment's length field runs past the end of the candidate.
    Truncated,
    /// The header alone exceeds `MAX_HEADER_BYTES`.
    HeaderTooLarge,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotJpeg => "does not start with a JPEG signature",
            Self::NoScan => "has no scan, so it carries no tables to lend",
            Self::NotSequential => "is not a baseline or extended sequential frame",
            Self::Truncated => "declares a segment longer than the file",
            Self::HeaderTooLarge => "has a header larger than this tool will hold",
        })
    }
}

/// A candidate rejected as a reference, and where the reading stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceError {
    /// What was wrong.
    pub fault: Fault,
    /// Offset within the candidate at which it was decided.
    pub at: ByteOffset,
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the reference {} (at byte {})",
            self.fault,
            self.at.get()
        )
    }
}

impl std::error::Error for ReferenceError {}

/// The header a batch of images shares, taken verbatim from one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// `SOI` through the end of the `SOS` segment, copied byte for byte.
    ///
    /// Never re-encoded: re-encoding is a chance to write a table that differs
    /// from the one the camera wrote, and that it does not is the whole value
    /// of a reference.
    prefix: Box<[u8]>,
    /// Frame dimensions the reference declares, width then height.
    dimensions: (u16, u16),
}

impl Reference {
    /// Reads the header of `candidate`, which must be a whole sequential JPEG.
    ///
    /// # Errors
    ///
    /// [`ReferenceError`] when the candidate is not a JPEG, is progressive or
    /// arithmetic-coded, declares a segment past its own end, or carries a
    /// header beyond `MAX_HEADER_BYTES`. Every one of these is a statement
    /// about the file the examiner chose, not about the medium.
    pub fn read(candidate: &[u8]) -> Result<Self, ReferenceError> {
        let fail = |fault, at: usize| ReferenceError {
            fault,
            at: ByteOffset::new(at as u64),
        };
        if candidate.first() != Some(&0xFF) || candidate.get(1) != Some(&MARKER_SOI) {
            return Err(fail(Fault::NotJpeg, 0));
        }

        let mut at = 2_usize;
        let mut dimensions = None;
        loop {
            if at >= MAX_HEADER_BYTES {
                return Err(fail(Fault::HeaderTooLarge, at));
            }
            // Fill bytes are legal between segments; skip them one at a time.
            let Some(&byte) = candidate.get(at) else {
                return Err(fail(Fault::NoScan, at));
            };
            if byte != 0xFF {
                return Err(fail(Fault::NoScan, at));
            }
            let Some(&marker) = candidate.get(at + 1) else {
                return Err(fail(Fault::NoScan, at + 1));
            };
            if marker == 0xFF {
                at += 1;
                continue;
            }
            if STANDALONE.contains(&marker) || marker == MARKER_EOI {
                return Err(fail(Fault::NoScan, at));
            }

            let Some(length) = segment_length(candidate, at + 2) else {
                return Err(fail(Fault::Truncated, at + 2));
            };
            let end = at
                .checked_add(2)
                .and_then(|start| start.checked_add(length))
                .ok_or_else(|| fail(Fault::Truncated, at))?;
            if end > candidate.len() {
                return Err(fail(Fault::Truncated, at));
            }

            match marker {
                MARKER_SOF0 | MARKER_SOF1 => {
                    // SOF payload: precision, height, width, component count.
                    let height =
                        be16(candidate, at + 5).ok_or_else(|| fail(Fault::Truncated, at))?;
                    let width =
                        be16(candidate, at + 7).ok_or_else(|| fail(Fault::Truncated, at))?;
                    dimensions = Some((width, height));
                }
                // Any other frame marker is a coding this tool cannot lend a
                // header for. T.81 Table B.1: 0xC0..=0xCF less DHT and DAC.
                0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                    return Err(fail(Fault::NotSequential, at));
                }
                MARKER_SOS => {
                    let Some(dimensions) = dimensions else {
                        return Err(fail(Fault::NotSequential, at));
                    };
                    if end > MAX_HEADER_BYTES {
                        return Err(fail(Fault::HeaderTooLarge, end));
                    }
                    return Ok(Self {
                        prefix: candidate[..end].into(),
                        dimensions,
                    });
                }
                _ => {}
            }
            at = end;
        }
    }

    /// Frame dimensions the reference declares, width then height.
    #[must_use]
    pub const fn dimensions(&self) -> (u16, u16) {
        self.dimensions
    }

    /// The header bytes a graft prepends.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// Builds a decodable JPEG from this header and an orphan's entropy bytes.
    ///
    /// The result is **pixels, not a file**. The frame declares the reference's
    /// dimensions, the strip's position inside that frame is unknown, and these
    /// bytes in this order never existed on the medium. A caller reports one at
    /// the weakest tier, naming the reference it was grafted onto, and never as
    /// a recovered file (`A-CONFIDENCE-HONEST`).
    #[must_use]
    pub fn graft(&self, entropy: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.prefix.len() + entropy.len() + 2);
        out.extend_from_slice(&self.prefix);
        out.extend_from_slice(entropy);
        out.extend_from_slice(&[0xFF, MARKER_EOI]);
        out
    }
}

/// Big-endian `u16` at `at`, or `None` if it does not fit.
fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([pair[0], pair[1]]))
}

/// Payload length of the segment whose length field starts at `at`.
///
/// The field counts itself, so a segment shorter than its own field is
/// malformed and reads as no length at all.
fn segment_length(bytes: &[u8], at: usize) -> Option<usize> {
    let declared = usize::from(be16(bytes, at)?);
    (declared >= 2).then_some(declared)
}
