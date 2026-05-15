use crate::error::{ArgosError, ValidationKind};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const DHT: u8 = 0xC4;
const DQT: u8 = 0xDB;
const SOF0: u8 = 0xC0;
const SOF1: u8 = 0xC1;
const SOF2: u8 = 0xC2;
const SOF3: u8 = 0xC3;
const RST_LOW: u8 = 0xD0;
const RST_HIGH: u8 = 0xD7;
const MAX_DC_CATEGORY: u8 = 11;
const MAX_AC_CATEGORY: u8 = 10;
const COEFFICIENTS_PER_BLOCK: usize = 64;
const ZERO_DOMINANCE_THRESHOLD: f32 = 0.8;

#[derive(Debug, Clone)]
struct Segment {
    marker: u8,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct HuffmanLut {
    mincode: [i32; 17],
    maxcode: [i32; 17],
    valptr: [usize; 17],
    values: Vec<u8>,
}

impl HuffmanLut {
    fn from_segment_data(segment_body: &[u8]) -> Result<(u8, u8, Self), ArgosError> {
        if segment_body.len() < 17 {
            return Err(ArgosError::Validation {
                kind: ValidationKind::BadHuffmanTable,
            });
        }
        let class = (segment_body[0] >> 4) & 0x0F;
        let id = segment_body[0] & 0x0F;
        if id > 3 {
            return Err(ArgosError::Validation {
                kind: ValidationKind::BadHuffmanTable,
            });
        }
        let mut bits = [0u8; 16];
        bits.copy_from_slice(&segment_body[1..17]);
        let total: usize = bits.iter().map(|&b| b as usize).sum();
        if segment_body.len() < 17 + total {
            return Err(ArgosError::Validation {
                kind: ValidationKind::BadHuffmanTable,
            });
        }
        let values = segment_body[17..17 + total].to_vec();

        let mut mincode = [-1i32; 17];
        let mut maxcode = [-1i32; 17];
        let mut valptr = [0usize; 17];
        let mut code: i32 = 0;
        let mut value_index = 0usize;
        for length in 1..=16usize {
            let count = bits[length - 1] as usize;
            if count > 0 {
                valptr[length] = value_index;
                mincode[length] = code;
                value_index += count;
                code += count as i32;
                maxcode[length] = code - 1;
            }
            code <<= 1;
        }
        if value_index != total {
            return Err(ArgosError::Validation {
                kind: ValidationKind::BadHuffmanTable,
            });
        }
        Ok((
            class,
            id,
            Self {
                mincode,
                maxcode,
                valptr,
                values,
            },
        ))
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u64,
    bit_count: u8,
    marker_seen: Option<u8>,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_buf: 0,
            bit_count: 0,
            marker_seen: None,
        }
    }

    fn refill(&mut self) {
        while self.bit_count <= 56 && self.pos < self.data.len() && self.marker_seen.is_none() {
            let byte = self.data[self.pos];
            self.pos += 1;
            if byte == 0xFF {
                if self.pos >= self.data.len() {
                    self.marker_seen = Some(0);
                    return;
                }
                let stuffed = self.data[self.pos];
                self.pos += 1;
                if stuffed == 0x00 {
                    self.bit_buf = (self.bit_buf << 8) | 0xFF;
                    self.bit_count += 8;
                } else {
                    self.marker_seen = Some(stuffed);
                    return;
                }
            } else {
                self.bit_buf = (self.bit_buf << 8) | byte as u64;
                self.bit_count += 8;
            }
        }
    }

    fn receive(&mut self, n: u8) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.bit_count < n {
            self.refill();
            if self.bit_count < n {
                return None;
            }
        }
        let shift = self.bit_count - n;
        let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
        let value = ((self.bit_buf >> shift) & mask) as u32;
        self.bit_count -= n;
        self.bit_buf &= (1u64 << self.bit_count).wrapping_sub(1);
        Some(value)
    }
}

fn decode_symbol(bits: &mut BitReader, lut: &HuffmanLut) -> Option<u8> {
    let mut code: i32 = bits.receive(1)? as i32;
    for length in 1..=16usize {
        if lut.maxcode[length] >= 0 && code <= lut.maxcode[length] {
            let index = lut.valptr[length] + (code - lut.mincode[length]) as usize;
            return lut.values.get(index).copied();
        }
        if length == 16 {
            return None;
        }
        let next_bit = bits.receive(1)? as i32;
        code = (code << 1) | next_bit;
    }
    None
}

fn decode_block(bits: &mut BitReader, dc_lut: &HuffmanLut, ac_lut: &HuffmanLut) -> Option<()> {
    let dc_category = decode_symbol(bits, dc_lut)?;
    if dc_category > MAX_DC_CATEGORY {
        return None;
    }
    if dc_category > 0 {
        bits.receive(dc_category)?;
    }

    let mut k = 1usize;
    while k < COEFFICIENTS_PER_BLOCK {
        let rs = decode_symbol(bits, ac_lut)?;
        let run = ((rs >> 4) & 0x0F) as usize;
        let category = rs & 0x0F;
        if category == 0 {
            if run == 15 {
                k += 16;
                if k > COEFFICIENTS_PER_BLOCK {
                    return None;
                }
                continue;
            }
            return Some(());
        }
        if category > MAX_AC_CATEGORY {
            return None;
        }
        let skip = k.checked_add(run)?;
        if skip >= COEFFICIENTS_PER_BLOCK {
            return None;
        }
        bits.receive(category)?;
        k = skip + 1;
    }
    Some(())
}

#[derive(Debug, Clone, Copy)]
struct FrameComponent {
    id: u8,
    h_samp: u8,
    v_samp: u8,
    qt_idx: u8,
}

#[derive(Debug, Clone)]
struct Frame {
    width: u16,
    height: u16,
    components: Vec<FrameComponent>,
}

#[derive(Debug, Clone, Copy)]
struct ScanComponent {
    h_samp: u8,
    v_samp: u8,
    dc_idx: u8,
    ac_idx: u8,
}

fn parse_frame(body: &[u8]) -> Option<Frame> {
    if body.len() < 6 {
        return None;
    }
    let precision = body[0];
    if precision != 8 {
        return None;
    }
    let height = u16::from_be_bytes([body[1], body[2]]);
    let width = u16::from_be_bytes([body[3], body[4]]);
    let nf = body[5] as usize;
    if nf == 0 || nf > 4 {
        return None;
    }
    if body.len() < 6 + 3 * nf {
        return None;
    }
    let mut components = Vec::with_capacity(nf);
    for k in 0..nf {
        let base = 6 + k * 3;
        let id = body[base];
        let sampling = body[base + 1];
        let qt_idx = body[base + 2];
        let h_samp = (sampling >> 4) & 0x0F;
        let v_samp = sampling & 0x0F;
        if h_samp == 0 || v_samp == 0 || h_samp > 4 || v_samp > 4 || qt_idx > 3 {
            return None;
        }
        components.push(FrameComponent {
            id,
            h_samp,
            v_samp,
            qt_idx,
        });
    }
    Some(Frame {
        width,
        height,
        components,
    })
}

fn parse_scan_components(body: &[u8], frame: &Frame) -> Option<Vec<ScanComponent>> {
    if body.is_empty() {
        return None;
    }
    let ns = body[0] as usize;
    if ns == 0 || ns > 4 {
        return None;
    }
    if body.len() < 1 + 2 * ns + 3 {
        return None;
    }
    let mut scan = Vec::with_capacity(ns);
    for k in 0..ns {
        let base = 1 + k * 2;
        let cs = body[base];
        let tdta = body[base + 1];
        let dc_idx = (tdta >> 4) & 0x0F;
        let ac_idx = tdta & 0x0F;
        if dc_idx > 3 || ac_idx > 3 {
            return None;
        }
        let comp = frame.components.iter().find(|c| c.id == cs)?;
        scan.push(ScanComponent {
            h_samp: comp.h_samp,
            v_samp: comp.v_samp,
            dc_idx,
            ac_idx,
        });
    }
    Some(scan)
}

fn record_quant_tables(body: &[u8], present: &mut [bool; 4]) {
    let mut offset = 0;
    while offset < body.len() {
        let header = body[offset];
        let precision = (header >> 4) & 0x0F;
        let table_id = (header & 0x0F) as usize;
        let entry_size = if precision == 0 { 64 } else { 128 };
        if table_id < 4 && offset + 1 + entry_size <= body.len() {
            present[table_id] = true;
        }
        if offset + 1 + entry_size > body.len() {
            return;
        }
        offset += 1 + entry_size;
    }
}

type HuffmanLutTable = [Option<HuffmanLut>; 4];

fn collect_huffman_luts(
    segments: &[Segment],
) -> Result<(HuffmanLutTable, HuffmanLutTable), ArgosError> {
    let mut dc_luts: [Option<HuffmanLut>; 4] = [None, None, None, None];
    let mut ac_luts: [Option<HuffmanLut>; 4] = [None, None, None, None];
    for seg in segments.iter().filter(|s| s.marker == DHT) {
        let mut offset = 0;
        while offset < seg.data.len() {
            let (class, id, lut) = HuffmanLut::from_segment_data(&seg.data[offset..])?;
            let consumed = 17 + lut.values.len();
            if class == 0 {
                dc_luts[id as usize] = Some(lut);
            } else {
                ac_luts[id as usize] = Some(lut);
            }
            offset += consumed;
        }
    }
    Ok((dc_luts, ac_luts))
}

fn is_baseline_marker(marker: u8) -> bool {
    marker == SOF0
}

fn is_sof_marker(marker: u8) -> bool {
    matches!(marker, SOF0 | SOF1 | SOF2 | SOF3)
}

#[derive(Debug)]
struct ParsedJpeg {
    segments: Vec<Segment>,
    entropy_start: usize,
    entropy_end: usize,
}

fn parse_jpeg(data: &[u8]) -> Result<ParsedJpeg, ArgosError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != SOI {
        return Err(ArgosError::Validation {
            kind: ValidationKind::MissingSoi,
        });
    }

    let mut segments = Vec::new();
    let mut i = 2;
    let mut entropy_start = None;

    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0x00 {
            i += 2;
            continue;
        }
        if marker == EOI {
            break;
        }
        if i + 3 >= data.len() {
            return Err(ArgosError::Validation {
                kind: ValidationKind::TruncatedSegment,
            });
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 || i + 2 + len > data.len() {
            return Err(ArgosError::Validation {
                kind: ValidationKind::TruncatedSegment,
            });
        }
        segments.push(Segment {
            marker,
            data: data[i + 4..i + 2 + len].to_vec(),
        });
        i += 2 + len;
        if marker == SOS {
            entropy_start = Some(i);
            break;
        }
    }

    let entropy_end =
        find_eoi_offset(data, entropy_start.unwrap_or(i)).ok_or(ArgosError::Validation {
            kind: ValidationKind::MissingEoi,
        })?;

    Ok(ParsedJpeg {
        segments,
        entropy_start: entropy_start.unwrap_or(entropy_end),
        entropy_end,
    })
}

fn find_eoi_offset(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < data.len() {
        if data[i] == 0xFF {
            let next = data[i + 1];
            if next == EOI {
                return Some(i);
            }
            if next == 0x00 || (RST_LOW..=RST_HIGH).contains(&next) {
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    None
}

fn mcus_expected(frame: &Frame, scan: &[ScanComponent]) -> usize {
    let max_h = scan.iter().map(|c| c.h_samp).max().unwrap_or(1).max(1) as usize;
    let max_v = scan.iter().map(|c| c.v_samp).max().unwrap_or(1).max(1) as usize;
    let pixels_per_mcu_w = max_h * 8;
    let pixels_per_mcu_v = max_v * 8;
    let mcus_w = (frame.width as usize).div_ceil(pixels_per_mcu_w);
    let mcus_h = (frame.height as usize).div_ceil(pixels_per_mcu_v);
    mcus_w.saturating_mul(mcus_h)
}

fn decode_mcu(
    bits: &mut BitReader,
    scan: &[ScanComponent],
    dc_luts: &[Option<HuffmanLut>; 4],
    ac_luts: &[Option<HuffmanLut>; 4],
) -> Option<()> {
    for comp in scan {
        let dc_lut = dc_luts[comp.dc_idx as usize].as_ref()?;
        let ac_lut = ac_luts[comp.ac_idx as usize].as_ref()?;
        let blocks_in_mcu = comp.h_samp as usize * comp.v_samp as usize;
        for _ in 0..blocks_in_mcu {
            decode_block(bits, dc_lut, ac_lut)?;
        }
    }
    Some(())
}

pub fn validate(data: &[u8]) -> Result<f32, ArgosError> {
    let parsed = match parse_jpeg(data) {
        Ok(p) => p,
        Err(ArgosError::Validation { .. }) => return Ok(0.0),
        Err(e) => return Err(e),
    };

    let Some(sof) = parsed.segments.iter().find(|s| is_sof_marker(s.marker)) else {
        return Ok(0.0);
    };
    let Some(sos_seg) = parsed.segments.iter().find(|s| s.marker == SOS) else {
        return Ok(0.0);
    };
    let has_dht = parsed.segments.iter().any(|s| s.marker == DHT);
    let has_dqt = parsed.segments.iter().any(|s| s.marker == DQT);
    if !has_dht || !has_dqt {
        return Ok(0.0);
    }

    if !is_baseline_marker(sof.marker) {
        return Ok(0.5);
    }

    let Some(frame) = parse_frame(&sof.data) else {
        return Ok(0.0);
    };

    let mut qt_present = [false; 4];
    for seg in parsed.segments.iter().filter(|s| s.marker == DQT) {
        record_quant_tables(&seg.data, &mut qt_present);
    }
    for comp in &frame.components {
        if comp.qt_idx >= 4 || !qt_present[comp.qt_idx as usize] {
            return Ok(0.0);
        }
    }

    let (dc_luts, ac_luts) = match collect_huffman_luts(&parsed.segments) {
        Ok(p) => p,
        Err(_) => return Ok(0.0),
    };

    let Some(scan) = parse_scan_components(&sos_seg.data, &frame) else {
        return Ok(0.0);
    };

    let expected_mcus = mcus_expected(&frame, &scan);
    if expected_mcus == 0 {
        return Ok(0.0);
    }

    let entropy = &data[parsed.entropy_start..parsed.entropy_end];
    let mut bits = BitReader::new(entropy);
    let mut decoded = 0usize;

    while decoded < expected_mcus {
        if decode_mcu(&mut bits, &scan, &dc_luts, &ac_luts).is_none() {
            break;
        }
        decoded += 1;
    }

    Ok((decoded as f32 / expected_mcus as f32).min(1.0))
}

pub fn continuation_score(block: &[u8]) -> f32 {
    if block.is_empty() {
        return 0.0;
    }
    let zeros = block.iter().filter(|&&b| b == 0).count();
    let zero_ratio = zeros as f32 / block.len() as f32;
    if zero_ratio > ZERO_DOMINANCE_THRESHOLD {
        return 0.1;
    }
    for w in block.windows(2) {
        if w[0] == 0xFF && (w[1] == EOI || (w[1] >= RST_LOW && w[1] <= RST_HIGH)) {
            return 0.3;
        }
    }
    0.8
}
