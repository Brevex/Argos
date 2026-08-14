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

use argos_core::geometry::ByteOffset;

use crate::exif;
use crate::stream::Bytes;
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
