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
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;

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

    if computed_crc == stored_crc { 1.0 } else { 0.0 }
}
