//! Byte-addressed reading over a sector-addressed medium.
//!
//! Every recovery crate is sans-IO over `impl Read + Seek`; devices are
//! sector-addressed. [`BlockReader`] is the adapter between the two, so a scan
//! of a raw device runs the same code as a scan of an image file.

use std::io::{self, Read, Seek, SeekFrom};

use argos_core::geometry::Lba;
use argos_core::source::{BlockSource, Geometry};

/// `Read + Seek` view of a [`BlockSource`], addressed in bytes.
///
/// Reads that are sector-aligned go straight into the caller's buffer; only an
/// unaligned head or a sub-sector request is staged through the internal
/// bounce buffer, so the engine's large aligned chunk reads cost no extra copy.
///
/// Like every path to a medium in Argos this is read-only: the adapter exposes
/// no `Write`, and the port beneath it has no write method to expose.
#[derive(Debug)]
pub struct BlockReader<S> {
    source: S,
    geometry: Geometry,
    /// Total addressable bytes: whole sectors only.
    len: u64,
    /// Absolute byte position of the next read.
    pos: u64,
    /// One-sector staging buffer for unaligned reads; reused across calls
    /// (`M-MEM-REUSE`).
    bounce: Vec<u8>,
}

impl<S: BlockSource> BlockReader<S> {
    /// Wraps `source`, positioned at byte zero.
    ///
    /// # Panics
    ///
    /// Panics if the medium's sector count times its sector size overflows
    /// `u64` — a geometry that cannot describe a real device.
    #[must_use]
    pub fn new(source: S) -> Self {
        let geometry = source.geometry();
        let len = geometry.capacity_bytes().unwrap_or_else(|| {
            panic!(
                "medium geometry overflows: {} sectors of {}",
                geometry.sector_count, geometry.sector_size
            )
        });
        Self {
            source,
            geometry,
            len,
            pos: 0,
            bounce: Vec::new(),
        }
    }

    /// Addressable length of the medium in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the medium addresses no bytes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The medium's geometry.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Returns the wrapped source.
    #[must_use]
    pub fn into_inner(self) -> S {
        self.source
    }
}

impl<S: BlockSource> Read for BlockReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.len.saturating_sub(self.pos);
        if remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let sector_bytes = self.geometry.sector_size.get() as usize;
        let want = usize::try_from(remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        let within = usize::try_from(self.pos % self.geometry.sector_size.as_u64())
            .unwrap_or_else(|_| unreachable!("a remainder of a u32 sector size fits usize"));
        let lba = Lba::new(self.pos / self.geometry.sector_size.as_u64());

        // Aligned and at least one whole sector: read straight into `buf`.
        if within == 0 && want >= sector_bytes {
            let take = (want / sector_bytes) * sector_bytes;
            self.source
                .read_at(lba, &mut buf[..take])
                .map_err(io::Error::other)?;
            self.pos += take as u64;
            return Ok(take);
        }

        // Unaligned head or a sub-sector request: stage one sector and copy
        // out the requested slice. The caller's `read_exact` loops for more.
        self.bounce.clear();
        self.bounce.resize(sector_bytes, 0);
        self.source
            .read_at(lba, &mut self.bounce)
            .map_err(io::Error::other)?;
        let take = want.min(sector_bytes - within);
        buf[..take].copy_from_slice(&self.bounce[within..within + take]);
        self.pos += take as u64;
        Ok(take)
    }
}

impl<S: BlockSource> Seek for BlockReader<S> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        // Seeking past the end is legal and reads zero bytes there; seeking to
        // a negative position is not.
        let Some(target) = target else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the medium",
            ));
        };
        self.pos = target;
        Ok(target)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.pos)
    }
}
