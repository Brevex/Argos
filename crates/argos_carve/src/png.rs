//! PNG validation state machine (ISO 15948 / RFC 2083).
//!
//! Walks a candidate as signature → `IHDR` → chunk sequence → `IEND`. Every
//! chunk length is bounds-checked, every chunk CRC32 is verified, `IHDR`
//! dimensions and bit-depth/colour combinations are checked against the
//! specification, and the concatenated `IDAT` zlib stream is inflated
//! incrementally — output is discarded, capped, and never buffered whole, so
//! a crafted stream cannot balloon memory. The first violation becomes a
//! [`Verdict::Corrupt`] carrying the exact fragmentation point.

use std::io::{Read, Seek};

use argos_core::ByteOffset;
use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

use crate::Bytes;
use crate::{CarveError, Scratch, Verdict};

/// Largest chunk data length the spec allows (2^31 − 1). Source: ISO 15948
/// §5.3: "length … must not exceed 2^31 − 1 bytes".
const MAX_CHUNK_BYTES: u32 = 0x7FFF_FFFF;

/// Sanity cap on either image dimension. Far beyond any camera or scanner
/// output (gigapixel panoramas stay under ~200 000 px on a side); a corrupt
/// `IHDR` past this is rejected instead of driving huge inflate budgets.
const MAX_DIMENSION: u32 = 1_000_000;

/// Cap on total decompressed `IDAT` output. Bounds validation CPU/time on a
/// crafted high-ratio stream; 1 GiB exceeds the raw size of any plausible
/// photograph this tool targets.
const MAX_DECOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;

/// Bytes of chunk data processed per step; matches the cursor refill size so
/// each step is one buffered read.
const PIECE_BYTES: u32 = 64 * 1024;

/// Validates the PNG candidate starting at `start`.
///
/// Consumes at most `limit - start` bytes of `src`; the caller caps `limit`
/// at the medium end and [`crate::MAX_IMAGE_BYTES`].
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
    let Scratch {
        stream,
        seg,
        inflate_out,
    } = scratch;
    let mut bytes = Bytes::new(src, start.get(), limit, stream);

    let corrupt = |at: u64| Verdict::Corrupt {
        at: ByteOffset::new(at),
        thumbnail: None,
    };

    // Signature.
    seg.clear();
    let ok = bytes
        .read_into(seg, crate::PNG_SIGNATURE.len())
        .map_err(|source| CarveError::io(start, source))?;
    if !ok || seg[..] != crate::PNG_SIGNATURE {
        return Ok(corrupt(start.get()));
    }

    // IHDR must be first.
    let Some(header) = read_chunk_header(&mut bytes)? else {
        return Ok(corrupt(bytes.pos()));
    };
    if &header.kind != b"IHDR" || header.length != 13 {
        return Ok(corrupt(bytes.pos()));
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(&header.kind);
    seg.clear();
    let ok = bytes
        .read_into(seg, 13)
        .map_err(|source| CarveError::io(ByteOffset::new(bytes.pos()), source))?;
    if !ok {
        return Ok(corrupt(bytes.pos()));
    }
    crc.update(seg);
    let Some(ihdr) = Ihdr::parse(seg) else {
        return Ok(corrupt(bytes.pos()));
    };
    if !check_crc(&mut bytes, crc)? {
        return Ok(corrupt(bytes.pos()));
    }

    // Decompressed cap: twice the non-interlaced raw size covers the Adam7
    // per-pass filter-byte overhead with margin.
    let Some(inflate_cap) = ihdr
        .raw_bytes()
        .and_then(|raw| raw.checked_mul(2))
        .map(|cap| cap.min(MAX_DECOMPRESSED_BYTES))
    else {
        return Ok(corrupt(bytes.pos()));
    };

    walk_chunks(&mut bytes, seg, inflate_out, inflate_cap, start)
}

/// Walks the chunk sequence after `IHDR` up to a validated `IEND`.
fn walk_chunks<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    seg: &mut Vec<u8>,
    inflate_out: &mut Vec<u8>,
    inflate_cap: u64,
    start: ByteOffset,
) -> Result<Verdict, CarveError> {
    let corrupt = |at: u64| Verdict::Corrupt {
        at: ByteOffset::new(at),
        thumbnail: None,
    };
    let mut inflater = InflateState::new_boxed(DataFormat::Zlib);
    let piece = usize::try_from(PIECE_BYTES).unwrap_or_else(|_| {
        panic!("PIECE_BYTES ({PIECE_BYTES}) must fit usize on supported targets")
    });
    inflate_out.resize(piece, 0);
    let mut total_out = 0_u64;
    let mut stream_ended = false;
    let mut idat_seen = false;
    let mut idat_closed = false;

    loop {
        let Some(chunk) = read_chunk_header(bytes)? else {
            return Ok(corrupt(bytes.pos()));
        };
        if chunk.length > MAX_CHUNK_BYTES || !chunk.kind.iter().all(u8::is_ascii_alphabetic) {
            return Ok(corrupt(bytes.pos()));
        }
        let is_idat = &chunk.kind == b"IDAT";
        // IDAT chunks must be consecutive. Source: ISO 15948 §5.6.
        if is_idat && idat_closed {
            return Ok(corrupt(bytes.pos()));
        }
        if !is_idat && idat_seen {
            idat_closed = true;
        }
        idat_seen |= is_idat;

        let mut crc = crc32fast::Hasher::new();
        crc.update(&chunk.kind);
        let data_start = bytes.pos();
        let mut remaining = chunk.length;
        while remaining > 0 {
            let take = remaining.min(PIECE_BYTES);
            seg.clear();
            let at = bytes.pos();
            let take_len = usize::try_from(take).unwrap_or_else(|_| {
                panic!("piece of {take} bytes must fit usize: it is capped at PIECE_BYTES")
            });
            let ok = bytes
                .read_into(seg, take_len)
                .map_err(|source| CarveError::io(ByteOffset::new(at), source))?;
            if !ok {
                return Ok(corrupt(bytes.pos()));
            }
            crc.update(seg);
            if is_idat
                && let Err(consumed) = feed_inflater(
                    &mut inflater,
                    seg,
                    inflate_out,
                    &mut total_out,
                    &mut stream_ended,
                    inflate_cap,
                )
            {
                return Ok(corrupt(at + consumed as u64));
            }
            remaining -= take;
        }
        if !check_crc(bytes, crc)? {
            // A CRC mismatch cannot localize the damage inside the chunk; the
            // chunk data start is the earliest offset corruption may begin.
            return Ok(corrupt(data_start));
        }

        if &chunk.kind == b"IEND" {
            if chunk.length != 0 || !idat_seen || !stream_ended {
                return Ok(corrupt(bytes.pos()));
            }
            return Ok(Verdict::Complete {
                length: bytes.pos() - start.get(),
                thumbnail: None,
            });
        }
    }
}

/// A chunk's length and type, as read from the stream.
struct ChunkHeader {
    length: u32,
    kind: [u8; 4],
}

fn read_chunk_header<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
) -> Result<Option<ChunkHeader>, CarveError> {
    let Some(length) = read_u32(bytes)? else {
        return Ok(None);
    };
    let mut kind = [0_u8; 4];
    for slot in &mut kind {
        let at = bytes.pos();
        match bytes
            .next()
            .map_err(|source| CarveError::io(ByteOffset::new(at), source))?
        {
            Some(byte) => *slot = byte,
            None => return Ok(None),
        }
    }
    Ok(Some(ChunkHeader { length, kind }))
}

/// Feeds one piece of `IDAT` data through the inflater, discarding output.
/// On failure returns the offset *within this piece* where inflation stopped,
/// so the caller can report the exact fragmentation point.
fn feed_inflater(
    inflater: &mut InflateState,
    mut input: &[u8],
    out: &mut [u8],
    total_out: &mut u64,
    stream_ended: &mut bool,
    cap: u64,
) -> Result<(), usize> {
    let mut consumed_total = 0_usize;
    while !input.is_empty() {
        if *stream_ended {
            // Trailing garbage after the zlib stream inside IDAT.
            return Err(consumed_total);
        }
        let result = inflate(inflater, input, out, MZFlush::None);
        *total_out += result.bytes_written as u64;
        if *total_out > cap {
            return Err(consumed_total);
        }
        match result.status {
            Ok(MZStatus::StreamEnd) => *stream_ended = true,
            Ok(_) => {
                if result.bytes_consumed == 0 && result.bytes_written == 0 {
                    // No progress: corrupt stream state.
                    return Err(consumed_total);
                }
            }
            Err(_) => return Err(consumed_total),
        }
        consumed_total += result.bytes_consumed;
        input = &input[result.bytes_consumed..];
    }
    Ok(())
}

/// Pixel dimensions the candidate's `IHDR` declares, without validating the
/// rest of it.
///
/// A frame states its size before its data, so what a candidate claims to be
/// is known for the cost of thirty-three bytes. That is what lets a search
/// spend its budget on photograph-sized frames: a used disk holds two orders
/// of magnitude more cache entries and icons than pictures, and no reassembly
/// of a 48x48 icon could produce anything but the icon it already is.
///
/// `None` when the signature or the `IHDR` does not verify — including its
/// CRC, so a coincidence does not get to state a size.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub(crate) fn header_dimensions<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    scratch: &mut Scratch,
) -> Result<Option<(u32, u32)>, CarveError> {
    let Scratch { stream, seg, .. } = scratch;
    let mut bytes = Bytes::new(src, start.get(), limit, stream);

    seg.clear();
    let ok = bytes
        .read_into(seg, crate::PNG_SIGNATURE.len())
        .map_err(|source| CarveError::io(start, source))?;
    if !ok || seg[..] != crate::PNG_SIGNATURE {
        return Ok(None);
    }

    let Some(header) = read_chunk_header(&mut bytes)? else {
        return Ok(None);
    };
    if &header.kind != b"IHDR" || header.length != 13 {
        return Ok(None);
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(&header.kind);
    seg.clear();
    let ok = bytes
        .read_into(seg, 13)
        .map_err(|source| CarveError::io(ByteOffset::new(bytes.pos()), source))?;
    if !ok {
        return Ok(None);
    }
    crc.update(seg);
    let Some(ihdr) = Ihdr::parse(seg) else {
        return Ok(None);
    };
    if !check_crc(&mut bytes, crc)? {
        return Ok(None);
    }
    Ok(Some((ihdr.width, ihdr.height)))
}

/// Reads the stored chunk CRC and compares it to the computed one.
fn check_crc<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    crc: crc32fast::Hasher,
) -> Result<bool, CarveError> {
    let Some(stored) = read_u32(bytes)? else {
        return Ok(false);
    };
    Ok(stored == crc.finalize())
}

fn read_u32<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u32>, CarveError> {
    let mut raw = [0_u8; 4];
    for slot in &mut raw {
        let at = bytes.pos();
        match bytes
            .next()
            .map_err(|source| CarveError::io(ByteOffset::new(at), source))?
        {
            Some(byte) => *slot = byte,
            None => return Ok(None),
        }
    }
    Ok(Some(u32::from_be_bytes(raw)))
}

/// The `IHDR` fields the validator checks. Source: ISO 15948 §11.2.2.
struct Ihdr {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
}

impl Ihdr {
    fn parse(payload: &[u8]) -> Option<Self> {
        let width = u32::from_be_bytes(payload.get(0..4)?.try_into().ok()?);
        let height = u32::from_be_bytes(payload.get(4..8)?.try_into().ok()?);
        let bit_depth = *payload.get(8)?;
        let color_type = *payload.get(9)?;
        let compression = *payload.get(10)?;
        let filter = *payload.get(11)?;
        let interlace = *payload.get(12)?;

        if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
            return None;
        }
        // Legal bit-depth/colour-type combinations. Source: ISO 15948 §11.2.2.
        let depth_ok = match color_type {
            0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
            2 | 4 | 6 => matches!(bit_depth, 8 | 16),
            3 => matches!(bit_depth, 1 | 2 | 4 | 8),
            _ => false,
        };
        if !depth_ok || compression != 0 || filter != 0 || interlace > 1 {
            return None;
        }
        Some(Self {
            width,
            height,
            bit_depth,
            color_type,
        })
    }

    /// Samples per pixel for the colour type. Source: ISO 15948 §6.1.
    fn channels(&self) -> u64 {
        match self.color_type {
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 1,
        }
    }

    /// Raw (decompressed, non-interlaced) image size: per row, the pixel bits
    /// rounded up to bytes plus one filter byte.
    fn raw_bytes(&self) -> Option<u64> {
        let row_bits = u64::from(self.width)
            .checked_mul(self.channels())?
            .checked_mul(u64::from(self.bit_depth))?;
        let row_bytes = row_bits.div_ceil(8).checked_add(1)?;
        u64::from(self.height).checked_mul(row_bytes)
    }
}
