//! JPEG validation state machine (ITU-T T.81, Annex B).
//!
//! Walks a candidate as SOI → marker segments → SOS → entropy-coded scan →
//! EOI, byte by byte. Every segment length is bounds-checked before use, the
//! entropy stream is tracked for byte stuffing and restart-marker cadence, and
//! the first structural violation becomes a [`Verdict::Corrupt`] carrying the
//! exact fragmentation point. Progressive JPEGs (multiple scans) are legal.
//!
//! EXIF `APP1` segments are additionally mined for an embedded thumbnail,
//! which survives as a separate lower-tier artifact even when the parent
//! image breaks later.

use std::io::{Read, Seek};

use argos_core::ByteOffset;

use crate::Bytes;
use crate::{CarveError, Scratch, Thumbnail, Verdict};

/// Marker codes from ITU-T T.81, Table B.1. Only the codes the state machine
/// branches on are named; `SOF` variants are matched as a range.
pub(crate) const MARKER_SOI: u8 = 0xD8;
pub(crate) const MARKER_EOI: u8 = 0xD9;
pub(crate) const MARKER_SOS: u8 = 0xDA;
const MARKER_DRI: u8 = 0xDD;
pub(crate) const MARKER_APP1: u8 = 0xE1;
/// First and last restart markers `RST0`..`RST7`.
const MARKER_RST0: u8 = 0xD0;
const MARKER_RST7: u8 = 0xD7;

/// `APP1` payloads start with this prefix when they carry EXIF metadata.
/// Source: EXIF 2.32 §4.7.2.
pub(crate) const EXIF_HEADER: [u8; 6] = *b"Exif\0\0";

/// Validates the JPEG candidate starting at `start`.
///
/// Consumes at most `limit - start` bytes of `src`; the caller caps `limit` at
/// the medium end and [`crate::MAX_IMAGE_BYTES`].
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails. Structural violations are
/// a [`Verdict::Corrupt`], not an error — corruption is the expected input.
pub fn validate<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    scratch: &mut Scratch,
) -> Result<Verdict, CarveError> {
    let Scratch { stream, seg, .. } = scratch;
    let mut bytes = Bytes::new(src, start.get(), limit, stream);
    let mut thumbnail: Option<Thumbnail> = None;

    let corrupt = |at: u64, thumbnail: Option<Thumbnail>| Verdict::Corrupt {
        at: ByteOffset::new(at),
        thumbnail,
    };

    // SOI.
    if next(&mut bytes)? != Some(0xFF) || next(&mut bytes)? != Some(MARKER_SOI) {
        return Ok(corrupt(start.get(), None));
    }

    let mut restart_interval = 0_u16;
    let mut seen_sof = false;
    // A marker code consumed by the entropy scan that still needs handling.
    let mut pending: Option<u8> = None;

    loop {
        let code = if let Some(code) = pending.take() {
            code
        } else {
            if next(&mut bytes)? != Some(0xFF) {
                return Ok(corrupt(bytes.pos().saturating_sub(1), thumbnail));
            }
            match fill_bytes_then_code(&mut bytes)? {
                Some(code) => code,
                None => return Ok(corrupt(bytes.pos(), thumbnail)),
            }
        };

        match code {
            MARKER_SOS => {
                if !seen_sof {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                }
                if !skip_segment(&mut bytes)? {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                }
                match entropy_scan(&mut bytes, restart_interval)? {
                    ScanEnd::Eoi => {
                        return Ok(Verdict::Complete {
                            length: bytes.pos() - start.get(),
                            thumbnail,
                        });
                    }
                    ScanEnd::Marker(code) => pending = Some(code),
                    ScanEnd::Corrupt(at) => return Ok(corrupt(at, thumbnail)),
                }
            }
            MARKER_DRI => {
                let Some(len) = read_len(&mut bytes)? else {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                };
                if len != 4 {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                }
                let (Some(hi), Some(lo)) = (next(&mut bytes)?, next(&mut bytes)?) else {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                };
                restart_interval = u16::from_be_bytes([hi, lo]);
            }
            MARKER_APP1 => {
                let Some(payload_start) = read_segment(&mut bytes, seg)? else {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                };
                if thumbnail.is_none() {
                    thumbnail = exif_thumbnail(seg, payload_start, limit);
                }
            }
            // SOF0..SOF15, excluding DHT (C4), JPG (C8) and DAC (CC) which are
            // plain length segments.
            0xC0..=0xCF if code != 0xC4 && code != 0xC8 && code != 0xCC => {
                seen_sof = true;
                if !skip_segment(&mut bytes)? {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                }
            }
            // DHT, JPG, DAC, DQT, DNL, other APPn, COM, JPGn extensions.
            0xC4 | 0xC8 | 0xCC | 0xDB | 0xDC | 0xE0 | 0xE2..=0xEF | 0xF0..=0xFD | 0xFE => {
                if !skip_segment(&mut bytes)? {
                    return Ok(corrupt(bytes.pos(), thumbnail));
                }
            }
            // Nested SOI, bare EOI without a scan, stray RST/TEM, or anything
            // else: not a legal segment sequence. Point at the 0xFF that
            // introduced the illegal marker.
            _ => return Ok(corrupt(bytes.pos().saturating_sub(2), thumbnail)),
        }
    }
}

/// How an entropy-coded scan ended.
enum ScanEnd {
    /// `EOI` reached; the image is complete.
    Eoi,
    /// A non-restart marker ended the scan (progressive: more segments follow).
    Marker(u8),
    /// The stream broke at this absolute offset.
    Corrupt(u64),
}

/// Tracks the entropy-coded stream: `0xFF` must introduce stuffing, a restart
/// marker in cyclic order, a fill byte, or a legal marker.
fn entropy_scan<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    restart_interval: u16,
) -> Result<ScanEnd, CarveError> {
    let mut expected_rst = 0_u8;
    loop {
        let Some(byte) = next(bytes)? else {
            return Ok(ScanEnd::Corrupt(bytes.pos()));
        };
        if byte != 0xFF {
            continue;
        }
        let Some(code) = fill_bytes_then_code(bytes)? else {
            return Ok(ScanEnd::Corrupt(bytes.pos()));
        };
        match code {
            0x00 => {}
            MARKER_RST0..=MARKER_RST7 => {
                let index = code - MARKER_RST0;
                if restart_interval == 0 || index != expected_rst {
                    // Point at the 0xFF that introduced the illegal marker.
                    return Ok(ScanEnd::Corrupt(bytes.pos().saturating_sub(2)));
                }
                // Restart markers cycle RST0..RST7. Source: T.81 §B.2.1.
                expected_rst = (expected_rst + 1) % 8;
            }
            MARKER_EOI => return Ok(ScanEnd::Eoi),
            // Any other marker legally ends the scan; the segment loop decides
            // whether it is a valid continuation.
            _ => return Ok(ScanEnd::Marker(code)),
        }
    }
}

/// Reads a marker code, tolerating `0xFF` fill bytes (T.81 §B.1.1.2).
fn fill_bytes_then_code<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
) -> Result<Option<u8>, CarveError> {
    loop {
        match next(bytes)? {
            Some(0xFF) => {}
            other => return Ok(other),
        }
    }
}

/// Reads a segment length field; `None` on truncation or an illegal length.
fn read_len<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u16>, CarveError> {
    let (Some(hi), Some(lo)) = (next(bytes)?, next(bytes)?) else {
        return Ok(None);
    };
    let len = u16::from_be_bytes([hi, lo]);
    // The length counts its own two bytes. Source: T.81 §B.1.1.4.
    if len < 2 { Ok(None) } else { Ok(Some(len)) }
}

/// Skips a length-prefixed segment; `false` on truncation.
fn skip_segment<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<bool, CarveError> {
    let Some(len) = read_len(bytes)? else {
        return Ok(false);
    };
    Ok(bytes.skip(u64::from(len) - 2))
}

/// Reads a length-prefixed segment payload into `seg`; returns the payload's
/// absolute start offset, or `None` on truncation.
fn read_segment<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    seg: &mut Vec<u8>,
) -> Result<Option<u64>, CarveError> {
    let Some(len) = read_len(bytes)? else {
        return Ok(None);
    };
    let start = bytes.pos();
    seg.clear();
    // Bounded by the u16 length field: at most 65533 bytes.
    let ok = bytes
        .read_into(seg, usize::from(len) - 2)
        .map_err(|source| CarveError::io(ByteOffset::new(start), source))?;
    Ok(ok.then_some(start))
}

/// Maps an EXIF thumbnail found in an `APP1` payload to absolute offsets,
/// discarding anything that reaches past the candidate limit.
fn exif_thumbnail(payload: &[u8], payload_start: u64, limit: u64) -> Option<Thumbnail> {
    let tiff = payload.get(EXIF_HEADER.len()..)?;
    if payload[..EXIF_HEADER.len()] != EXIF_HEADER {
        return None;
    }
    let found = exif::thumbnail(tiff)?;
    let offset = payload_start
        .checked_add(EXIF_HEADER.len() as u64)?
        .checked_add(found.offset as u64)?;
    let end = offset.checked_add(found.length as u64)?;
    if found.length == 0 || end > limit {
        return None;
    }
    Some(Thumbnail {
        offset: ByteOffset::new(offset),
        length: found.length as u64,
    })
}

/// One byte from the cursor, with I/O failures mapped to [`CarveError`].
fn next<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u8>, CarveError> {
    let at = bytes.pos();
    bytes
        .next()
        .map_err(|source| CarveError::io(ByteOffset::new(at), source))
}

pub mod exif {
    //! Minimal TIFF/IFD walker for EXIF thumbnails (EXIF 2.32 §4.6).
    //!
    //! Reads exactly enough of the TIFF structure inside a JPEG `APP1` payload to
    //! locate the IFD1 thumbnail: header, IFD0 entry table, the next-IFD pointer,
    //! and IFD1's `JPEGInterchangeFormat`/`JPEGInterchangeFormatLength` tags.
    //! All offsets come from the medium and are validated against the payload
    //! before use; the walk visits at most two IFDs, so a crafted chain cannot
    //! loop.

    use argos_core::ports::Capture;

    /// TIFF byte-order marks. Source: TIFF 6.0 §2.
    const BYTE_ORDER_LE: [u8; 2] = *b"II";
    const BYTE_ORDER_BE: [u8; 2] = *b"MM";

    /// TIFF magic number following the byte-order mark. Source: TIFF 6.0 §2.
    const TIFF_MAGIC: u16 = 42;

    /// IFD1 tag holding the thumbnail's offset from the TIFF header.
    /// Source: EXIF 2.32, Table 17 (`JPEGInterchangeFormat`).
    const TAG_THUMBNAIL_OFFSET: u16 = 0x0201;

    /// IFD1 tag holding the thumbnail's byte length.
    /// Source: EXIF 2.32, Table 17 (`JPEGInterchangeFormatLength`).
    const TAG_THUMBNAIL_LENGTH: u16 = 0x0202;

    /// IFD0 tags naming the equipment. Source: EXIF 2.32, Table 4.
    const TAG_MAKE: u16 = 0x010F;
    const TAG_MODEL: u16 = 0x0110;

    /// IFD0 tag holding the file's own timestamp. Source: EXIF 2.32, Table 4.
    const TAG_DATETIME: u16 = 0x0132;

    /// IFD0 tag pointing at the Exif private IFD. Source: EXIF 2.32, Table 4.
    const TAG_EXIF_IFD: u16 = 0x8769;

    /// Exif-IFD tag holding when the picture was taken.
    /// Source: EXIF 2.32, Table 8 (`DateTimeOriginal`).
    const TAG_DATETIME_ORIGINAL: u16 = 0x9003;

    /// Exif-IFD tags holding the frame's pixel dimensions.
    /// Source: EXIF 2.32, Table 8 (`PixelXDimension`/`PixelYDimension`).
    const TAG_PIXEL_WIDTH: u16 = 0xA002;
    const TAG_PIXEL_HEIGHT: u16 = 0xA003;

    /// TIFF field types this walker reads. Source: TIFF 6.0 §2, Table 2.
    const TYPE_SHORT: u16 = 3;
    const TYPE_LONG: u16 = 4;
    const TYPE_ASCII: u16 = 2;

    /// Bytes per IFD entry: tag, type, count, value/offset. Source: TIFF 6.0 §2.
    const IFD_ENTRY_BYTES: usize = 12;

    /// Bytes of an IFD entry's value that are stored inline. Source: TIFF 6.0 §2 —
    /// a value of four bytes or fewer sits in the entry, longer ones are pointed at.
    const INLINE_VALUE_BYTES: usize = 4;

    /// Longest text field kept from a record.
    ///
    /// `Make` and `Model` are short by convention and `DateTimeOriginal` is exactly
    /// twenty bytes; this bounds what a length read from the medium can allocate
    /// (`A-BOUNDED-ALLOC`) at far more than any real value needs.
    const MAX_TEXT_BYTES: usize = 128;

    /// IFDs one walk may visit.
    ///
    /// IFD0, IFD1 and the Exif private IFD. A fixed count is what stops a crafted
    /// chain from looping, whatever its pointers say.
    const MAX_IFDS: usize = 3;

    /// An embedded thumbnail located inside a TIFF payload.
    ///
    /// Offsets are relative to the start of the TIFF header (the byte-order mark),
    /// exactly as stored on the medium.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct TiffThumbnail {
        /// Thumbnail start, relative to the TIFF header.
        pub offset: usize,
        /// Thumbnail length in bytes.
        pub length: usize,
    }

    /// Locates the IFD1 thumbnail in `tiff` (an `APP1` payload after `Exif\0\0`).
    ///
    /// Returns `None` for anything malformed, out of range, or absent — a corrupt
    /// EXIF block is expected input, never an error or a panic.
    #[must_use]
    pub fn thumbnail(tiff: &[u8]) -> Option<TiffThumbnail> {
        let le = match tiff.get(..2)? {
            b if *b == BYTE_ORDER_LE => true,
            b if *b == BYTE_ORDER_BE => false,
            _ => return None,
        };
        if read_u16(tiff, 2, le)? != TIFF_MAGIC {
            return None;
        }
        let ifd0 = usize::try_from(read_u32(tiff, 4, le)?).ok()?;

        // IFD0: skip its entries to reach the next-IFD pointer.
        let ifd0_count = usize::from(read_u16(tiff, ifd0, le)?);
        let next_ptr = ifd0
            .checked_add(2)?
            .checked_add(ifd0_count.checked_mul(IFD_ENTRY_BYTES)?)?;
        let ifd1 = usize::try_from(read_u32(tiff, next_ptr, le)?).ok()?;
        if ifd1 == 0 {
            return None;
        }

        // IFD1: scan entries for the thumbnail tags.
        let ifd1_count = usize::from(read_u16(tiff, ifd1, le)?);
        let mut offset = None;
        let mut length = None;
        for index in 0..ifd1_count {
            let entry = ifd1
                .checked_add(2)?
                .checked_add(index.checked_mul(IFD_ENTRY_BYTES)?)?;
            let value_at = entry.checked_add(8)?;
            let tag = read_u16(tiff, entry, le)?;
            if tag == TAG_THUMBNAIL_OFFSET {
                offset = Some(usize::try_from(read_u32(tiff, value_at, le)?).ok()?);
            } else if tag == TAG_THUMBNAIL_LENGTH {
                length = Some(usize::try_from(read_u32(tiff, value_at, le)?).ok()?);
            }
        }
        let (offset, length) = (offset?, length?);

        // The thumbnail must lie inside the payload we actually have.
        let end = offset.checked_add(length)?;
        if length == 0 || end > tiff.len() {
            return None;
        }
        Some(TiffThumbnail { offset, length })
    }

    /// Reads what `tiff` records about the picture and the camera.
    ///
    /// Returns an empty [`Capture`] for anything malformed, out of range or
    /// absent — a corrupt EXIF block is expected input, never an error or a panic.
    #[must_use]
    pub fn metadata(tiff: &[u8]) -> Capture {
        let mut found = Capture::default();
        let Some(le) = byte_order(tiff) else {
            return found;
        };
        let Some(ifd0) = read_u32(tiff, 4, le).and_then(|at| usize::try_from(at).ok()) else {
            return found;
        };

        // IFD0, then the Exif private IFD it points at. Bounded by a count rather
        // than by where the pointers lead.
        let mut visit = [Some(ifd0), None, None];
        for index in 0..MAX_IFDS {
            let Some(Some(ifd)) = visit.get(index).copied() else {
                continue;
            };
            let Some(count) = read_u16(tiff, ifd, le).map(usize::from) else {
                continue;
            };
            for entry in 0..count {
                let Some(at) = ifd
                    .checked_add(2)
                    .and_then(|base| entry.checked_mul(IFD_ENTRY_BYTES)?.checked_add(base))
                else {
                    break;
                };
                let (Some(tag), Some(kind)) = (read_u16(tiff, at, le), read_u16(tiff, at + 2, le))
                else {
                    break;
                };
                match tag {
                    TAG_MAKE => found.make = text(tiff, at, le),
                    TAG_MODEL => found.model = text(tiff, at, le),
                    TAG_DATETIME => found.modified = text(tiff, at, le),
                    TAG_DATETIME_ORIGINAL => found.taken = text(tiff, at, le),
                    TAG_PIXEL_WIDTH | TAG_PIXEL_HEIGHT => {
                        let value = number(tiff, at, kind, le);
                        let (width, height) = found.pixels.unwrap_or_default();
                        found.pixels = match (tag, value) {
                            (TAG_PIXEL_WIDTH, Some(value)) => Some((value, height)),
                            (_, Some(value)) => Some((width, value)),
                            (_, None) => found.pixels,
                        };
                    }
                    TAG_EXIF_IFD => {
                        if let Some(slot) = visit.get_mut(index + 1) {
                            *slot = read_u32(tiff, at + 8, le)
                                .and_then(|offset| usize::try_from(offset).ok());
                        }
                    }
                    _ => {}
                }
            }
        }
        // A dimension pair is only meaningful whole.
        if found
            .pixels
            .is_some_and(|(width, height)| width == 0 || height == 0)
        {
            found.pixels = None;
        }
        found
    }

    /// The byte order `tiff` declares, when it declares a TIFF at all.
    fn byte_order(tiff: &[u8]) -> Option<bool> {
        let le = match tiff.get(..2)? {
            b if *b == BYTE_ORDER_LE => true,
            b if *b == BYTE_ORDER_BE => false,
            _ => return None,
        };
        (read_u16(tiff, 2, le)? == TIFF_MAGIC).then_some(le)
    }

    /// Reads an entry's `ASCII` value, inline or pointed at.
    ///
    /// Trailing `NUL`s are dropped and anything that is not printable ASCII makes
    /// the value nothing: this text reaches a manifest, and a field of control
    /// bytes read off a used disk is not a camera's name.
    fn text(tiff: &[u8], entry: usize, le: bool) -> Option<String> {
        if read_u16(tiff, entry + 2, le)? != TYPE_ASCII {
            return None;
        }
        let count = usize::try_from(read_u32(tiff, entry + 4, le)?).ok()?;
        if count == 0 || count > MAX_TEXT_BYTES {
            return None;
        }
        let raw = if count <= INLINE_VALUE_BYTES {
            tiff.get(entry + 8..entry + 8 + count)?
        } else {
            let at = usize::try_from(read_u32(tiff, entry + 8, le)?).ok()?;
            tiff.get(at..at.checked_add(count)?)?
        };
        let trimmed = raw.split(|byte| *byte == 0).next().unwrap_or_default();
        if trimmed.is_empty() || !trimmed.iter().all(|byte| (0x20..0x7F).contains(byte)) {
            return None;
        }
        Some(String::from_utf8_lossy(trimmed).into_owned())
    }

    /// Reads an entry's single `SHORT` or `LONG` value.
    fn number(tiff: &[u8], entry: usize, kind: u16, le: bool) -> Option<u32> {
        if read_u32(tiff, entry + 4, le)? != 1 {
            return None;
        }
        match kind {
            TYPE_SHORT => read_u16(tiff, entry + 8, le).map(u32::from),
            TYPE_LONG => read_u32(tiff, entry + 8, le),
            _ => None,
        }
    }

    fn read_u16(buf: &[u8], at: usize, le: bool) -> Option<u16> {
        let raw = buf.get(at..at.checked_add(2)?)?;
        let pair = [raw[0], raw[1]];
        Some(if le {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        })
    }

    fn read_u32(buf: &[u8], at: usize, le: bool) -> Option<u32> {
        let raw = buf.get(at..at.checked_add(4)?)?;
        let quad = [raw[0], raw[1], raw[2], raw[3]];
        Some(if le {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    }
}
