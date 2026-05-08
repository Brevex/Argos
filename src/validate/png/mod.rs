use crc32fast::Hasher;

use crate::error::{ArgosError, ValidationKind};

const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug, Clone)]
pub struct Chunk {
    pub chunk_type: [u8; 4],
    pub data: Vec<u8>,
    pub crc: u32,
}

pub fn validate(data: &[u8]) -> Result<f32, ArgosError> {
    if data.len() < SIGNATURE.len() || data[..SIGNATURE.len()] != SIGNATURE {
        return Ok(0.0);
    }

    let chunks = match parse_chunks(data) {
        Ok(c) => c,
        Err(ArgosError::Validation { .. }) => return Ok(0.0),
        Err(e) => return Err(e),
    };

    if chunks.is_empty() {
        return Ok(0.0);
    }

    if !is_ihdr(&chunks[0].chunk_type) {
        return Ok(0.0);
    }

    if !is_iend(&chunks[chunks.len() - 1].chunk_type) {
        return Ok(0.0);
    }

    let mut valid = 0usize;
    for chunk in &chunks {
        if verify_crc(chunk) {
            valid += 1;
        }
    }

    let score = if chunks.is_empty() {
        0.0
    } else {
        (valid as f32 / chunks.len() as f32).min(1.0)
    };

    Ok(score)
}

pub fn parse_chunks(data: &[u8]) -> Result<Vec<Chunk>, ArgosError> {
    if data.len() < SIGNATURE.len() + 12 {
        return Err(ArgosError::Validation {
            kind: ValidationKind::TruncatedChunk,
        });
    }

    let mut chunks = Vec::new();
    let mut pos = SIGNATURE.len();

    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;

        if pos + 12 + len > data.len() {
            break;
        }

        let chunk_type = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let chunk_data = data[pos + 8..pos + 8 + len].to_vec();
        let crc = u32::from_be_bytes([
            data[pos + 8 + len],
            data[pos + 9 + len],
            data[pos + 10 + len],
            data[pos + 11 + len],
        ]);

        chunks.push(Chunk {
            chunk_type,
            data: chunk_data,
            crc,
        });

        if is_iend(&chunk_type) {
            break;
        }

        pos += 12 + len;
    }

    if chunks.is_empty() {
        return Err(ArgosError::Validation {
            kind: ValidationKind::TruncatedChunk,
        });
    }

    if !is_ihdr(&chunks[0].chunk_type) {
        return Err(ArgosError::Validation {
            kind: ValidationKind::MissingIhdr,
        });
    }

    if !is_iend(&chunks[chunks.len() - 1].chunk_type) {
        return Err(ArgosError::Validation {
            kind: ValidationKind::MissingIend,
        });
    }

    Ok(chunks)
}

fn verify_crc(chunk: &Chunk) -> bool {
    let mut hasher = Hasher::new();
    hasher.update(&chunk.chunk_type);
    hasher.update(&chunk.data);
    hasher.finalize() == chunk.crc
}

fn is_ihdr(t: &[u8; 4]) -> bool {
    t == b"IHDR"
}

fn is_iend(t: &[u8; 4]) -> bool {
    t == b"IEND"
}

#[derive(Debug, Clone, Default)]
pub struct PartialChunk {
    pub pending: Vec<u8>,
    pub chunk_type: [u8; 4],
    pub expected_len: u32,
}

pub fn continuation_score(partial: &mut PartialChunk, block: &[u8]) -> f32 {
    partial.pending.extend_from_slice(block);

    if partial.pending.len() < 8 {
        return 0.5;
    }

    if partial.expected_len == 0 && partial.pending.len() >= 8 {
        partial.expected_len = u32::from_be_bytes([
            partial.pending[0],
            partial.pending[1],
            partial.pending[2],
            partial.pending[3],
        ]);
        partial.chunk_type = [
            partial.pending[4],
            partial.pending[5],
            partial.pending[6],
            partial.pending[7],
        ];
    }

    let total_needed = 12 + partial.expected_len as usize;
    if partial.pending.len() < total_needed {
        return 0.5 + 0.5 * (partial.pending.len() as f32 / total_needed as f32);
    }

    let data = &partial.pending[8..8 + partial.expected_len as usize];
    let stored_crc = u32::from_be_bytes([
        partial.pending[8 + partial.expected_len as usize],
        partial.pending[9 + partial.expected_len as usize],
        partial.pending[10 + partial.expected_len as usize],
        partial.pending[11 + partial.expected_len as usize],
    ]);

    let mut hasher = Hasher::new();
    hasher.update(&partial.chunk_type);
    hasher.update(data);
    let computed_crc = hasher.finalize();

    if computed_crc == stored_crc {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
fn make_crc(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(chunk_type);
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
fn make_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let crc = make_crc(chunk_type, data);
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn valid_png() -> Vec<u8> {
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&SIGNATURE);

        let ihdr = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00];
        data.extend_from_slice(&make_chunk(b"IHDR", &ihdr));

        let idat = [0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01];
        data.extend_from_slice(&make_chunk(b"IDAT", &idat));

        data.extend_from_slice(&make_chunk(b"IEND", &[]));
        data
    }

    #[test]
    fn validate_accepts_valid_png() {
        let score = validate(&valid_png()).unwrap();
        assert_eq!(score, 1.0);
    }

    #[test]
    fn validate_rejects_garbage() {
        let score = validate(&[0u8; 100]).unwrap();
        assert_eq!(score, 0.0);
    }

    #[test]
    fn validate_rejects_missing_iend() {
        let mut data = valid_png();
        data.truncate(data.len() - 12);
        let score = validate(&data).unwrap();
        assert_eq!(score, 0.0);
    }

    #[test]
    fn validate_rejects_bad_crc() {
        let mut data = valid_png();
        let crc_byte = data.len() - 2;
        data[crc_byte] ^= 0xFF;
        let score = validate(&data).unwrap();
        assert!(score < 1.0);
        assert!(score > 0.0);
    }

    #[test]
    fn parse_chunks_extracts_three() {
        let chunks = parse_chunks(&valid_png()).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(&chunks[0].chunk_type, b"IHDR");
        assert_eq!(&chunks[1].chunk_type, b"IDAT");
        assert_eq!(&chunks[2].chunk_type, b"IEND");
    }

    #[test]
    fn crc_matches_reference() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SIGNATURE);
        buf.extend_from_slice(&make_chunk(b"IHDR", &[0x00; 13]));
        buf.extend_from_slice(&make_chunk(b"IEND", &[]));
        let parsed = parse_chunks(&buf).unwrap();
        assert!(verify_crc(&parsed[0]));
    }

    proptest! {
        #[test]
        fn random_data_never_crashes(data: Vec<u8>) {
            let _ = validate(&data);
        }

        #[test]
        fn signature_plus_random_never_crashes(data: Vec<u8>) {
            let mut buf = Vec::with_capacity(8 + data.len());
            buf.extend_from_slice(&SIGNATURE);
            buf.extend_from_slice(&data);
            let _ = validate(&buf);
        }
    }
}
