//! JPEG entropy-coded scan decoding, MCU by MCU — the reassembly oracle.
//!
//! Reassembly is a search that tests thousands of hypotheses, so the thing
//! that judges a hypothesis decides whether the results are evidence. Walking
//! the marker grammar is not enough: roughly a quarter of the byte values
//! following an `0xFF` make the parser skip a length-prefixed segment whose
//! length it read from the unknown bytes themselves, jump up to 65533 bytes,
//! and carry on. Random data therefore produces "valid structure" routinely,
//! and the position the parser reaches says more about where the candidate sat
//! on the disk than about whether it is a file.
//!
//! Decoding the entropy-coded data answers the question exactly instead:
//!
//! - **Complete** means every MCU the frame header requires was decoded and
//!   the stream then reached `EOI`. Nothing else counts.
//! - **Progress** is MCUs decoded. Garbage fails on the first Huffman code
//!   that is not in the table, which happens within a few bytes, so progress
//!   cannot be inflated by luck the way a stream position can.
//! - **Position** is the exact byte the decoder stopped on, so a fragment can
//!   be trimmed to what was really consumed.
//! - **Geometry** maps an MCU index to a pixel row, which is what lets
//!   reassembly measure smoothness across the stitch rather than over the
//!   whole image.
//!
//! No inverse DCT and no colour conversion happen here: coefficients are
//! decoded and discarded. The question is whether the bits *are* a scan, not
//! what it looks like.
//!
//! Scope is baseline and extended sequential Huffman (`SOF0`, `SOF1`).
//! Progressive and arithmetic-coded frames are reported as
//! [`ScanStop::Unsupported`] and are not reassembled — claiming a recovery
//! from a coding this decoder cannot check would be a guess.

use std::io::{Read, Seek};

use argos_core::geometry::ByteOffset;

use crate::stream::Bytes;
use crate::{CarveError, Scratch};

/// Components a frame may declare.
///
/// T.81 allows 255; JFIF uses one (greyscale) or three (YCbCr), and four
/// covers CMYK/YCCK. A frame claiming more is not one this decoder handles.
const MAX_COMPONENTS: usize = 4;

/// Huffman table slots per class. Source: T.81 §B.2.4.2 — four DC, four AC.
const MAX_TABLES: usize = 4;

/// Coefficients in one 8x8 block. Source: T.81 §A.3.
const BLOCK_COEFFICIENTS: usize = 64;

/// Samples along one edge of a block. Source: T.81 §A.3.
const BLOCK_SAMPLES: u32 = 8;

/// Longest Huffman code. Source: T.81 §B.2.4.2 — code lengths run 1..=16.
const MAX_CODE_BITS: usize = 16;

/// Largest sampling factor. Source: T.81 §B.2.2 — `H` and `V` are 1..=4.
const MAX_SAMPLING: u32 = 4;

/// Largest MCU count a frame may require.
///
/// A 65535x65535 frame of one-sample MCUs is about 67 million; the bound sits
/// above that and stops a crafted frame header from driving an unbounded loop
/// (A-BOUNDED-ALLOC).
const MAX_MCUS: u32 = 1 << 27;

/// Marker codes this decoder recognises. Source: T.81 Table B.1.
const MARKER_SOI: u8 = 0xD8;
const MARKER_EOI: u8 = 0xD9;
const MARKER_SOS: u8 = 0xDA;
const MARKER_DQT: u8 = 0xDB;
const MARKER_DNL: u8 = 0xDC;
const MARKER_DRI: u8 = 0xDD;
const MARKER_DHT: u8 = 0xC4;
const MARKER_RST0: u8 = 0xD0;
const MARKER_RST7: u8 = 0xD7;

/// Why an entropy scan stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScanStop {
    /// Every MCU the frame requires decoded, and the stream reached `EOI`.
    /// This is the only outcome that means "these bytes are this image".
    Complete,
    /// The data stopped decoding as this frame's scan.
    Broke,
    /// The frame uses a coding this decoder does not implement — progressive
    /// or arithmetic. Nothing is claimed about it either way.
    Unsupported,
}

/// What decoding a candidate's entropy-coded scan established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOutcome {
    /// MCUs decoded before stopping.
    pub mcus_decoded: u32,
    /// MCUs the frame header requires. Zero when no frame was parsed.
    pub mcus_required: u32,
    /// MCUs per row, for mapping an MCU index to a pixel row.
    pub mcus_across: u32,
    /// Pixel rows one MCU spans.
    pub mcu_rows: u32,
    /// MCU being decoded when the stream crossed the watched offset — the
    /// splice, when a caller is checking one. Zero when nothing was watched.
    pub seam_mcu: u32,
    /// First byte past the last one the decoder consumed.
    pub end: ByteOffset,
    /// Why it stopped.
    pub stop: ScanStop,
}

impl ScanOutcome {
    /// Whether the bytes decoded end to end as the frame they claim to be.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.stop == ScanStop::Complete
            && self.mcus_required > 0
            && self.mcus_decoded == self.mcus_required
    }

    /// Pixel row where the MCU at `index` starts.
    ///
    /// This is what turns a fragment boundary into a place to look in the
    /// decoded image: the splice happened at the MCU the decoder was on, and
    /// its seam shows in this row.
    #[must_use]
    pub fn pixel_row_of(&self, mcu_index: u32) -> u32 {
        if self.mcus_across == 0 {
            return 0;
        }
        (mcu_index / self.mcus_across).saturating_mul(self.mcu_rows)
    }

    /// An outcome describing a candidate nothing could be established about.
    fn nothing(end: u64, stop: ScanStop) -> Self {
        Self {
            mcus_decoded: 0,
            mcus_required: 0,
            mcus_across: 0,
            mcu_rows: 0,
            seam_mcu: 0,
            end: ByteOffset::new(end),
            stop,
        }
    }
}

/// Decodes the entropy-coded scan of the JPEG candidate at `start`.
///
/// Reads at most `limit - start` bytes. Corruption is never an error: it is a
/// [`ScanStop::Broke`] outcome carrying how far the decoder got.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn scan<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    scratch: &mut Scratch,
) -> Result<ScanOutcome, CarveError> {
    scan_watching(src, start, limit, None, scratch)
}

/// Decodes as [`scan`] does, additionally reporting which MCU was being
/// decoded when the stream crossed `watch`.
///
/// `watch` is an offset relative to `start`, and it is how a caller finds the
/// **stitch row**: a reassembly's fragment boundary is a byte offset, and what
/// tells you whether the splice is real is the picture at the pixel row that
/// byte produced.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn scan_watching<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    watch: Option<u64>,
    scratch: &mut Scratch,
) -> Result<ScanOutcome, CarveError> {
    let Scratch { stream, seg, .. } = scratch;
    let mut bytes = Bytes::new(src, start.get(), limit, stream);
    let mut tables = Tables::default();
    let mut frame: Option<Frame> = None;
    let mut restart_interval = 0_u16;

    if next(&mut bytes)? != Some(0xFF) || next(&mut bytes)? != Some(MARKER_SOI) {
        return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
    }

    loop {
        let Some(code) = next_marker(&mut bytes)? else {
            return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
        };
        match code {
            MARKER_DHT => {
                let Some(()) = read_payload(&mut bytes, seg)? else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                if tables.absorb(seg).is_none() {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                }
            }
            MARKER_DRI => {
                let Some(()) = read_payload(&mut bytes, seg)? else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                let (Some(&hi), Some(&lo)) = (seg.first(), seg.get(1)) else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                restart_interval = u16::from_be_bytes([hi, lo]);
            }
            // Baseline and extended sequential, Huffman coded.
            0xC0 | 0xC1 => {
                let Some(()) = read_payload(&mut bytes, seg)? else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                let Some(parsed) = Frame::parse(seg) else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                frame = Some(parsed);
            }
            // Progressive, lossless, hierarchical and every arithmetic-coded
            // frame: outside what this decoder can check.
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Unsupported));
            }
            MARKER_SOS => {
                let Some(frame) = frame else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                let Some(()) = read_payload(&mut bytes, seg)? else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                let Some(scan) = ScanHeader::parse(seg, &frame) else {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                };
                return decode_scan(
                    &mut bytes,
                    &frame,
                    &scan,
                    &tables,
                    restart_interval,
                    watch.map(|at| start.get().saturating_add(at)),
                );
            }
            MARKER_EOI => return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke)),
            // Quantisation tables, comments, application data and the rest:
            // nothing the entropy decoder needs, but their length is trusted
            // only as far as the read limit allows.
            MARKER_DQT | MARKER_DNL | 0xC8 | 0xCC | 0xE0..=0xEF | 0xFE => {
                if !skip_payload(&mut bytes)? {
                    return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
                }
            }
            _ => return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke)),
        }
    }
}

/// The frame header's geometry and components.
#[derive(Clone, Copy, Debug)]
struct Frame {
    width: u32,
    height: u32,
    components: [Component; MAX_COMPONENTS],
    count: usize,
    hmax: u32,
    vmax: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Component {
    id: u8,
    horizontal: u32,
    vertical: u32,
}

impl Frame {
    /// Parses an `SOFn` payload: precision, dimensions, then per component an
    /// id, packed sampling factors and a quantisation table selector.
    /// Source: T.81 §B.2.2.
    fn parse(payload: &[u8]) -> Option<Self> {
        // Precision at 0 is not used here; dimensions follow it.
        let height = u32::from(u16::from_be_bytes([*payload.get(1)?, *payload.get(2)?]));
        let width = u32::from(u16::from_be_bytes([*payload.get(3)?, *payload.get(4)?]));
        let count = usize::from(*payload.get(5)?);
        if width == 0 || height == 0 || count == 0 || count > MAX_COMPONENTS {
            return None;
        }

        let mut components = [Component::default(); MAX_COMPONENTS];
        let mut hmax = 1_u32;
        let mut vmax = 1_u32;
        for (index, component) in components.iter_mut().take(count).enumerate() {
            let at = 6 + index * 3;
            let sampling = *payload.get(at + 1)?;
            let horizontal = u32::from(sampling >> 4);
            let vertical = u32::from(sampling & 0x0F);
            if horizontal == 0
                || vertical == 0
                || horizontal > MAX_SAMPLING
                || vertical > MAX_SAMPLING
            {
                return None;
            }
            *component = Component {
                id: *payload.get(at)?,
                horizontal,
                vertical,
            };
            hmax = hmax.max(horizontal);
            vmax = vmax.max(vertical);
        }

        Some(Self {
            width,
            height,
            components,
            count,
            hmax,
            vmax,
        })
    }

    fn component(&self, id: u8) -> Option<Component> {
        self.components
            .iter()
            .take(self.count)
            .copied()
            .find(|component| component.id == id)
    }
}

/// The scan header: which components, and which Huffman tables each uses.
#[derive(Clone, Copy, Debug)]
struct ScanHeader {
    entries: [ScanComponent; MAX_COMPONENTS],
    count: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScanComponent {
    component: Component,
    dc_table: usize,
    ac_table: usize,
}

impl ScanHeader {
    /// Parses an `SOS` payload. Source: T.81 §B.2.3.
    fn parse(payload: &[u8], frame: &Frame) -> Option<Self> {
        let count = usize::from(*payload.first()?);
        if count == 0 || count > MAX_COMPONENTS || count > frame.count {
            return None;
        }
        let mut entries = [ScanComponent::default(); MAX_COMPONENTS];
        for (index, entry) in entries.iter_mut().take(count).enumerate() {
            let at = 1 + index * 2;
            let selector = *payload.get(at + 1)?;
            let dc_table = usize::from(selector >> 4);
            let ac_table = usize::from(selector & 0x0F);
            if dc_table >= MAX_TABLES || ac_table >= MAX_TABLES {
                return None;
            }
            *entry = ScanComponent {
                component: frame.component(*payload.get(at)?)?,
                dc_table,
                ac_table,
            };
        }
        // Spectral selection and successive approximation follow; a baseline
        // scan must cover the whole spectrum in one pass.
        let spectral_start = *payload.get(1 + count * 2)?;
        let spectral_end = *payload.get(2 + count * 2)?;
        if spectral_start != 0 || usize::from(spectral_end) != BLOCK_COEFFICIENTS - 1 {
            return None;
        }
        Some(Self { entries, count })
    }
}

/// The Huffman tables a frame has declared so far.
#[derive(Clone, Default)]
struct Tables {
    dc: [Option<HuffTable>; MAX_TABLES],
    ac: [Option<HuffTable>; MAX_TABLES],
}

impl Tables {
    /// Absorbs a `DHT` payload, which may hold several tables back to back.
    /// Source: T.81 §B.2.4.2.
    fn absorb(&mut self, payload: &[u8]) -> Option<()> {
        let mut at = 0_usize;
        while at < payload.len() {
            let selector = *payload.get(at)?;
            let class = selector >> 4;
            let slot = usize::from(selector & 0x0F);
            if class > 1 || slot >= MAX_TABLES {
                return None;
            }
            let counts = payload.get(at + 1..at + 1 + MAX_CODE_BITS)?;
            let total: usize = counts.iter().map(|&count| usize::from(count)).sum();
            // A Huffman table cannot hold more values than there are symbols.
            if total > 256 {
                return None;
            }
            let values = payload.get(at + 1 + MAX_CODE_BITS..at + 1 + MAX_CODE_BITS + total)?;
            let table = HuffTable::build(counts, values)?;
            if class == 0 {
                self.dc[slot] = Some(table);
            } else {
                self.ac[slot] = Some(table);
            }
            at += 1 + MAX_CODE_BITS + total;
        }
        Some(())
    }
}

/// Symbols one Huffman table may hold. Source: T.81 §B.2.4.2 — a table maps
/// codes to byte values, so it can never hold more than there are of those.
const MAX_SYMBOLS: usize = 256;

/// A canonical Huffman decoding table, in the form T.81 §F.2.2.3 uses.
///
/// The symbol list is inline rather than a `Vec`: a table is rebuilt for every
/// reassembly hypothesis, and thousands of those run per fragmented candidate,
/// so a heap allocation here would be one per hypothesis (`M-MEM-REUSE`).
#[derive(Clone)]
struct HuffTable {
    /// Smallest code of each length, indexed by length.
    mincode: [i32; MAX_CODE_BITS + 1],
    /// Largest code of each length; `-1` when the length has no codes.
    maxcode: [i32; MAX_CODE_BITS + 1],
    /// Index into `values` of the first symbol of each length.
    valptr: [usize; MAX_CODE_BITS + 1],
    values: [u8; MAX_SYMBOLS],
    symbols: usize,
}

impl HuffTable {
    fn build(counts: &[u8], values: &[u8]) -> Option<Self> {
        let mut mincode = [0_i32; MAX_CODE_BITS + 1];
        let mut maxcode = [-1_i32; MAX_CODE_BITS + 1];
        let mut valptr = [0_usize; MAX_CODE_BITS + 1];

        let mut code = 0_i32;
        let mut index = 0_usize;
        for length in 1..=MAX_CODE_BITS {
            let count = usize::from(*counts.get(length - 1)?);
            if count == 0 {
                // No codes of this length; `maxcode` stays -1 so the decoder
                // never matches here.
                code <<= 1;
                continue;
            }
            valptr[length] = index;
            mincode[length] = code;
            code = code.checked_add(i32::try_from(count).ok()?)?;
            maxcode[length] = code - 1;
            index += count;
            // A canonical table's codes must stay within their bit width.
            if code > 1 << length {
                return None;
            }
            code <<= 1;
        }

        let mut symbols = [0_u8; MAX_SYMBOLS];
        let taken = values.len().min(MAX_SYMBOLS);
        symbols[..taken].copy_from_slice(&values[..taken]);
        Some(Self {
            mincode,
            maxcode,
            valptr,
            values: symbols,
            symbols: taken,
        })
    }
}

/// Where the bit reader stopped feeding.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Halt {
    /// Still inside entropy-coded data.
    None,
    /// A marker was met; the scan cannot continue through it.
    Marker(u8),
    /// The read limit or the medium ended.
    Ended,
}

/// Reads entropy-coded bits, undoing byte stuffing and stopping at markers.
///
/// Bytes are pulled one at a time, on demand, so the cursor position stays
/// within one byte of what has actually been consumed — reassembly trims
/// fragments to that position, so over-reading ahead would misreport it.
struct BitReader<'a, 'b, R> {
    bytes: &'a mut Bytes<'b, R>,
    /// Bits pulled but not yet consumed, most significant first.
    accumulator: u32,
    /// How many of `accumulator`'s low bits are valid.
    bits: u32,
    halt: Halt,
}

impl<'a, 'b, R: Read + Seek> BitReader<'a, 'b, R> {
    fn new(bytes: &'a mut Bytes<'b, R>) -> Self {
        Self {
            bytes,
            accumulator: 0,
            bits: 0,
            halt: Halt::None,
        }
    }

    /// Pulls the next byte of entropy data, resolving `FF 00` stuffing and
    /// noting the marker that ends the scan. Source: T.81 §B.1.1.5.
    fn next_data_byte(&mut self) -> Result<Option<u8>, CarveError> {
        if self.halt != Halt::None {
            return Ok(None);
        }
        let Some(byte) = next(self.bytes)? else {
            self.halt = Halt::Ended;
            return Ok(None);
        };
        if byte != 0xFF {
            return Ok(Some(byte));
        }
        // `0xFF` introduces either stuffing, fill bytes, or a marker.
        let mut code = 0xFF_u8;
        while code == 0xFF {
            let Some(next_byte) = next(self.bytes)? else {
                self.halt = Halt::Ended;
                return Ok(None);
            };
            code = next_byte;
        }
        if code == 0x00 {
            return Ok(Some(0xFF));
        }
        self.halt = Halt::Marker(code);
        Ok(None)
    }

    /// One bit, most significant first; `None` once the data ends.
    fn bit(&mut self) -> Result<Option<u32>, CarveError> {
        if self.bits == 0 {
            let Some(byte) = self.next_data_byte()? else {
                return Ok(None);
            };
            self.accumulator = u32::from(byte);
            self.bits = 8;
        }
        self.bits -= 1;
        Ok(Some((self.accumulator >> self.bits) & 1))
    }

    /// `count` bits as an unsigned value.
    fn receive(&mut self, count: u32) -> Result<Option<u32>, CarveError> {
        let mut value = 0_u32;
        for _ in 0..count {
            let Some(bit) = self.bit()? else {
                return Ok(None);
            };
            value = (value << 1) | bit;
        }
        Ok(Some(value))
    }

    /// Decodes one Huffman-coded symbol. Source: T.81 §F.2.2.3.
    fn symbol(&mut self, table: &HuffTable) -> Result<Option<u8>, CarveError> {
        let mut code = 0_i32;
        for length in 1..=MAX_CODE_BITS {
            let Some(bit) = self.bit()? else {
                return Ok(None);
            };
            code = (code << 1) | i32::try_from(bit).unwrap_or(0);
            if table.maxcode[length] >= code {
                let offset = usize::try_from(code - table.mincode[length]).unwrap_or(usize::MAX);
                return Ok(table
                    .valptr
                    .get(length)
                    .and_then(|base| base.checked_add(offset))
                    .filter(|at| *at < table.symbols)
                    .and_then(|at| table.values.get(at))
                    .copied());
            }
        }
        // Sixteen bits without matching a code: not this table's data.
        Ok(None)
    }

    /// Discards bits up to the next byte boundary, as a restart marker or the
    /// end of a scan requires. Source: T.81 §B.1.1.5.
    fn align(&mut self) {
        self.bits = 0;
    }

    /// The marker that stopped the scan, consuming it if it has not been met
    /// yet.
    fn marker(&mut self) -> Result<Option<u8>, CarveError> {
        match self.halt {
            Halt::Marker(code) => Ok(Some(code)),
            Halt::Ended => Ok(None),
            Halt::None => {
                // Nothing has ended the data yet, so the next bytes must be
                // the marker themselves.
                self.align();
                if next(self.bytes)? != Some(0xFF) {
                    self.halt = Halt::Ended;
                    return Ok(None);
                }
                let mut code = 0xFF_u8;
                while code == 0xFF {
                    let Some(next_byte) = next(self.bytes)? else {
                        self.halt = Halt::Ended;
                        return Ok(None);
                    };
                    code = next_byte;
                }
                self.halt = Halt::Marker(code);
                Ok(Some(code))
            }
        }
    }

    /// Clears a consumed restart marker so decoding can resume after it.
    fn resume(&mut self) {
        if matches!(self.halt, Halt::Marker(_)) {
            self.halt = Halt::None;
        }
        self.align();
    }
}

/// Decodes the scan, returning how much of the frame it accounted for.
fn decode_scan<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    frame: &Frame,
    scan: &ScanHeader,
    tables: &Tables,
    restart_interval: u16,
    watch: Option<u64>,
) -> Result<ScanOutcome, CarveError> {
    let Some(geometry) = McuGeometry::of(frame, scan) else {
        return Ok(ScanOutcome::nothing(bytes.pos(), ScanStop::Broke));
    };

    let mut reader = BitReader::new(bytes);
    let mut predictors = [0_i32; MAX_COMPONENTS];
    let mut decoded = 0_u32;
    let mut expected_restart = 0_u8;
    // The MCU the decoder was on when it first read past the watched offset.
    let mut seam_mcu = 0_u32;
    let mut seam_found = watch.is_none();

    let outcome = |reader: &BitReader<'_, '_, R>, decoded, seam_mcu, stop| ScanOutcome {
        mcus_decoded: decoded,
        mcus_required: geometry.total,
        mcus_across: geometry.across,
        mcu_rows: geometry.rows,
        seam_mcu,
        end: ByteOffset::new(reader.bytes.pos()),
        stop,
    };

    while decoded < geometry.total {
        if restart_interval > 0
            && decoded > 0
            && decoded.is_multiple_of(u32::from(restart_interval))
        {
            reader.align();
            let Some(code) = reader.marker()? else {
                return Ok(outcome(&reader, decoded, seam_mcu, ScanStop::Broke));
            };
            // Restart markers run RST0..RST7 in order. A stream that skips one
            // is not this scan.
            if !(MARKER_RST0..=MARKER_RST7).contains(&code)
                || code - MARKER_RST0 != expected_restart
            {
                return Ok(outcome(&reader, decoded, seam_mcu, ScanStop::Broke));
            }
            expected_restart = (expected_restart + 1) % 8;
            predictors = [0_i32; MAX_COMPONENTS];
            reader.resume();
        }

        if !decode_mcu(&mut reader, scan, tables, &mut predictors)? {
            return Ok(outcome(&reader, decoded, seam_mcu, ScanStop::Broke));
        }
        decoded += 1;
        if !seam_found && watch.is_some_and(|at| reader.bytes.pos() >= at) {
            seam_mcu = decoded;
            seam_found = true;
        }
    }

    // Every MCU accounted for; the frame must now end.
    reader.align();
    let stop = match reader.marker()? {
        Some(MARKER_EOI) => ScanStop::Complete,
        // A `DNL` segment may legally follow, but nothing else, and a frame
        // that needs one has already told us its height.
        _ => ScanStop::Broke,
    };
    Ok(outcome(&reader, decoded, seam_mcu, stop))
}

/// Decodes one minimum coded unit; `false` when the data is not one.
fn decode_mcu<R: Read + Seek>(
    reader: &mut BitReader<'_, '_, R>,
    scan: &ScanHeader,
    tables: &Tables,
    predictors: &mut [i32; MAX_COMPONENTS],
) -> Result<bool, CarveError> {
    for (entry, predictor) in scan
        .entries
        .iter()
        .take(scan.count)
        .zip(predictors.iter_mut())
    {
        let (Some(dc), Some(ac)) = (
            tables.dc[entry.dc_table].as_ref(),
            tables.ac[entry.ac_table].as_ref(),
        ) else {
            return Ok(false);
        };
        // An interleaved scan carries this component's sampling factors' worth
        // of blocks per MCU; a single-component scan carries exactly one.
        let blocks = if scan.count == 1 {
            1
        } else {
            entry.component.horizontal * entry.component.vertical
        };
        for _ in 0..blocks {
            if !decode_block(reader, dc, ac, predictor)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Decodes one 8x8 block's coefficients, discarding their values.
///
/// Only whether the bits form a legal block matters here: a wrong splice
/// produces a Huffman code that is not in the table, or a run that overflows
/// the block, within a handful of bytes.
fn decode_block<R: Read + Seek>(
    reader: &mut BitReader<'_, '_, R>,
    dc: &HuffTable,
    ac: &HuffTable,
    predictor: &mut i32,
) -> Result<bool, CarveError> {
    // DC coefficient: a category, then that many bits of difference.
    // Source: T.81 §F.2.2.1.
    let Some(category) = reader.symbol(dc)? else {
        return Ok(false);
    };
    // The difference category cannot exceed the coefficient's bit width.
    if category > 15 {
        return Ok(false);
    }
    if category > 0 {
        let Some(raw) = reader.receive(u32::from(category))? else {
            return Ok(false);
        };
        *predictor = predictor.wrapping_add(extend(raw, u32::from(category)));
    }

    // AC coefficients: run-length pairs until end of block.
    // Source: T.81 §F.2.2.2.
    let mut coefficient = 1_usize;
    while coefficient < BLOCK_COEFFICIENTS {
        let Some(symbol) = reader.symbol(ac)? else {
            return Ok(false);
        };
        let run = usize::from(symbol >> 4);
        let size = u32::from(symbol & 0x0F);
        if size == 0 {
            if run == 15 {
                // A run of sixteen zeroes, which still has to fit the block.
                coefficient += 16;
                continue;
            }
            // End of block.
            return Ok(true);
        }
        coefficient += run;
        if coefficient >= BLOCK_COEFFICIENTS {
            return Ok(false);
        }
        if reader.receive(size)?.is_none() {
            return Ok(false);
        }
        coefficient += 1;
    }
    Ok(coefficient == BLOCK_COEFFICIENTS)
}

/// Sign-extends a `size`-bit value the way T.81 §F.2.2.1 defines.
fn extend(value: u32, size: u32) -> i32 {
    if size == 0 {
        return 0;
    }
    let threshold = 1_u32 << (size - 1);
    let value = i64::from(value);
    if value < i64::from(threshold) {
        i32::try_from(value - (1_i64 << size) + 1).unwrap_or(0)
    } else {
        i32::try_from(value).unwrap_or(0)
    }
}

/// How the frame's samples divide into minimum coded units.
#[derive(Clone, Copy, Debug)]
struct McuGeometry {
    across: u32,
    total: u32,
    /// Pixel rows one MCU spans.
    rows: u32,
}

impl McuGeometry {
    /// Source: T.81 §A.2.2 (interleaved) and §A.2.3 (non-interleaved).
    fn of(frame: &Frame, scan: &ScanHeader) -> Option<Self> {
        let (across, down, rows) = if scan.count == 1 {
            // A single-component scan is a grid of that component's blocks.
            let component = scan.entries[0].component;
            let width = ceil_div(frame.width.checked_mul(component.horizontal)?, frame.hmax);
            let height = ceil_div(frame.height.checked_mul(component.vertical)?, frame.vmax);
            (
                ceil_div(width, BLOCK_SAMPLES),
                ceil_div(height, BLOCK_SAMPLES),
                BLOCK_SAMPLES.checked_mul(frame.vmax)? / component.vertical.max(1),
            )
        } else {
            let mcu_width = BLOCK_SAMPLES.checked_mul(frame.hmax)?;
            let mcu_height = BLOCK_SAMPLES.checked_mul(frame.vmax)?;
            (
                ceil_div(frame.width, mcu_width),
                ceil_div(frame.height, mcu_height),
                mcu_height,
            )
        };
        let total = across.checked_mul(down)?;
        if total == 0 || total > MAX_MCUS {
            return None;
        }
        Some(Self {
            across,
            total,
            rows: rows.max(1),
        })
    }
}

fn ceil_div(value: u32, divisor: u32) -> u32 {
    if divisor == 0 {
        return 0;
    }
    value.div_ceil(divisor)
}

/// Reads the next marker code, skipping fill bytes.
fn next_marker<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u8>, CarveError> {
    if next(bytes)? != Some(0xFF) {
        return Ok(None);
    }
    loop {
        match next(bytes)? {
            Some(0xFF) => {}
            other => return Ok(other),
        }
    }
}

/// Reads a length-prefixed segment's payload into `out`.
fn read_payload<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    out: &mut Vec<u8>,
) -> Result<Option<()>, CarveError> {
    let Some(len) = segment_len(bytes)? else {
        return Ok(None);
    };
    out.clear();
    if !bytes
        .read_into(out, usize::from(len))
        .map_err(|source| CarveError::io(ByteOffset::new(bytes.pos()), source))?
    {
        return Ok(None);
    }
    Ok(Some(()))
}

/// Skips a length-prefixed segment.
fn skip_payload<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<bool, CarveError> {
    let Some(len) = segment_len(bytes)? else {
        return Ok(false);
    };
    Ok(bytes.skip(u64::from(len)))
}

/// A segment's payload length: the field minus the two bytes it occupies.
/// Source: T.81 §B.1.1.4.
fn segment_len<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u16>, CarveError> {
    let (Some(hi), Some(lo)) = (next(bytes)?, next(bytes)?) else {
        return Ok(None);
    };
    let len = u16::from_be_bytes([hi, lo]);
    if len < 2 { Ok(None) } else { Ok(Some(len - 2)) }
}

fn next<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u8>, CarveError> {
    let at = bytes.pos();
    bytes
        .next()
        .map_err(|source| CarveError::io(ByteOffset::new(at), source))
}
