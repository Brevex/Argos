//! Bounded, buffered byte cursor over `impl Read + Seek` for validators.
//!
//! Validators consume the source strictly forward through this cursor; every
//! read is bounded by an absolute end limit fixed at construction, so a corrupt
//! length field can never drive a validator past its candidate budget.

use std::io::{self, Read, Seek, SeekFrom};

/// Bytes the cursor buffers per refill. 64 KiB covers the largest JPEG marker
/// segment (u16 length) in at most two refills while staying cache-friendly.
const REFILL_BYTES: usize = 64 * 1024;

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
        }
    }

    /// Absolute offset of the next unread byte.
    pub(crate) fn pos(&self) -> u64 {
        self.buf_start + self.idx as u64
    }

    /// Next byte, or `None` at the end limit or source EOF.
    pub(crate) fn next(&mut self) -> io::Result<Option<u8>> {
        if self.idx == self.buf_len && !self.refill()? {
            return Ok(None);
        }
        let byte = self.buf[self.idx];
        self.idx += 1;
        Ok(Some(byte))
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
        let want =
            usize::try_from((self.end - pos).min(REFILL_BYTES as u64)).unwrap_or(REFILL_BYTES);
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
