//! Checked field reads over untrusted byte buffers, plus bounded medium reads.
//!
//! Every accessor returns `Option`: an out-of-range field is a failed object
//! parse, never a panic (A-UNTRUSTED-ONDISK, A-PARSER-SAFE).

use std::io::{Read, Seek, SeekFrom};

use argos_core::geometry::ByteOffset;

use crate::FsError;

/// Largest single structure any parser in this crate reads in one call.
///
/// Every on-disk structure Argos interprets is far smaller (the biggest are a
/// GPT entry array at 128 x 128 bytes and an ext4/APFS block at 64 KiB); this
/// bound is what stops a corrupt size field from turning one read into a
/// multi-terabyte allocation (A-BOUNDED-ALLOC).
pub(crate) const MAX_READ_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn u16_le(buf: &[u8], at: usize) -> Option<u16> {
    let raw = buf.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

pub(crate) fn u32_le(buf: &[u8], at: usize) -> Option<u32> {
    let raw = buf.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

pub(crate) fn u64_le(buf: &[u8], at: usize) -> Option<u64> {
    let raw = buf.get(at..at.checked_add(8)?)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(raw);
    Some(u64::from_le_bytes(bytes))
}

pub(crate) fn u32_be(buf: &[u8], at: usize) -> Option<u32> {
    let raw = buf.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// Reads exactly `len` bytes at absolute `offset` into the reused `buf`.
///
/// `Ok(false)` when the medium ends first or when `len` exceeds
/// [`MAX_READ_BYTES`] — a request that large is a corrupt size field, so it
/// fails the object's parse rather than allocating. `Err` only on I/O faults.
pub(crate) fn read_at<R: Read + Seek>(
    src: &mut R,
    offset: u64,
    len: usize,
    buf: &mut Vec<u8>,
) -> Result<bool, FsError> {
    if len > MAX_READ_BYTES {
        buf.clear();
        return Ok(false);
    }
    buf.resize(len, 0);
    src.seek(SeekFrom::Start(offset))
        .map_err(|source| FsError::io(ByteOffset::new(offset), source))?;
    let mut filled = 0;
    while filled < len {
        let n = src
            .read(&mut buf[filled..])
            .map_err(|source| FsError::io(ByteOffset::new(offset), source))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(filled == len)
}

/// UTF-16LE decode with a documented cap, for names read off the medium.
/// Invalid units decode lossily; the cap bounds allocation independently of
/// any on-disk length (A-BOUNDED-ALLOC).
pub(crate) fn utf16le_name(raw: &[u8], max_chars: usize) -> Option<String> {
    let units: Vec<u16> = raw
        .chunks_exact(2)
        .take(max_chars)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}
