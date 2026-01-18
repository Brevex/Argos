use std::collections::HashMap;

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

    pub fn parse_dht_segment(&mut self, data: &[u8]) -> bool {
        let mut pos = 0;

        while pos < data.len() {
            if pos + 17 > data.len() {
                break;
            }

            let tc_th = data[pos];
            let table_class = (tc_th >> 4) & 0x0F;
            let table_id = tc_th & 0x0F;

            let counts = &data[pos + 1..pos + 17];
            let total_symbols: usize = counts.iter().map(|&c| c as usize).sum();

            if pos + 17 + total_symbols > data.len() {
                return false;
            }

            if !self.load_table(
                table_class,
                table_id,
                &data[pos + 1..pos + 17 + total_symbols],
            ) {
                return false;
            }

            pos += 17 + total_symbols;
        }

        true
    }

    pub fn reset_dc_predictors(&mut self) {
        self.dc_pred = [0; 4];
    }

    pub fn get_dc_pred(&self, component: usize) -> i16 {
        self.dc_pred.get(component).copied().unwrap_or(0)
    }

    pub fn validate_and_extract_dc(
        &mut self,
        entropy_data: &[u8],
        num_components: usize,
        max_blocks: usize,
    ) -> Result<Vec<i16>, u64> {
        let mut dc_values = Vec::with_capacity(max_blocks * num_components);
        let mut bit_reader = BitReader::new(entropy_data);
        let mut blocks_decoded = 0;

        self.reset_dc_predictors();

        while blocks_decoded < max_blocks {
            for comp in 0..num_components {
                let dc_table = match &self.dc_tables[comp.min(3)] {
                    Some(t) => t,
                    None => return Err(bit_reader.byte_position() as u64),
                };

                let (category, bits_used) = match bit_reader.peek_bits(16) {
                    Some(bits) => match dc_table.decode(bits, 16) {
                        Some((sym, len)) => (sym, len),
                        None => return Err(bit_reader.byte_position() as u64),
                    },
                    None => break,
                };

                bit_reader.consume_bits(bits_used);

                let dc_diff = if category == 0 {
                    0i16
                } else {
                    match bit_reader.read_bits(category) {
                        Some(bits) => {
                            if bits < (1 << (category - 1)) {
                                bits as i16 - ((1 << category) - 1)
                            } else {
                                bits as i16
                            }
                        }
                        None => return Err(bit_reader.byte_position() as u64),
                    }
                };

                let dc_value = self.dc_pred[comp.min(3)] + dc_diff;
                self.dc_pred[comp.min(3)] = dc_value;
                dc_values.push(dc_value);
            }

            blocks_decoded += 1;

            if let Some((0xFF, next)) = bit_reader.peek_marker() {
                if next >= 0xD0 && next <= 0xD7 {
                    bit_reader.consume_marker();
                    self.reset_dc_predictors();
                } else if next == 0xD9 {
                    break;
                }
            }
        }

        Ok(dc_values)
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

        let normalized = (avg_diff / 100.0).min(1.0);

        1.0 - normalized
    }
}

impl Default for HuffmanDecoder {
    fn default() -> Self {
        Self::new()
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buffer: u32,
    bits_in_buffer: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_buffer: 0,
            bits_in_buffer: 0,
        }
    }

    fn byte_position(&self) -> usize {
        self.pos
    }

    fn fill_buffer(&mut self) {
        while self.bits_in_buffer <= 24 && self.pos < self.data.len() {
            let byte = self.data[self.pos];
            self.pos += 1;

            if byte == 0xFF && self.pos < self.data.len() {
                let next = self.data[self.pos];
                if next == 0x00 {
                    self.pos += 1;
                } else if next >= 0xD0 && next <= 0xD7 {
                    self.pos -= 1;
                    return;
                } else if next == 0xD9 {
                    self.pos -= 1;
                    return;
                } else if next == 0xFF {
                    continue;
                }
            }

            self.bit_buffer = (self.bit_buffer << 8) | (byte as u32);
            self.bits_in_buffer += 8;
        }
    }

    fn peek_bits(&mut self, count: u8) -> Option<u32> {
        self.fill_buffer();

        if self.bits_in_buffer < count {
            return None;
        }

        let shift = self.bits_in_buffer - count;
        Some((self.bit_buffer >> shift) & ((1 << count) - 1))
    }

    fn consume_bits(&mut self, count: u8) {
        if count <= self.bits_in_buffer {
            self.bits_in_buffer -= count;
            self.bit_buffer &= (1 << self.bits_in_buffer) - 1;
        }
    }

    fn read_bits(&mut self, count: u8) -> Option<u16> {
        let bits = self.peek_bits(count)?;
        self.consume_bits(count);
        Some(bits as u16)
    }

    fn peek_marker(&mut self) -> Option<(u8, u8)> {
        self.bits_in_buffer = 0;
        self.bit_buffer = 0;

        if self.pos + 1 < self.data.len() && self.data[self.pos] == 0xFF {
            Some((0xFF, self.data[self.pos + 1]))
        } else {
            None
        }
    }

    fn consume_marker(&mut self) {
        self.pos += 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_table_creation() {
        let data = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x01];

        let table = HuffmanTable::from_dht_data(&data).unwrap();

        let (sym, len) = table.decode(0x00000000, 8).unwrap();
        assert_eq!(sym, 0x00);
        assert_eq!(len, 1);
        let (sym, len) = table.decode(0x80000000, 8).unwrap();
        assert_eq!(sym, 0x01);
        assert_eq!(len, 1);
    }

    #[test]
    fn test_dc_continuity_score() {
        let head = vec![100, 101, 102, 103, 104];
        let tail = vec![104, 105, 106, 107, 108];
        let score = HuffmanDecoder::dc_continuity_score(&head, &tail);
        assert!(
            score > 0.9,
            "Expected high score for continuous values: {}",
            score
        );

        let head = vec![100, 101, 102, 103, 104];
        let tail = vec![-500, -501, -502, -503, -504];
        let score = HuffmanDecoder::dc_continuity_score(&head, &tail);
        assert!(
            score < 0.5,
            "Expected low score for discontinuous values: {}",
            score
        );

        assert_eq!(HuffmanDecoder::dc_continuity_score(&[], &[1, 2, 3]), 0.0);
        assert_eq!(HuffmanDecoder::dc_continuity_score(&[1, 2, 3], &[]), 0.0);
    }

    #[test]
    fn test_huffman_decoder_creation() {
        let decoder = HuffmanDecoder::new();
        assert!(decoder.dc_tables.iter().all(|t| t.is_none()));
        assert!(decoder.ac_tables.iter().all(|t| t.is_none()));
    }

    #[test]
    fn test_dc_predictor_reset() {
        let mut decoder = HuffmanDecoder::new();
        decoder.dc_pred = [100, 200, 300, 400];
        decoder.reset_dc_predictors();
        assert_eq!(decoder.dc_pred, [0, 0, 0, 0]);
    }
}
