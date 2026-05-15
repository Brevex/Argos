use crate::carve::{Candidate, ImageFormat};

#[derive(Debug, Clone)]
pub struct Artifact {
    pub offset: u64,
    pub length: u64,
    pub format: ImageFormat,
}

pub fn reassemble_ssd(candidates: Vec<Candidate>) -> Vec<Artifact> {
    let mut artifacts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if let Some(length) = candidate.length {
            artifacts.push(Artifact {
                offset: candidate.offset,
                length,
                format: candidate.format,
            });
        }
    }
    artifacts
}
