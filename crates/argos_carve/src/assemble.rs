//! Reading a file that is scattered across a medium as if it were whole.
//!
//! Reassembly is a search: propose an extent list, ask the real decoder whether
//! it is the file, keep what validates. [`Assembled`] is what makes that
//! possible — it presents any list of source extents as one contiguous
//! `Read + Seek` stream, so a format's state machine validates a hypothesis
//! without knowing the bytes were ever apart.
//!
//! Offsets addressed through this reader are **file offsets**, not medium
//! offsets: position zero is the file's first byte, wherever it lives.

use std::io::{self, Read, Seek, SeekFrom};

use argos_core::geometry::ByteRange;

/// A list of source extents presented as one contiguous byte stream.
#[derive(Debug)]
pub struct Assembled<'a, R> {
    src: &'a mut R,
    extents: &'a [ByteRange],
    /// Extent currently being read.
    index: usize,
    /// Bytes already produced from the current extent.
    consumed: u64,
    /// Whether the source is positioned for the current extent.
    positioned: bool,
}

impl<'a, R: Read + Seek> Assembled<'a, R> {
    /// Presents `extents` of `src`, in the given order, as one stream.
    ///
    /// The order is the file's order, which is not necessarily the medium's:
    /// a fragmented file's later bytes routinely sit at a lower offset than
    /// its earlier ones.
    pub fn new(src: &'a mut R, extents: &'a [ByteRange]) -> Self {
        Self {
            src,
            extents,
            index: 0,
            consumed: 0,
            positioned: false,
        }
    }

    /// Total length of the assembled stream.
    #[must_use]
    pub fn len(&self) -> u64 {
        total_len(self.extents)
    }

    /// Whether the assembled stream holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Position in the assembled stream: bytes produced so far.
    fn position(&self) -> u64 {
        self.extents
            .iter()
            .take(self.index)
            .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
            .saturating_add(self.consumed)
    }
}

/// Total length of an extent list, saturating rather than wrapping on a length
/// that could only come from a corrupt medium.
#[must_use]
pub fn total_len(extents: &[ByteRange]) -> u64 {
    extents
        .iter()
        .fold(0_u64, |sum, extent| sum.saturating_add(extent.len))
}

impl<R: Read + Seek> Read for Assembled<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let Some(extent) = self.extents.get(self.index) else {
                return Ok(0);
            };
            let remaining = extent.len.saturating_sub(self.consumed);
            if remaining == 0 {
                self.index += 1;
                self.consumed = 0;
                self.positioned = false;
                continue;
            }
            if !self.positioned {
                let at = extent
                    .start
                    .checked_add(self.consumed)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "extent position overflows the medium's address space",
                        )
                    })?
                    .get();
                self.src.seek(SeekFrom::Start(at))?;
                self.positioned = true;
            }
            let want = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(buf.len());
            if want == 0 {
                return Ok(0);
            }
            let read = self.src.read(&mut buf[..want])?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the medium ends inside an extent this file claims",
                ));
            }
            self.consumed = self.consumed.saturating_add(read as u64);
            return Ok(read);
        }
    }
}

impl<R: Read + Seek> Seek for Assembled<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len().checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position().checked_add_signed(delta),
        };
        let Some(target) = target else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the assembled file",
            ));
        };

        // Walk to the extent holding `target`. Extent lists are bounded by the
        // caller (a fragment budget, or a filesystem's own run count), so this
        // stays short.
        let mut remaining = target;
        self.index = self.extents.len();
        self.consumed = 0;
        self.positioned = false;
        for (index, extent) in self.extents.iter().enumerate() {
            if remaining < extent.len {
                self.index = index;
                self.consumed = remaining;
                break;
            }
            remaining = remaining.saturating_sub(extent.len);
        }
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position())
    }
}
