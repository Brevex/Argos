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

use argos_core::ByteOffset;

use crate::Bytes;
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
    /// Pixel width the frame header declares. Zero when no frame was parsed.
    ///
    /// This is what the image *claims to be*, read from the header before any
    /// of its data is decoded, so it is known for a candidate that never
    /// completes — which is what lets a search spend its budget on frames the
    /// size of photographs rather than on a thumbnail cache.
    pub width: u32,
    /// Pixel height the frame header declares. Zero when no frame was parsed.
    pub height: u32,
    /// MCU the decoder had reached as the stream crossed each watched
    /// offset, in the order the offsets were given — one per splice a
    /// reassembly is asking about. A zero means that offset was never
    /// crossed, so there is no seam to look at.
    pub seam_mcus: [u32; MAX_SEAMS],
    /// How many entries of [`ScanOutcome::seam_mcus`] are meaningful.
    pub seams: usize,
    /// First byte past the last one the decoder consumed.
    ///
    /// Includes the bytes it read while failing, so it marks where the stream
    /// stopped being this image rather than where the image stopped.
    pub end: ByteOffset,
    /// First byte past the last **complete** MCU.
    ///
    /// Everything before this decoded as picture; between here and
    /// [`ScanOutcome::end`] are the bytes the decoder read on its way to
    /// discovering they were not. Reporting a prefix of the image means
    /// reporting up to here, or the file would carry a tail of whatever
    /// followed it on the medium.
    pub settled: ByteOffset,
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
            width: 0,
            height: 0,
            seam_mcus: [0; MAX_SEAMS],
            seams: 0,
            end: ByteOffset::new(end),
            settled: ByteOffset::new(end),
            stop,
        }
    }
}

/// Fragment boundaries one scan may be asked to watch.
///
/// An assembly of `n` fragments has `n - 1` splices, and the reassembly search
/// caps a path at [`MAX_FRAGMENTS`](crate::reassemble::MAX_FRAGMENTS)
/// fragments, so this is that cap less one.
pub const MAX_SEAMS: usize = crate::reassemble::MAX_FRAGMENTS - 1;

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
    scan_watching(src, start, limit, &[], scratch)
}

/// Decodes as [`scan`] does, additionally reporting which MCU the decoder had
/// reached as the stream crossed each offset in `watch`.
///
/// The offsets are relative to `start` and must be ascending. They are how a
/// caller finds the **stitch rows**: a reassembly's fragment boundaries are
/// byte offsets, and what tells you whether a splice is real is the picture at
/// the pixel row that byte produced. Every boundary is reported, not just the
/// first — an assembly of four fragments has three splices, and one bad splice
/// anywhere makes the whole thing a fabrication.
///
/// At most [`MAX_SEAMS`] offsets are tracked; further ones are ignored.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn scan_watching<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    limit: u64,
    watch: &[u64],
    scratch: &mut Scratch,
) -> Result<ScanOutcome, CarveError> {
    let Scratch { stream, seg, .. } = scratch;
    let mut bytes = Bytes::new(src, start.get(), limit, stream);
    let header = match parse_header(&mut bytes, seg)? {
        Ok(header) => header,
        Err(stop) => return Ok(ScanOutcome::nothing(bytes.pos(), stop)),
    };
    let (outcome, _) = decode_scan(&mut bytes, &header, Cursor::default(), watch, start.get())?;
    Ok(outcome)
}

/// A decode stopped inside a fragment, ready to be carried on over whatever
/// follows it.
///
/// A gap search proposes thousands of continuations for one first fragment,
/// and decoding that fragment from `SOI` for each of them is what the search
/// actually spends its time on: the cost is linear in the fragment's size, so
/// a megabyte-long first fragment makes every hypothesis cost milliseconds and
/// a full sweep cost hours. Decoding it once and restoring this instead makes
/// a hypothesis cost what its own bytes cost.
///
/// Built by [`resume_at`] and spent by [`scan_resumed`].
#[derive(Clone, Debug)]
pub struct Resume {
    header: Header,
    cursor: Cursor,
    /// Medium offset of the first byte the resumed decode has to read. It is
    /// an MCU boundary at or below the fragment end the caller asked for, so
    /// the bytes between the two are replayed — at most one MCU's worth.
    replay_from: u64,
}

impl Resume {
    /// Where the resumed decode has to start reading.
    #[must_use]
    pub fn replay_from(&self) -> ByteOffset {
        ByteOffset::new(self.replay_from)
    }

    /// MCUs already accounted for when the decode stopped here.
    #[must_use]
    pub fn mcus_decoded(&self) -> u32 {
        self.cursor.decoded
    }

    /// MCUs the frame requires in total.
    #[must_use]
    pub fn mcus_required(&self) -> u32 {
        self.header.geometry.total
    }

    /// Pixel dimensions the frame header declares.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.header.width, self.header.height)
    }
}

/// Decodes the candidate at `start` up to `until`, keeping the position.
///
/// `None` when nothing decoded — no parsable frame, a coding this decoder does
/// not implement, or not one whole MCU inside the range.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn resume_at<R: Read + Seek>(
    src: &mut R,
    start: ByteOffset,
    until: u64,
    scratch: &mut Scratch,
) -> Result<Option<Resume>, CarveError> {
    if until <= start.get() {
        return Ok(None);
    }
    let Scratch { stream, seg, .. } = scratch;
    let mut bytes = Bytes::new(src, start.get(), until, stream);
    let Ok(header) = parse_header(&mut bytes, seg)? else {
        return Ok(None);
    };
    let (_, snapshot) = decode_scan(&mut bytes, &header, Cursor::default(), &[], start.get())?;
    Ok(snapshot.map(|snapshot| Resume {
        header,
        cursor: snapshot.cursor,
        replay_from: snapshot.at,
    }))
}

/// Carries a [`Resume`] on over `src`, which must present the replayed bytes
/// followed by the continuation being tested.
///
/// `watch` and the returned offsets are in that stream's coordinates, not the
/// medium's. MCU counts are absolute: what the resumed decode adds is added to
/// what the prefix already accounted for, so a caller compares them against
/// [`ScanOutcome::mcus_required`] exactly as an unresumed scan's.
///
/// # Errors
///
/// Fails only when reading or seeking `src` fails.
pub fn scan_resumed<R: Read + Seek>(
    resume: &Resume,
    src: &mut R,
    limit: u64,
    watch: &[u64],
    scratch: &mut Scratch,
) -> Result<ScanOutcome, CarveError> {
    let Scratch { stream, .. } = scratch;
    let mut bytes = Bytes::new(src, 0, limit, stream);
    let (outcome, _) = decode_scan(&mut bytes, &resume.header, resume.cursor, watch, 0)?;
    Ok(outcome)
}

/// What the frame headers settled, and what every hypothesis resuming inside
/// this scan shares.
///
/// Held apart from the decode position because it is the expensive half: the
/// Huffman tables are built once per candidate, while the position is copied
/// per hypothesis.
#[derive(Clone, Debug)]
struct Header {
    scan: ScanHeader,
    tables: Tables,
    geometry: McuGeometry,
    restart_interval: u16,
    width: u32,
    height: u32,
}

/// How far a scan had got, and everything needed to carry on from there.
///
/// Small and `Copy`: this is what a resumed hypothesis restores, so it must
/// not cost what re-reading the fragment would.
#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    predictors: [i32; MAX_COMPONENTS],
    decoded: u32,
    expected_restart: u8,
    /// Bits pulled but not yet consumed, most significant first.
    accumulator: u64,
    /// How many of `accumulator`'s low bits are valid.
    bits: u32,
    halt: Halt,
}

/// A decode position at an MCU boundary, and where on the medium it sits.
#[derive(Clone, Copy)]
struct Snapshot {
    cursor: Cursor,
    /// First byte the decoder had not yet consumed.
    at: u64,
}

/// Walks the marker segments up to and including `SOS`.
fn parse_header<R: Read + Seek>(
    bytes: &mut Bytes<'_, R>,
    seg: &mut Vec<u8>,
) -> Result<Result<Header, ScanStop>, CarveError> {
    let mut tables = Tables::default();
    let mut frame: Option<Frame> = None;
    let mut restart_interval = 0_u16;

    if next(bytes)? != Some(0xFF) || next(bytes)? != Some(MARKER_SOI) {
        return Ok(Err(ScanStop::Broke));
    }

    loop {
        let Some(code) = next_marker(bytes)? else {
            return Ok(Err(ScanStop::Broke));
        };
        match code {
            MARKER_DHT => {
                let Some(()) = read_payload(bytes, seg)? else {
                    return Ok(Err(ScanStop::Broke));
                };
                if tables.absorb(seg).is_none() {
                    return Ok(Err(ScanStop::Broke));
                }
            }
            MARKER_DRI => {
                let Some(()) = read_payload(bytes, seg)? else {
                    return Ok(Err(ScanStop::Broke));
                };
                let (Some(&hi), Some(&lo)) = (seg.first(), seg.get(1)) else {
                    return Ok(Err(ScanStop::Broke));
                };
                restart_interval = u16::from_be_bytes([hi, lo]);
            }
            // Baseline and extended sequential, Huffman coded.
            0xC0 | 0xC1 => {
                let Some(()) = read_payload(bytes, seg)? else {
                    return Ok(Err(ScanStop::Broke));
                };
                let Some(parsed) = Frame::parse(seg) else {
                    return Ok(Err(ScanStop::Broke));
                };
                frame = Some(parsed);
            }
            // Progressive, lossless, hierarchical and every arithmetic-coded
            // frame: outside what this decoder can check.
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Ok(Err(ScanStop::Unsupported));
            }
            MARKER_SOS => {
                let Some(frame) = frame else {
                    return Ok(Err(ScanStop::Broke));
                };
                let Some(()) = read_payload(bytes, seg)? else {
                    return Ok(Err(ScanStop::Broke));
                };
                let Some(scan) = ScanHeader::parse(seg, &frame) else {
                    return Ok(Err(ScanStop::Broke));
                };
                let Some(geometry) = McuGeometry::of(&frame, &scan) else {
                    return Ok(Err(ScanStop::Broke));
                };
                return Ok(Ok(Header {
                    scan,
                    tables,
                    geometry,
                    restart_interval,
                    width: frame.width,
                    height: frame.height,
                }));
            }
            MARKER_EOI => return Ok(Err(ScanStop::Broke)),
            // Quantisation tables, comments, application data and the rest:
            // nothing the entropy decoder needs, but their length is trusted
            // only as far as the read limit allows.
            MARKER_DQT | MARKER_DNL | 0xC8 | 0xCC | 0xE0..=0xEF | 0xFE => {
                if !skip_payload(bytes)? {
                    return Ok(Err(ScanStop::Broke));
                }
            }
            _ => return Ok(Err(ScanStop::Broke)),
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
#[derive(Clone, Debug, Default)]
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
/// Code bits one direct lookup resolves.
///
/// Eight covers the great majority of codes a JPEG encoder emits — the common
/// symbols are the short ones, which is what Huffman coding is for — and costs
/// a 512-byte table per Huffman table. Longer codes fall through to the
/// canonical walk, which is the same code that ran before and still decides
/// them (`M-DOCUMENTED-MAGIC`).
const LUT_BITS: u32 = 10;

/// Entries in a direct-lookup table: one per possible `LUT_BITS` prefix.
const LUT_SIZE: usize = 1 << LUT_BITS;

/// A prefix no code of `LUT_BITS` or fewer bits matches.
///
/// Zero cannot be a real entry because a real one carries a length of at least
/// one in its high byte, so it needs no separate flag.
const LUT_MISS: u16 = 0;

#[derive(Clone, Debug)]
struct HuffTable {
    /// Symbol and code length for every `LUT_BITS`-bit prefix, packed as
    /// `(length << 8) | symbol`, or [`LUT_MISS`].
    ///
    /// Built once per table, and a table is built once per candidate rather
    /// than per hypothesis, so this is paid for by every hypothesis that
    /// follows it.
    lut: [u16; LUT_SIZE],
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
        let mut table = Self {
            lut: [LUT_MISS; LUT_SIZE],
            mincode,
            maxcode,
            valptr,
            values: symbols,
            symbols: taken,
        };
        table.fill_lut();
        Some(table)
    }

    /// Records, for every short prefix, what the canonical walk would decide.
    ///
    /// A prefix the walk resolves at `length` bits stands for every longer
    /// prefix that begins with it, so one code fills a run of entries. A code
    /// the walk would refuse — one whose symbol index falls outside the table —
    /// is left as [`LUT_MISS`] rather than guessed at, so the walk refuses it
    /// exactly as it did before.
    fn fill_lut(&mut self) {
        for length in 1..=LUT_BITS as usize {
            if self.maxcode[length] < 0 {
                continue;
            }
            for code in self.mincode[length]..=self.maxcode[length] {
                let Ok(offset) = usize::try_from(code - self.mincode[length]) else {
                    continue;
                };
                let Some(symbol) = self
                    .valptr
                    .get(length)
                    .and_then(|base| base.checked_add(offset))
                    .filter(|at| *at < self.symbols)
                    .and_then(|at| self.values.get(at))
                    .copied()
                else {
                    continue;
                };
                let Ok(code) = usize::try_from(code) else {
                    continue;
                };
                let shift = LUT_BITS as usize - length;
                let first = code << shift;
                let length = u16::try_from(length).unwrap_or(u16::MAX);
                let entry = (length << 8) | u16::from(symbol);
                for slot in first..(first + (1 << shift)).min(LUT_SIZE) {
                    self.lut[slot] = entry;
                }
            }
        }
    }
}

/// Where the bit reader stopped feeding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Halt {
    /// Still inside entropy-coded data.
    #[default]
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
/// Bytes one fast refill pulls.
///
/// The reader is bit-serial by nature, so what it can amortise is the refill:
/// pulling seven clean bytes at once buys about eleven Huffman symbols before
/// the next check, instead of one check per eight bits. Seven rather than eight
/// leaves the accumulator room to hold a partial byte's leftover bits without
/// the top of a `u64` falling off (`M-DOCUMENTED-MAGIC`).
const FAST_FILL_BYTES: u32 = 7;

struct BitReader<'a, 'b, R> {
    bytes: &'a mut Bytes<'b, R>,
    /// Bits pulled but not yet consumed, most significant first.
    accumulator: u64,
    /// How many of `accumulator`'s low bits are valid.
    bits: u32,
    /// Whole trailing bytes of `accumulator` that a fast refill pulled.
    ///
    /// Those cost exactly one physical byte each, so they are the ones
    /// [`BitReader::settle`] may give back. A byte the slow path pulled may
    /// have been `FF 00` and cost two, and is never counted here.
    clean: u32,
    halt: Halt,
}

impl<'a, 'b, R: Read + Seek> BitReader<'a, 'b, R> {
    /// A reader carrying on from where `cursor` left off.
    ///
    /// The leftover bits matter: a scan boundary almost never falls on a byte
    /// boundary, so a resume that started a fresh accumulator would drop the
    /// tail of the last code it read.
    fn resuming(bytes: &'a mut Bytes<'b, R>, cursor: Cursor) -> Self {
        Self {
            bytes,
            accumulator: cursor.accumulator,
            bits: cursor.bits,
            // A cursor is only ever taken settled, so nothing it carries is a
            // whole byte this reader may hand back.
            clean: 0,
            halt: cursor.halt,
        }
    }

    /// Pulls [`FAST_FILL_BYTES`] at once when the next ones carry no `0xFF`.
    ///
    /// Only ever called with an empty accumulator, which is what keeps the
    /// arithmetic in [`BitReader::settle`] exact: everything buffered is then
    /// whole, clean bytes.
    #[inline]
    fn fast_fill(&mut self) -> bool {
        debug_assert_eq!(self.bits, 0, "a fast refill assumes an empty accumulator");
        let Some(packed) = self.bytes.take_clean(FAST_FILL_BYTES as usize) else {
            return false;
        };
        self.accumulator = packed;
        self.bits = FAST_FILL_BYTES * 8;
        self.clean = FAST_FILL_BYTES;
        true
    }

    /// Gives back every whole buffered byte, restoring the one-byte lookahead
    /// the rest of the decoder is written against.
    ///
    /// The source position is what a caller reports as `end`, as `settled`, and
    /// as the offset a splice was crossed at, so it has to mean the same thing
    /// it meant when the reader never held more than a byte. Reading ahead is
    /// therefore something that happens only *within* an MCU: at every boundary
    /// where the position is observed, the lookahead is handed back first.
    #[inline]
    fn settle(&mut self) {
        let whole = (self.bits / 8).min(self.clean);
        if whole == 0 {
            return;
        }
        let shift = whole * 8;
        self.bytes.rewind(whole as usize);
        self.accumulator >>= shift;
        self.bits -= shift;
        self.clean -= whole;
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
    #[inline]
    fn bit(&mut self) -> Result<Option<u32>, CarveError> {
        if self.bits == 0 && !self.fast_fill() {
            let Some(byte) = self.next_data_byte()? else {
                return Ok(None);
            };
            self.accumulator = u64::from(byte);
            self.bits = 8;
            self.clean = 0;
        }
        self.bits -= 1;
        Ok(Some(((self.accumulator >> self.bits) & 1) as u32))
    }

    /// `count` bits as an unsigned value.
    ///
    /// Taken in one piece when they are already buffered, which reads nothing
    /// and so cannot change where the data ends; otherwise bit by bit, which is
    /// the path that refills and meets the markers.
    #[inline]
    fn receive(&mut self, count: u32) -> Result<Option<u32>, CarveError> {
        if count == 0 {
            return Ok(Some(0));
        }
        if self.bits >= count {
            self.bits -= count;
            let mask = (1_u64 << count) - 1;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "count is at most the 16 bits a coefficient category can ask for"
            )]
            return Ok(Some(((self.accumulator >> self.bits) & mask) as u32));
        }
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
    ///
    /// The lookup resolves a short code from bits already buffered, which is
    /// most codes most of the time. It is entered only with [`LUT_BITS`] bits
    /// in hand, so it reads nothing and cannot change where the data ends; a
    /// prefix it does not resolve falls through to the walk below, which is
    /// what decided every code before the table existed and still decides the
    /// long ones.
    #[inline]
    fn symbol(&mut self, table: &HuffTable) -> Result<Option<u8>, CarveError> {
        if self.bits >= LUT_BITS {
            // The mask bounds the value to LUT_SIZE, so the table indexes it.
            let prefix = usize::try_from(
                (self.accumulator >> (self.bits - LUT_BITS)) & (LUT_SIZE as u64 - 1),
            )
            .unwrap_or(0);
            let entry = table.lut[prefix];
            if entry != LUT_MISS {
                self.bits -= u32::from(entry >> 8);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the low byte of the entry is the symbol"
                )]
                return Ok(Some(entry as u8));
            }
        }
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
        // The lookahead goes back before the partial byte is discarded, or the
        // bytes behind it would be discarded with it.
        self.settle();
        self.bits = 0;
        self.clean = 0;
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
    header: &Header,
    start: Cursor,
    watch: &[u64],
    origin: u64,
) -> Result<(ScanOutcome, Option<Snapshot>), CarveError> {
    let Header {
        scan,
        tables,
        geometry,
        restart_interval,
        ..
    } = header;

    let geometry = *geometry;
    let restart_interval = *restart_interval;
    let (width, height) = (header.width, header.height);

    let mut reader = BitReader::resuming(bytes, start);
    let mut predictors = start.predictors;
    let mut decoded = start.decoded;
    let mut expected_restart = start.expected_restart;
    // Absolute positions of the boundaries to watch, and the MCU the decoder
    // had reached as it crossed each. Watching every boundary is what makes a
    // multi-fragment assembly checkable: the caller judges each splice, and
    // one bad splice condemns the assembly however clean the others are.
    let mut watched = [0_u64; MAX_SEAMS];
    let seams = watch.len().min(MAX_SEAMS);
    for (slot, at) in watched[..seams].iter_mut().zip(watch) {
        *slot = origin.saturating_add(*at);
    }
    let mut seam_mcus = [0_u32; MAX_SEAMS];
    let mut crossed = 0_usize;
    // The last MCU boundary reached, which is where a later hypothesis may
    // pick the decode up. Only a boundary will do: mid-MCU the predictors and
    // the bit accumulator describe a block that is half read.
    let mut snapshot: Option<Snapshot> = None;

    let outcome =
        |reader: &BitReader<'_, '_, R>, decoded, seam_mcus, settled: Option<Snapshot>, stop| {
            ScanOutcome {
                mcus_decoded: decoded,
                mcus_required: geometry.total,
                mcus_across: geometry.across,
                mcu_rows: geometry.rows,
                width,
                height,
                seam_mcus,
                seams,
                end: ByteOffset::new(reader.bytes.pos()),
                settled: ByteOffset::new(settled.map_or_else(|| reader.bytes.pos(), |at| at.at)),
                stop,
            }
        };

    while decoded < geometry.total {
        if restart_interval > 0
            && decoded > 0
            && decoded.is_multiple_of(u32::from(restart_interval))
        {
            reader.align();
            let Some(code) = reader.marker()? else {
                reader.settle();
                return Ok((
                    outcome(&reader, decoded, seam_mcus, snapshot, ScanStop::Broke),
                    snapshot,
                ));
            };
            // Restart markers run RST0..RST7 in order. A stream that skips one
            // is not this scan.
            if !(MARKER_RST0..=MARKER_RST7).contains(&code)
                || code - MARKER_RST0 != expected_restart
            {
                reader.settle();
                return Ok((
                    outcome(&reader, decoded, seam_mcus, snapshot, ScanStop::Broke),
                    snapshot,
                ));
            }
            expected_restart = (expected_restart + 1) % 8;
            predictors = [0_i32; MAX_COMPONENTS];
            reader.resume();
        }

        if !decode_mcu(&mut reader, scan, tables, &mut predictors)? {
            reader.settle();
            return Ok((
                outcome(&reader, decoded, seam_mcus, snapshot, ScanStop::Broke),
                snapshot,
            ));
        }
        decoded += 1;
        reader.settle();
        snapshot = Some(Snapshot {
            cursor: Cursor {
                predictors,
                decoded,
                expected_restart,
                accumulator: reader.accumulator,
                bits: reader.bits,
                halt: reader.halt,
            },
            at: reader.bytes.pos(),
        });
        // The boundaries ascend, so each is crossed in turn.
        while crossed < seams && reader.bytes.pos() >= watched[crossed] {
            seam_mcus[crossed] = decoded;
            crossed += 1;
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
    reader.settle();
    Ok((
        outcome(&reader, decoded, seam_mcus, snapshot, stop),
        snapshot,
    ))
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

/// The offset is read only on the error path, and only `refill` can fail —
/// which leaves the cursor untouched, so the position after a failure is the
/// position before it. Taking it eagerly would cost a load and an add on every
/// byte of every hypothesis for a value almost nothing ever reads.
#[inline]
fn next<R: Read + Seek>(bytes: &mut Bytes<'_, R>) -> Result<Option<u8>, CarveError> {
    match bytes.next() {
        Ok(byte) => Ok(byte),
        Err(source) => Err(CarveError::io(ByteOffset::new(bytes.pos()), source)),
    }
}

#[cfg(test)]
mod tests {
    use super::{HuffTable, LUT_BITS, LUT_MISS, LUT_SIZE, MAX_CODE_BITS};

    /// What the canonical walk of T.81 §F.2.2.3 decides for a short prefix.
    ///
    /// A transcription of the loop in [`BitReader::symbol`], reading bits from
    /// `prefix` instead of from a stream, so the two can be compared without
    /// one of them being the other.
    fn canonical(table: &HuffTable, prefix: u16) -> Option<(u8, u32)> {
        let mut code = 0_i32;
        for length in 1..=LUT_BITS as usize {
            let bit = i32::from((prefix >> (LUT_BITS as usize - length)) & 1);
            code = (code << 1) | bit;
            if table.maxcode[length] >= code {
                let offset = usize::try_from(code - table.mincode[length]).unwrap_or(usize::MAX);
                let symbol = table
                    .valptr
                    .get(length)
                    .and_then(|base| base.checked_add(offset))
                    .filter(|at| *at < table.symbols)
                    .and_then(|at| table.values.get(at))
                    .copied()?;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "length is bounded by LUT_BITS"
                )]
                return Some((symbol, length as u32));
            }
        }
        None
    }

    /// Tables spanning the shapes a Huffman table can take: every code one
    /// length, codes spread over many lengths, a single code, a full byte of
    /// codes, and counts that describe more symbols than are supplied.
    fn tables() -> Vec<HuffTable> {
        let mut built = Vec::new();
        let values: Vec<u8> = (0..=255_u8).collect();

        for length in 1..=MAX_CODE_BITS {
            let mut counts = [0_u8; MAX_CODE_BITS];
            let available = 1_usize << length.min(8);
            counts[length - 1] = u8::try_from(available.min(255)).unwrap_or(255);
            if let Some(table) = HuffTable::build(&counts, &values) {
                built.push(table);
            }
        }

        // A spread: one code of every length, which is what an encoder that
        // has seen a wide alphabet produces.
        let spread = [1_u8; MAX_CODE_BITS];
        if let Some(table) = HuffTable::build(&spread, &values) {
            built.push(table);
        }

        // Counts that promise more symbols than the value list carries: the
        // walk refuses those, and the table must refuse them the same way.
        let mut greedy = [0_u8; MAX_CODE_BITS];
        greedy[7] = 200;
        if let Some(table) = HuffTable::build(&greedy, &values[..4]) {
            built.push(table);
        }

        assert!(built.len() > 4, "the shapes must actually build");
        built
    }

    /// The lookup is not a faster approximation of the walk; it is the walk's
    /// answer, precomputed. Nothing else would let it stand in front of the
    /// oracle that decides what counts as a recovered photograph.
    ///
    /// This is exhaustive rather than sampled: the domain is every prefix of
    /// [`LUT_BITS`] bits, all `LUT_SIZE` of them, so checking each one checks
    /// the whole of it.
    #[test]
    fn the_lookup_agrees_with_the_canonical_walk_on_every_prefix() {
        for table in tables() {
            for prefix in 0..LUT_SIZE {
                let byte = u16::try_from(prefix).expect("LUT_SIZE fits a u16");
                let entry = table.lut[prefix];
                match canonical(&table, byte) {
                    Some((symbol, length)) => assert_eq!(
                        entry,
                        (u16::try_from(length).expect("bounded by LUT_BITS") << 8)
                            | u16::from(symbol),
                        "prefix {byte:08b}: the walk decodes {symbol} in {length} bits and the \
                         table does not agree"
                    ),
                    None => assert_eq!(
                        entry, LUT_MISS,
                        "prefix {byte:08b}: the walk resolves nothing here, so the table must \
                         send it to the walk rather than answer for it"
                    ),
                }
            }
        }
    }
}
