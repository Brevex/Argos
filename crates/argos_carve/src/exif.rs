//! Minimal TIFF/IFD walker for EXIF thumbnails (EXIF 2.32 §4.6).
//!
//! Reads exactly enough of the TIFF structure inside a JPEG `APP1` payload to
//! locate the IFD1 thumbnail: header, IFD0 entry table, the next-IFD pointer,
//! and IFD1's `JPEGInterchangeFormat`/`JPEGInterchangeFormatLength` tags.
//! All offsets come from the medium and are validated against the payload
//! before use; the walk visits at most two IFDs, so a crafted chain cannot
//! loop.

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

/// Bytes per IFD entry: tag, type, count, value/offset. Source: TIFF 6.0 §2.
const IFD_ENTRY_BYTES: usize = 12;

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
