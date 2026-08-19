//! Bounded, buffered byte cursor over `impl Read + Seek` for validators.
//!
//! Validators consume the source strictly forward through this cursor; every
//! read is bounded by an absolute end limit fixed at construction, so a corrupt
//! length field can never drive a validator past its candidate budget.

use std::io::{self, Read, Seek, SeekFrom};

/// Bytes the cursor buffers per refill once it has grown. 64 KiB covers the
/// largest JPEG marker segment (u16 length) in at most two refills while
/// staying cache-friendly.
const REFILL_BYTES: usize = 64 * 1024;

/// Bytes the first refill reads.
///
/// A reassembly hypothesis reads a few bytes of a proposed fragment and stops,
/// because unrelated data fails on the first Huffman code outside the table.
/// Reading 64 KiB to look at 30 of them is what the search would spend its
/// time on, so the cursor starts small and doubles: a validator walking a real
/// image reaches the full size within a few refills, while a rejected
/// hypothesis never pays for it.
const FIRST_REFILL_BYTES: usize = 1024;

/// Forward-only bounded cursor; positions are absolute source offsets.
pub(crate) struct Bytes<'a, R> {
    src: &'a mut R,
    /// Absolute offset of `buf[0]`.
    buf_start: u64,
    /// Valid length of `buf`.
    buf_len: usize,
    /// Next unread index into `buf`.
    idx: usize,
    /// Absolute end limit; no byte at or past it is ever returned.
    end: u64,
    /// Reused refill buffer, owned by the caller's `Scratch`.
    buf: &'a mut Vec<u8>,
    /// Bytes the next refill reads, doubling up to [`REFILL_BYTES`].
    step: usize,
}

impl<'a, R: Read + Seek> Bytes<'a, R> {
    /// A cursor over `src` from `start` (absolute) up to `end` (exclusive).
    pub(crate) fn new(src: &'a mut R, start: u64, end: u64, buf: &'a mut Vec<u8>) -> Self {
        buf.clear();
        Self {
            src,
            buf_start: start,
            buf_len: 0,
            idx: 0,
            end,
            buf,
            step: FIRST_REFILL_BYTES,
        }
    }

    /// Absolute offset of the next unread byte.
    pub(crate) fn pos(&self) -> u64 {
        self.buf_start + self.idx as u64
    }

    /// Next byte, or `None` at the end limit or source EOF.
    #[inline]
    pub(crate) fn next(&mut self) -> io::Result<Option<u8>> {
        if self.idx == self.buf_len && !self.refill()? {
            return Ok(None);
        }
        let byte = self.buf[self.idx];
        self.idx += 1;
        Ok(Some(byte))
    }

    /// `n` buffered bytes containing no `0xFF`, packed big-endian and consumed.
    ///
    /// `None` when fewer than `n` are buffered or any of them is `0xFF`, and it
    /// never refills. Both are what make it safe for a caller reading
    /// entropy-coded data: `0xFF` introduces stuffing, fill bytes or a marker,
    /// and resolving those is the byte-at-a-time path's business. A window this
    /// returns is one physical byte per logical byte, which is what lets
    /// [`Bytes::rewind`] give it back.
    #[inline]
    pub(crate) fn take_clean(&mut self, n: usize) -> Option<u64> {
        debug_assert!(n <= 8, "a window wider than a u64 cannot be packed: {n}");
        let end = self.idx.checked_add(n)?;
        if end > self.buf_len {
            return None;
        }
        let window = self.buf.get(self.idx..end)?;
        let mut packed = 0_u64;
        for byte in window {
            if *byte == 0xFF {
                return None;
            }
            packed = (packed << 8) | u64::from(*byte);
        }
        self.idx = end;
        Some(packed)
    }

    /// Gives back `n` bytes taken through [`Bytes::take_clean`].
    ///
    /// Only ever called with a count taken from the same buffer and not yet
    /// crossed by a refill, so the bytes are still there to give back.
    #[inline]
    pub(crate) fn rewind(&mut self, n: usize) {
        self.idx = self.idx.saturating_sub(n);
    }

    /// Appends exactly `n` bytes to `out`; `false` when the limit or EOF cuts
    /// the read short (partial bytes are not appended).
    pub(crate) fn read_into(&mut self, out: &mut Vec<u8>, n: usize) -> io::Result<bool> {
        let mark = out.len();
        out.reserve(n);
        let mut remaining = n;
        while remaining > 0 {
            if self.idx == self.buf_len && !self.refill()? {
                out.truncate(mark);
                return Ok(false);
            }
            let take = remaining.min(self.buf_len - self.idx);
            out.extend_from_slice(&self.buf[self.idx..self.idx + take]);
            self.idx += take;
            remaining -= take;
        }
        Ok(true)
    }

    /// Skips `n` bytes; `false` when the limit is hit first.
    pub(crate) fn skip(&mut self, n: u64) -> bool {
        let buffered = (self.buf_len - self.idx) as u64;
        if n <= buffered {
            self.idx += usize::try_from(n).unwrap_or_else(|_| {
                unreachable!("n <= buffered bytes, which fits usize");
            });
            return true;
        }
        let target = self.pos().saturating_add(n);
        if target > self.end {
            // Position past the limit: consume what is buffered and report short.
            self.idx = self.buf_len;
            self.buf_start = target.min(self.end);
            self.buf_len = 0;
            self.idx = 0;
            return false;
        }
        self.buf_start = target;
        self.buf_len = 0;
        self.idx = 0;
        true
    }

    /// Refills the buffer at the current position; `false` at limit or EOF.
    fn refill(&mut self) -> io::Result<bool> {
        let pos = self.pos();
        if pos >= self.end {
            return Ok(false);
        }
        let want = usize::try_from((self.end - pos).min(self.step as u64)).unwrap_or(self.step);
        self.step = self.step.saturating_mul(2).min(REFILL_BYTES);
        self.buf.resize(want, 0);
        self.src.seek(SeekFrom::Start(pos))?;
        let mut filled = 0;
        while filled < want {
            let n = self.src.read(&mut self.buf[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        self.buf.truncate(filled);
        self.buf_start = pos;
        self.buf_len = filled;
        self.idx = 0;
        Ok(filled > 0)
    }
}
