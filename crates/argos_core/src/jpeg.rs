use crate::{CoreError, Result};
use std::collections::HashMap;

pub const SOI: [u8; 2] = [0xFF, 0xD8];
pub const EOI: [u8; 2] = [0xFF, 0xD9];
pub const SOS: u8 = 0xDA;
pub const DQT: u8 = 0xDB;
pub const DHT: u8 = 0xC4;
pub const SOF0: u8 = 0xC0;
pub const SOF2: u8 = 0xC2;
pub const DRI: u8 = 0xDD;
pub const RST0: u8 = 0xD0;
pub const RST7: u8 = 0xD7;

#[inline]
pub const fn is_restart_marker(marker: u8) -> bool {
    marker >= RST0 && marker <= RST7
}

#[inline]
pub const fn is_sof_marker(marker: u8) -> bool {
    matches!(marker, SOF0 | 0xC1 | SOF2 | 0xC3 | 0xC5..=0xCF)
}

#[inline]
pub const fn is_standalone_marker(marker: u8) -> bool {
    matches!(marker, 0xD8 | 0xD9) || is_restart_marker(marker) || marker == 0x01
}

#[inline]
pub const fn restart_marker_index(marker: u8) -> Option<u8> {
    if is_restart_marker(marker) {
        Some(marker - RST0)
    } else {
        None
    }
}

#[inline]
pub fn quick_validate_header(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    if data[0] != 0xFF || data[1] != 0xD8 {
        return false;
    }

    if data[2] != 0xFF {
        return false;
    }

    let marker = data[3];
    matches!(marker,
        0xE0..=0xEF |
        0xDB |
        0xC4 |
        0xC0..=0xC3 | 0xC5..=0xCF |
        0xDD |
        0xFE
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerType {
    Soi,
    Eoi,
    Sos,
    Dqt,
    Dht,
    Sof(u8),
    Dri,
    App(u8),
    Com,
    Rst(u8),
    Other(u8),
}

impl MarkerType {
    #[inline]
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0xD8 => Self::Soi,
            0xD9 => Self::Eoi,
            0xDA => Self::Sos,
            0xDB => Self::Dqt,
            0xC4 => Self::Dht,
            0xDD => Self::Dri,
            0xFE => Self::Com,
            b if (0xD0..=0xD7).contains(&b) => Self::Rst(b - 0xD0),
            b if (0xE0..=0xEF).contains(&b) => Self::App(b - 0xE0),
            b if is_sof_marker(b) => Self::Sof(b),
            b => Self::Other(b),
        }
    }

    #[inline]
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Soi => 0xD8,
            Self::Eoi => 0xD9,
            Self::Sos => 0xDA,
            Self::Dqt => 0xDB,
            Self::Dht => 0xC4,
            Self::Dri => 0xDD,
            Self::Com => 0xFE,
            Self::Rst(n) => 0xD0 + n,
            Self::App(n) => 0xE0 + n,
            Self::Sof(b) | Self::Other(b) => b,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct JpegMarker {
    pub marker_type: MarkerType,
    pub offset: u64,
    pub length: u16,
}

impl JpegMarker {
    #[inline]
    pub const fn total_size(&self) -> u64 {
        if self.length == 0 {
            2
        } else {
            2 + self.length as u64
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThumbnailInfo {
    pub soi_offset: u64,
    pub eoi_offset: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct JpegStructure {
    pub markers: Vec<JpegMarker>,
    pub sos_offset: Option<u64>,
    pub image_width: u16,
    pub image_height: u16,
    pub is_progressive: bool,
    pub restart_interval: u16,
    pub thumbnail: Option<ThumbnailInfo>,
    pub corruption_point: Option<u64>,
    pub valid_end_offset: u64,
    pub skipped_segments: u32,
    pub is_truncated: bool,
    pub has_valid_content: bool,
}

pub struct JpegParser;

impl JpegParser {
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    pub fn parse(&self, data: &[u8]) -> Result<JpegStructure> {
        if data.len() < 4 {
            return Err(CoreError::InvalidFormat("Data too short for JPEG".into()));
        }
        if data[0] != 0xFF || data[1] != 0xD8 {
            return Err(CoreError::InvalidFormat("Missing JPEG SOI marker".into()));
        }

        let mut structure = JpegStructure::default();
        structure.markers.push(JpegMarker {
            marker_type: MarkerType::Soi,
            offset: 0,
            length: 0,
        });
        let mut pos: usize = 2;
        let mut found_valid_marker = false;

        while pos < data.len().saturating_sub(1) {
            if data[pos] != 0xFF {
                pos += 1;
                continue;
            }
            while pos < data.len().saturating_sub(1) && data[pos + 1] == 0xFF {
                pos += 1;
            }
            if pos >= data.len().saturating_sub(1) {
                break;
            }

            let marker_byte = data[pos + 1];
            if marker_byte == 0x00 {
                pos += 2;
                continue;
            }

            let marker_type = MarkerType::from_byte(marker_byte);
            let marker_offset = pos as u64;

            if matches!(
                marker_type,
                MarkerType::App(_)
                    | MarkerType::Dqt
                    | MarkerType::Dht
                    | MarkerType::Sof(_)
                    | MarkerType::Sos
                    | MarkerType::Dri
                    | MarkerType::Com
            ) {
                found_valid_marker = true;
            }

            if is_standalone_marker(marker_byte) {
                structure.markers.push(JpegMarker {
                    marker_type,
                    offset: marker_offset,
                    length: 0,
                });
                if matches!(marker_type, MarkerType::Eoi) {
                    structure.valid_end_offset = pos as u64 + 2;
                    structure.has_valid_content = found_valid_marker;
                    return Ok(structure);
                }
                pos += 2;
                continue;
            }

            if pos + 3 >= data.len() {
                structure.is_truncated = true;
                structure.valid_end_offset = pos as u64;
                break;
            }
            let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);

            if length < 2 || pos + 2 + length as usize > data.len() {
                structure.skipped_segments += 1;
                if structure.corruption_point.is_none() {
                    structure.corruption_point = Some(pos as u64);
                }

                pos += 2;
                while pos < data.len().saturating_sub(1) {
                    if data[pos] == 0xFF && data[pos + 1] != 0x00 && data[pos + 1] != 0xFF {
                        let next_marker = data[pos + 1];
                        if matches!(next_marker,
                            0xC0..=0xCF | 0xD0..=0xD9 | 0xDA..=0xDF | 0xE0..=0xEF | 0xFE
                        ) {
                            break;
                        }
                    }
                    pos += 1;
                }
                continue;
            }

            structure.markers.push(JpegMarker {
                marker_type,
                offset: marker_offset,
                length,
            });

            match marker_type {
                MarkerType::Sof(_) => {
                    if pos + 9 <= data.len() {
                        structure.image_height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                        structure.image_width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                    }
                    if marker_byte == SOF2 {
                        structure.is_progressive = true;
                    }
                }
                MarkerType::Sos => {
                    structure.sos_offset = Some(marker_offset);
                    pos += 2 + length as usize;

                    while pos < data.len().saturating_sub(1) {
                        if data[pos] == 0xFF && data[pos + 1] != 0x00 {
                            let next = data[pos + 1];
                            if next == 0xD9 {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Eoi,
                                    offset: pos as u64,
                                    length: 0,
                                });
                                structure.valid_end_offset = pos as u64 + 2;
                                structure.has_valid_content = found_valid_marker;
                                return Ok(structure);
                            } else if is_restart_marker(next) {
                                structure.markers.push(JpegMarker {
                                    marker_type: MarkerType::Rst(next - 0xD0),
                                    offset: pos as u64,
                                    length: 0,
                                });
                                pos += 2;
                            } else if next == 0xFF {
                                pos += 1;
                            } else {
                                break;
                            }
                        } else {
                            pos += 1;
                        }
                    }

                    structure.is_truncated = true;
                    structure.valid_end_offset = pos as u64;
                    continue;
                }
                MarkerType::Dri => {
                    if length >= 4 && pos + 5 < data.len() {
                        structure.restart_interval =
                            u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
                    }
                }
                MarkerType::App(1) => {
                    self.parse_app1_exif(data, pos + 4, length - 2, &mut structure);
                }
                _ => {}
            }
            pos += 2 + length as usize;
        }

        if structure.valid_end_offset == 0 {
            structure.valid_end_offset = pos as u64;
            structure.is_truncated = true;
        }
        structure.has_valid_content = found_valid_marker;
        Ok(structure)
    }

    fn parse_app1_exif(
        &self,
        data: &[u8],
        offset: usize,
        length: u16,
        structure: &mut JpegStructure,
    ) {
        if length < 14 || offset + 14 > data.len() {
            return;
        }
        if &data[offset..offset + 6] != b"Exif\0\0" {
            return;
        }
        let segment_end = offset + length as usize;
        for i in (offset + 6)..segment_end.saturating_sub(2) {
            if data[i] == 0xFF && data[i + 1] == 0xD8 {
                let mut thumbnail = ThumbnailInfo {
                    soi_offset: i as u64,
                    eoi_offset: None,
                };
                for j in (i + 2)..segment_end.saturating_sub(1) {
                    if data[j] == 0xFF && data[j + 1] == 0xD9 {
                        thumbnail.eoi_offset = Some(j as u64);
                        break;
                    }
                }
                structure.thumbnail = Some(thumbnail);
                break;
            }
        }
    }
}

impl Default for JpegParser {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptionReason {
    InvalidMarkerSequence,
    HuffmanDecodeError,
    UnexpectedEof,
    InvalidSegmentLength,
    DcCoefficientDiscontinuity,
    MissingRequiredMarker(String),
    RestartSequenceError,
}

#[derive(Debug, Clone)]
pub enum ValidationResult {
    Valid(JpegStructure),
    CorruptedAt {
        offset: u64,
        reason: CorruptionReason,
        partial_structure: JpegStructure,
    },
    Truncated {
        last_valid_offset: u64,
        partial_structure: JpegStructure,
    },
    InvalidHeader,
}

impl ValidationResult {
    #[inline]
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }

    pub fn structure(&self) -> Option<&JpegStructure> {
        match self {
            Self::Valid(s)
            | Self::CorruptedAt {
                partial_structure: s,
                ..
            }
            | Self::Truncated {
                partial_structure: s,
                ..
            } => Some(s),
            Self::InvalidHeader => None,
        }
    }
}

pub struct JpegValidator {
    parser: JpegParser,
}

impl JpegValidator {
    #[inline]
    pub fn new() -> Self {
        Self {
            parser: JpegParser::new(),
        }
    }

    pub fn validate(&self, data: &[u8]) -> ValidationResult {
        let structure = match self.parser.parse(data) {
            Ok(s) => s,
            Err(_) => return ValidationResult::InvalidHeader,
        };

        let has_eoi = structure
            .markers
            .iter()
            .any(|m| matches!(m.marker_type, MarkerType::Eoi));
        if !has_eoi {
            return ValidationResult::Truncated {
                last_valid_offset: structure.valid_end_offset,
                partial_structure: structure,
            };
        }
        if let Some(offset) = structure.corruption_point {
            return ValidationResult::CorruptedAt {
                offset,
                reason: CorruptionReason::InvalidSegmentLength,
                partial_structure: structure,
            };
        }

        if let Some((offset, reason)) = self.validate_marker_sequence(&structure) {
            return ValidationResult::CorruptedAt {
                offset,
                reason,
                partial_structure: structure,
            };
        }

        if structure.restart_interval > 0 {
            if let Some((offset, reason)) = self.validate_restart_sequence(&structure) {
                return ValidationResult::CorruptedAt {
                    offset,
                    reason,
                    partial_structure: structure,
                };
            }
        }
        ValidationResult::Valid(structure)
    }

    #[inline]
    pub fn parse_structure(&self, data: &[u8]) -> Result<JpegStructure> {
        self.parser.parse(data)
    }

    fn validate_marker_sequence(
        &self,
        structure: &JpegStructure,
    ) -> Option<(u64, CorruptionReason)> {
        let (mut seen_soi, mut seen_dqt, mut seen_sof, mut seen_sos) = (false, false, false, false);
        for marker in &structure.markers {
            match marker.marker_type {
                MarkerType::Soi => {
                    if seen_soi && seen_sos {
                        return Some((marker.offset, CorruptionReason::InvalidMarkerSequence));
                    }
                    seen_soi = true;
                }
                MarkerType::Dqt => seen_dqt = true,
                MarkerType::Sof(_) => {
                    if !seen_dqt {
                        return Some((
                            marker.offset,
                            CorruptionReason::MissingRequiredMarker("DQT before SOF".into()),
                        ));
                    }
                    seen_sof = true;
                }
                MarkerType::Sos => {
                    if !seen_sof {
                        return Some((
                            marker.offset,
                            CorruptionReason::MissingRequiredMarker("SOF before SOS".into()),
                        ));
                    }
                    seen_sos = true;
                }
                _ => {}
            }
        }
        None
    }

    fn validate_restart_sequence(
        &self,
        structure: &JpegStructure,
    ) -> Option<(u64, CorruptionReason)> {
        let rst_markers: Vec<_> = structure
            .markers
            .iter()
            .filter(|m| matches!(m.marker_type, MarkerType::Rst(_)))
            .collect();
        if rst_markers.is_empty() {
            return None;
        }
        let mut expected = 0u8;
        for marker in rst_markers {
            if let MarkerType::Rst(n) = marker.marker_type {
                if n != expected {
                    return Some((marker.offset, CorruptionReason::RestartSequenceError));
                }
                expected = (expected + 1) % 8;
            }
        }
        None
    }
}

impl Default for JpegValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct HuffmanTable {
    max_bits: u8,
    lookup: HashMap<u32, (u8, u8)>,
}

impl HuffmanTable {
    pub fn from_dht_data(data: &[u8]) -> Option<Self> {
        if data.len() < 16 {
            return None;
        }
        let counts = &data[0..16];
        let total_symbols: usize = counts.iter().map(|&c| c as usize).sum();
        if data.len() < 16 + total_symbols {
            return None;
        }
        let symbols = &data[16..16 + total_symbols];
        let mut lookup = HashMap::new();
        let mut code: u32 = 0;
        let mut symbol_idx = 0;
        let mut max_bits = 0u8;

        for bits in 1..=16u8 {
            let count = counts[bits as usize - 1] as usize;
            for _ in 0..count {
                if symbol_idx >= symbols.len() {
                    return None;
                }
                lookup.insert(code, (symbols[symbol_idx], bits));
                symbol_idx += 1;
                code += 1;
                max_bits = bits;
            }
            code <<= 1;
        }
        Some(Self { max_bits, lookup })
    }

    pub fn decode(&self, bits: u32, available_bits: u8) -> Option<(u8, u8)> {
        let max_check = self.max_bits.min(available_bits);
        for len in 1..=max_check {
            let mask = (1u32 << len) - 1;
            let code = (bits >> (32 - len)) & mask;
            if let Some(&(symbol, code_len)) = self.lookup.get(&code) {
                if code_len == len {
                    return Some((symbol, len));
                }
            }
        }
        None
    }
}

pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buffer: u32,
    bits_available: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_buffer: 0,
            bits_available: 0,
        }
    }

    fn read_byte(&mut self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        }
        let byte = self.data[self.pos];
        self.pos += 1;

        if byte == 0xFF && self.pos < self.data.len() {
            if self.data[self.pos] == 0x00 {
                self.pos += 1;
            } else {
                self.pos -= 1;
                return None;
            }
        }
        Some(byte)
    }

    fn fill_buffer(&mut self, needed: u8) {
        while self.bits_available < needed {
            if let Some(byte) = self.read_byte() {
                self.bit_buffer = (self.bit_buffer << 8) | (byte as u32);
                self.bits_available += 8;
            } else {
                break;
            }
        }
    }

    pub fn peek_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 || n > 24 {
            return None;
        }
        self.fill_buffer(n);
        if self.bits_available < n {
            return None;
        }
        let shift = self.bits_available - n;
        Some((self.bit_buffer >> shift) & ((1 << n) - 1))
    }

    pub fn read_bits(&mut self, n: u8) -> Option<u32> {
        let bits = self.peek_bits(n)?;
        self.bits_available -= n;
        Some(bits)
    }

    pub fn decode_huffman(&mut self, table: &HuffmanTable) -> Option<u8> {
        self.fill_buffer(16);

        for len in 1..=16u8 {
            if len > self.bits_available {
                break;
            }
            let shift = self.bits_available - len;
            let code = (self.bit_buffer >> shift) & ((1 << len) - 1);

            if let Some(&(symbol, code_len)) = table.lookup.get(&code) {
                if code_len == len {
                    self.bits_available -= len;
                    return Some(symbol);
                }
            }
        }
        None
    }

    pub fn decode_value(&mut self, size: u8) -> Option<i16> {
        if size == 0 {
            return Some(0);
        }
        if size > 15 {
            return None;
        }

        let bits = self.read_bits(size)?;

        let half = 1u32 << (size - 1);
        if bits < half {
            Some((bits as i16) - ((1 << size) - 1) as i16)
        } else {
            Some(bits as i16)
        }
    }

    pub fn position(&self) -> usize {
        self.pos
    }
    pub fn is_at_end(&self) -> bool {
        self.pos >= self.data.len()
            || (self.pos + 1 < self.data.len()
                && self.data[self.pos] == 0xFF
                && self.data[self.pos + 1] != 0x00)
    }
}

#[derive(Debug, Clone)]
pub struct HuffmanDecoder {
    dc_tables: [Option<HuffmanTable>; 4],
    ac_tables: [Option<HuffmanTable>; 4],
    dc_pred: [i16; 4],
}

impl HuffmanDecoder {
    pub fn new() -> Self {
        Self {
            dc_tables: [None, None, None, None],
            ac_tables: [None, None, None, None],
            dc_pred: [0; 4],
        }
    }

    pub fn load_table(&mut self, table_class: u8, table_id: u8, data: &[u8]) -> bool {
        let table = match HuffmanTable::from_dht_data(data) {
            Some(t) => t,
            None => return false,
        };
        let id = (table_id & 0x03) as usize;
        if table_class == 0 {
            self.dc_tables[id] = Some(table);
        } else {
            self.ac_tables[id] = Some(table);
        }
        true
    }

    pub fn reset_dc_predictors(&mut self) {
        self.dc_pred = [0; 4];
    }

    pub fn dc_continuity_score(head_dc: &[i16], tail_dc: &[i16]) -> f32 {
        if head_dc.is_empty() || tail_dc.is_empty() {
            return 0.0;
        }
        let compare_count = 8.min(head_dc.len()).min(tail_dc.len());
        let head_end = &head_dc[head_dc.len() - compare_count..];
        let tail_start = &tail_dc[..compare_count];
        let total_diff: i32 = head_end
            .iter()
            .zip(tail_start.iter())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .sum();
        let avg_diff = total_diff as f32 / compare_count as f32;
        1.0 - (avg_diff / 100.0).min(1.0)
    }

    pub fn extract_dc_coefficients(
        &mut self,
        entropy_data: &[u8],
        components: u8,
        max_mcus: usize,
    ) -> Vec<i16> {
        let mut dc_values = Vec::with_capacity(max_mcus);
        let mut reader = BitReader::new(entropy_data);

        let dc_table = match &self.dc_tables[0] {
            Some(t) => t,
            None => return dc_values,
        };

        self.dc_pred = [0; 4];

        for _ in 0..max_mcus {
            if reader.is_at_end() {
                break;
            }

            for comp_idx in 0..(components as usize).min(4) {
                let dc_size = match reader.decode_huffman(dc_table) {
                    Some(s) => s,
                    None => break,
                };

                let dc_diff = match reader.decode_value(dc_size) {
                    Some(v) => v,
                    None => break,
                };

                self.dc_pred[comp_idx] = self.dc_pred[comp_idx].wrapping_add(dc_diff);

                if comp_idx == 0 {
                    dc_values.push(self.dc_pred[0]);
                }

                if let Some(ac_table) = &self.ac_tables[0] {
                    let mut ac_count = 0;
                    while ac_count < 63 {
                        if let Some(symbol) = reader.decode_huffman(ac_table) {
                            let run = symbol >> 4;
                            let size = symbol & 0x0F;

                            if size == 0 {
                                if run == 0 {
                                    break;
                                } else if run == 0x0F {
                                    ac_count += 16;
                                }
                            } else {
                                ac_count += run as usize + 1;
                                let _ = reader.read_bits(size);
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        dc_values
    }

    pub fn validate_stitch(&mut self, head_data: &[u8], tail_data: &[u8], components: u8) -> f32 {
        let head_dc = self.extract_dc_coefficients(head_data, components, 64);

        self.reset_dc_predictors();
        let tail_dc = self.extract_dc_coefficients(tail_data, components, 64);

        Self::dc_continuity_score(&head_dc, &tail_dc)
    }
}

impl Default for HuffmanDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartMarkerInfo {
    pub offset: usize,
    pub rst_number: u8,
}

#[derive(Debug, Clone, Default)]
pub struct RestartMarkerScanner;

impl RestartMarkerScanner {
    pub fn new() -> Self {
        Self
    }

    pub fn scan(&self, buffer: &[u8]) -> Vec<RestartMarkerInfo> {
        let mut markers = Vec::new();
        let mut i = 0;
        while i < buffer.len().saturating_sub(1) {
            if buffer[i] == 0xFF {
                let marker = buffer[i + 1];
                if marker == 0x00 {
                    i += 2;
                    continue;
                }
                if is_restart_marker(marker) {
                    if let Some(num) = restart_marker_index(marker) {
                        markers.push(RestartMarkerInfo {
                            offset: i,
                            rst_number: num,
                        });
                    }
                    i += 2;
                    continue;
                }
                if marker == 0xD9 {
                    break;
                }
            }
            i += 1;
        }
        markers
    }

    pub fn validate_sequence(&self, markers: &[RestartMarkerInfo]) -> bool {
        if markers.is_empty() {
            return true;
        }
        let mut expected = markers[0].rst_number;
        for (i, marker) in markers.iter().enumerate() {
            if i == 0 {
                continue;
            }
            expected = (expected + 1) % 8;
            if marker.rst_number != expected {
                return false;
            }
        }
        true
    }

    pub fn junction_score(
        &self,
        head_markers: &[RestartMarkerInfo],
        tail_markers: &[RestartMarkerInfo],
    ) -> f32 {
        let (Some(last_head), Some(first_tail)) = (head_markers.last(), tail_markers.first())
        else {
            return 0.5;
        };

        let expected_next = (last_head.rst_number + 1) % 8;
        if first_tail.rst_number == expected_next {
            1.0
        } else {
            let diff = (first_tail.rst_number as i8 - expected_next as i8).unsigned_abs();
            let min_diff = diff.min(8 - diff);
            1.0 - (min_diff as f32 / 4.0)
        }
    }
}

#[derive(Debug, Clone)]
pub struct JpegBoundaryDetector {
    pub use_eoi_hints: bool,
    pub validate_structure: bool,
    pub entropy_window: usize,
    pub entropy_threshold: f64,
    pub max_file_size: u64,
}

impl Default for JpegBoundaryDetector {
    fn default() -> Self {
        Self {
            use_eoi_hints: true,
            validate_structure: true,
            entropy_window: 8192,
            entropy_threshold: 2.5,
            max_file_size: 100 * 1024 * 1024, // 100MB
        }
    }
}

impl JpegBoundaryDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find_boundary(&self, data: &[u8]) -> Option<usize> {
        if data.len() < 4 {
            return None;
        }

        let parser = JpegParser::new();
        if let Ok(structure) = parser.parse(data) {
            if structure.valid_end_offset > 0 && !structure.is_truncated {
                return Some(structure.valid_end_offset as usize);
            }
        }

        let eoi_candidates = if self.use_eoi_hints {
            self.find_all_eoi_markers(data)
        } else {
            vec![]
        };

        let entropy_boundary = self.detect_entropy_drop(data);

        if let Some(entropy_bound) = entropy_boundary {
            if let Some(best_eoi) = self.select_best_eoi(&eoi_candidates, entropy_bound) {
                if self.validate_structure {
                    if self.validate_jpeg_at_offset(data, best_eoi) {
                        return Some(best_eoi);
                    }
                } else {
                    return Some(best_eoi);
                }
            }
            return Some(entropy_bound);
        }

        for &eoi in &eoi_candidates {
            if self.validate_structure {
                if self.validate_jpeg_at_offset(data, eoi) {
                    return Some(eoi);
                }
            } else {
                return Some(eoi);
            }
        }

        None
    }

    fn find_all_eoi_markers(&self, data: &[u8]) -> Vec<usize> {
        let mut candidates = Vec::new();
        let max_search = data.len().min(self.max_file_size as usize);

        for i in 0..max_search.saturating_sub(1) {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                candidates.push(i + 2); 
            }
        }
        candidates
    }

    fn detect_entropy_drop(&self, data: &[u8]) -> Option<usize> {
        if data.len() < self.entropy_window * 2 {
            return None;
        }

        let max_search = data.len().min(self.max_file_size as usize);
        let mut prev_entropy = Self::compute_entropy_for_window(&data[..self.entropy_window]);
        let mut offset = self.entropy_window;

        while offset + self.entropy_window <= max_search {
            let current_entropy =
                Self::compute_entropy_for_window(&data[offset..offset + self.entropy_window]);

            let delta = prev_entropy - current_entropy;

            if delta > self.entropy_threshold && prev_entropy > 6.0 {
                return Some(offset);
            }

            prev_entropy = current_entropy;
            offset += self.entropy_window;
        }

        None
    }

    fn select_best_eoi(&self, eois: &[usize], entropy_boundary: usize) -> Option<usize> {
        eois.iter()
            .filter(|&&eoi| eoi <= entropy_boundary + self.entropy_window)
            .max_by_key(|&&eoi| eoi)
            .copied()
    }

    fn validate_jpeg_at_offset(&self, data: &[u8], end_offset: usize) -> bool {
        if end_offset > data.len() || end_offset < 4 {
            return false;
        }

        let parser = JpegParser::new();
        match parser.parse(&data[..end_offset]) {
            Ok(structure) => {
                !structure.markers.is_empty() &&
                !structure.is_truncated &&
                structure.has_valid_content
            }
            Err(_) => false,
        }
    }

    fn compute_entropy_for_window(data: &[u8]) -> f64 {
        if data.is_empty() {
            return 0.0;
        }

        let mut counts = [0u64; 256];
        for &byte in data {
            counts[byte as usize] += 1;
        }

        let len = data.len() as f64;
        let mut entropy = 0.0;

        for &count in &counts {
            if count > 0 {
                let p = count as f64 / len;
                entropy -= p * p.log2();
            }
        }

        entropy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_jpeg() -> Vec<u8> {
        vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00,
        ]
        .into_iter()
        .chain(vec![0x10; 64])
        .chain(vec![
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0x01, 0x11, 0x00, 0xFF,
            0xC4, 0x00, 0x1F, 0x00,
        ])
        .chain(vec![0x00; 28])
        .chain(vec![
            0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
        ])
        .chain(vec![0x00; 10])
        .chain(vec![0xFF, 0xD9])
        .collect()
    }

    #[test]
    fn test_validate_valid_jpeg() {
        let result = JpegValidator::new().validate(&create_valid_jpeg());
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_truncated() {
        let mut data = create_valid_jpeg();
        data.truncate(data.len() - 10);
        assert!(matches!(
            JpegValidator::new().validate(&data),
            ValidationResult::Truncated { .. }
        ));
    }

    #[test]
    fn test_validate_invalid_header() {
        assert!(matches!(
            JpegValidator::new().validate(&[0x00; 4]),
            ValidationResult::InvalidHeader
        ));
    }

    #[test]
    fn test_marker_type_roundtrip() {
        for byte in 0u8..=255 {
            assert_eq!(MarkerType::from_byte(byte).to_byte(), byte);
        }
    }

    #[test]
    fn test_restart_marker_detection() {
        assert!(is_restart_marker(RST0));
        assert!(is_restart_marker(RST7));
        assert!(!is_restart_marker(0xD8));
    }

    #[test]
    fn test_dc_continuity_score() {
        let head = vec![100, 101, 102, 103, 104];
        let tail = vec![104, 105, 106, 107, 108];
        let score = HuffmanDecoder::dc_continuity_score(&head, &tail);
        assert!(score > 0.9);
    }

    #[test]
    fn test_restart_scanner() {
        let scanner = RestartMarkerScanner::new();
        let data = vec![
            0xFF, 0xD0, 0x00, 0x00, 0xFF, 0xD1, 0x00, 0x00, 0xFF, 0xD2, 0x00, 0x00,
        ];
        let markers = scanner.scan(&data);
        assert_eq!(markers.len(), 3);
        assert!(scanner.validate_sequence(&markers));
    }

    #[test]
    fn test_quick_validate_header_valid() {
        let valid = [0xFF, 0xD8, 0xFF, 0xE0];
        assert!(quick_validate_header(&valid));

        let exif = [0xFF, 0xD8, 0xFF, 0xE1];
        assert!(quick_validate_header(&exif));

        let dqt = [0xFF, 0xD8, 0xFF, 0xDB];
        assert!(quick_validate_header(&dqt));
    }

    #[test]
    fn test_quick_validate_header_invalid() {
        assert!(!quick_validate_header(&[0x00, 0x00, 0x00, 0x00]));
        assert!(!quick_validate_header(&[0xFF, 0xD8, 0xFF, 0x00]));
        assert!(!quick_validate_header(&[0xFF, 0xD8, 0x00, 0xE0]));
        assert!(!quick_validate_header(&[0xFF, 0xD8]));
    }
}
