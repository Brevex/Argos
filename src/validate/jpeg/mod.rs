use crate::error::{ArgosError, ValidationKind};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const DHT: u8 = 0xC4;
const SOF0: u8 = 0xC0;
const SOS: u8 = 0xDA;

#[derive(Debug, Clone)]
pub struct Segment {
    pub marker: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct HuffmanTable {
    pub class: u8,
    pub id: u8,
    pub bits: [u8; 16],
    pub values: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct DecoderState {
    pub dc_tables: [Option<HuffmanTable>; 4],
    pub ac_tables: [Option<HuffmanTable>; 4],
    pub frame_width: u16,
    pub frame_height: u16,
    pub components: u8,
}

pub fn validate(data: &[u8]) -> Result<f32, ArgosError> {
    let segments = match parse_segments(data) {
        Ok(s) => s,
        Err(ArgosError::Validation { .. }) => return Ok(0.0),
        Err(e) => return Err(e),
    };
    let mut state = DecoderState::default();

    for seg in &segments {
        match seg.marker {
            DHT => {
                let table = parse_huffman_table(&seg.data)?;
                let slot = table.id & 0x03;
                if table.class == 0 {
                    state.dc_tables[slot as usize] = Some(table);
                } else {
                    state.ac_tables[slot as usize] = Some(table);
                }
            }
            SOF0 => {
                if seg.data.len() < 6 {
                    return Err(ArgosError::Validation {
                        kind: ValidationKind::TruncatedSegment,
                    });
                }
                state.frame_height = u16::from_be_bytes([seg.data[1], seg.data[2]]);
                state.frame_width = u16::from_be_bytes([seg.data[3], seg.data[4]]);
                state.components = seg.data[5];
            }
            _ => {}
        }
    }

    let sos_pos = segments.iter().position(|s| s.marker == SOS);
    if sos_pos.is_none() {
        return Ok(0.0);
    }

    let has_all_tables = state.dc_tables.iter().any(|t| t.is_some())
        && state.ac_tables.iter().any(|t| t.is_some());
    if !has_all_tables {
        return Ok(0.0);
    }

    let entropy_start = find_entropy_start(data);
    let entropy_end = find_entropy_end(data);
    if entropy_start >= entropy_end {
        return Ok(0.0);
    }

    let mcus = decode_entropy(&data[entropy_start..entropy_end], &state)?;
    let expected_mcus = estimate_mcus(&state);
    let score = if expected_mcus == 0 {
        0.0
    } else {
        (mcus as f32 / expected_mcus as f32).min(1.0)
    };

    Ok(score)
}

fn parse_segments(data: &[u8]) -> Result<Vec<Segment>, ArgosError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != SOI {
        return Err(ArgosError::Validation {
            kind: ValidationKind::MissingSoi,
        });
    }

    let mut segments = Vec::new();
    let mut i = 2;

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
        if marker == SOS {
            segments.push(Segment {
                marker,
                data: Vec::new(),
            });
            break;
        }

        let len = if i + 3 < data.len() {
            u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize
        } else {
            return Err(ArgosError::Validation {
                kind: ValidationKind::TruncatedSegment,
            });
        };

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
    }

    if !data.windows(2).any(|w| w == [0xFF, EOI]) {
        return Err(ArgosError::Validation {
            kind: ValidationKind::MissingEoi,
        });
    }

    Ok(segments)
}

fn parse_huffman_table(data: &[u8]) -> Result<HuffmanTable, ArgosError> {
    if data.is_empty() {
        return Err(ArgosError::Validation {
            kind: ValidationKind::BadHuffmanTable,
        });
    }
    let class = (data[0] >> 4) & 0x0F;
    let id = data[0] & 0x0F;
    if data.len() < 17 {
        return Err(ArgosError::Validation {
            kind: ValidationKind::BadHuffmanTable,
        });
    }
    let mut bits = [0u8; 16];
    bits.copy_from_slice(&data[1..17]);
    let total_values: usize = bits.iter().map(|&b| b as usize).sum();
    if data.len() < 17 + total_values {
        return Err(ArgosError::Validation {
            kind: ValidationKind::BadHuffmanTable,
        });
    }
    let values = data[17..17 + total_values].to_vec();
    Ok(HuffmanTable {
        class,
        id,
        bits,
        values,
    })
}

fn find_entropy_start(data: &[u8]) -> usize {
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0xFF && data[i + 1] == SOS {
            if i + 3 < data.len() {
                let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
                return (i + 2 + len).min(data.len());
            }
            return data.len();
        }
    }
    data.len()
}

fn find_entropy_end(data: &[u8]) -> usize {
    for i in (2..data.len().saturating_sub(1)).rev() {
        if data[i] == 0xFF && data[i + 1] == EOI {
            return i;
        }
    }
    data.len()
}

fn decode_entropy(data: &[u8], state: &DecoderState) -> Result<usize, ArgosError> {
    let mut pos = 0;
    let mut mcus = 0;
    let max_mcus = estimate_mcus(state);

    while pos < data.len() && mcus < max_mcus * 2 {
        let mut bits = BitStream::new(data, pos);
        let mut eob = false;

        for _comp in 0..state.components.max(1) {
            let dc_slot = 0;
            let ac_slot = 0;

            if let Some(ref table) = state.dc_tables[dc_slot] {
                match decode_huffman_value(&mut bits, table) {
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            for _ in 0..63 {
                if eob {
                    break;
                }
                if let Some(ref table) = state.ac_tables[ac_slot] {
                    match decode_huffman_value(&mut bits, table) {
                        Ok(val) => {
                            if val == 0x00 {
                                eob = true;
                            }
                        }
                        Err(_) => {
                            eob = true;
                            break;
                        }
                    }
                }
            }
        }

        pos = bits.pos();
        if pos >= data.len() {
            break;
        }
        mcus += 1;

        if pos + 1 < data.len() && data[pos] == 0xFF && data[pos + 1] == EOI {
            break;
        }
    }

    Ok(mcus)
}

fn estimate_mcus(state: &DecoderState) -> usize {
    if state.frame_width == 0 || state.frame_height == 0 {
        return 1;
    }
    let w = (state.frame_width as usize + 7) / 8;
    let h = (state.frame_height as usize + 7) / 8;
    w * h
}

struct BitStream<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> BitStream<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos, bit: 0 }
    }

    fn pos(&self) -> usize {
        self.pos
    }

    fn next_bit(&mut self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        }
        let byte = self.data[self.pos];
        let bit = (byte >> (7 - self.bit)) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Some(bit)
    }
}

fn decode_huffman_value(bits: &mut BitStream, table: &HuffmanTable) -> Result<u8, ArgosError> {
    let mut code: u16 = 0;
    for (len_minus_one, &count) in table.bits.iter().enumerate() {
        if let Some(bit) = bits.next_bit() {
            code = (code << 1) | bit as u16;
        } else {
            return Err(ArgosError::Validation {
                kind: ValidationKind::BadHuffmanCode,
            });
        }
        if count > 0 {
            let max_code = (1u16 << (len_minus_one + 1)) - 1;
            if code <= max_code {
                let index = table.values.get(code as usize).copied().ok_or(
                    ArgosError::Validation {
                        kind: ValidationKind::BadHuffmanCode,
                    },
                )?;
                return Ok(index);
            }
        }
    }
    Err(ArgosError::Validation {
        kind: ValidationKind::BadHuffmanCode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_jpeg() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&[0xFF, SOI]);

        let mut dht = Vec::new();
        dht.push(0x00);
        dht.extend_from_slice(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        dht.push(0x00);
        let dht_len = (dht.len() + 2) as u16;
        data.push(0xFF);
        data.push(DHT);
        data.extend_from_slice(&dht_len.to_be_bytes());
        data.extend_from_slice(&dht);

        let mut dht_ac = Vec::new();
        dht_ac.push(0x10);
        dht_ac.extend_from_slice(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        dht_ac.push(0x00);
        let dht_ac_len = (dht_ac.len() + 2) as u16;
        data.push(0xFF);
        data.push(DHT);
        data.extend_from_slice(&dht_ac_len.to_be_bytes());
        data.extend_from_slice(&dht_ac);

        let mut sof = Vec::new();
        sof.push(0x08);
        sof.extend_from_slice(&8u16.to_be_bytes());
        sof.extend_from_slice(&8u16.to_be_bytes());
        sof.push(0x01);
        sof.extend_from_slice(&[0x01, 0x11, 0x00]);
        let sof_len = (sof.len() + 2) as u16;
        data.push(0xFF);
        data.push(SOF0);
        data.extend_from_slice(&sof_len.to_be_bytes());
        data.extend_from_slice(&sof);

        let mut sos = Vec::new();
        sos.push(0x01);
        sos.extend_from_slice(&[0x01, 0x00]);
        sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
        let sos_len = (sos.len() + 2) as u16;
        data.push(0xFF);
        data.push(SOS);
        data.extend_from_slice(&sos_len.to_be_bytes());
        data.extend_from_slice(&sos);

        data.push(0x00);
        data.push(0x00);

        data.push(0xFF);
        data.push(EOI);
        data
    }

    #[test]
    fn validate_rejects_garbage() {
        let score = validate(&[0u8; 100]).unwrap();
        assert_eq!(score, 0.0);
    }

    #[test]
    fn validate_accepts_minimal() {
        let data = minimal_jpeg();
        let score = validate(&data).unwrap();
        assert!(score > 0.0);
    }

    #[test]
    fn parse_segments_finds_so_i() {
        let data = minimal_jpeg();
        let segs = parse_segments(&data).unwrap();
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].marker, DHT);
        assert_eq!(segs[1].marker, DHT);
        assert_eq!(segs[2].marker, SOF0);
        assert_eq!(segs[3].marker, SOS);
    }

    #[test]
    fn parse_huffman_table_ok() {
        let raw = [
            0x00,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xAB,
        ];
        let table = parse_huffman_table(&raw).unwrap();
        assert_eq!(table.class, 0);
        assert_eq!(table.id, 0);
        assert_eq!(table.values, vec![0xAB]);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn random_data_never_crashes(data: Vec<u8>) {
            let _ = validate(&data);
        }

        #[test]
        fn prefix_with_soi_eoi_never_crashes(data: Vec<u8>) {
            let mut buf = Vec::with_capacity(data.len() + 4);
            buf.extend_from_slice(&[0xFF, SOI]);
            buf.extend_from_slice(&data);
            buf.extend_from_slice(&[0xFF, EOI]);
            let _ = validate(&buf);
        }
    }
}
