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

pub fn reassemble_hdd(_candidates: Vec<Candidate>) -> Vec<Artifact> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssd_assembles_linear() {
        let candidates = vec![
            Candidate {
                offset: 0,
                length: Some(100),
                format: ImageFormat::Jpeg,
            },
            Candidate {
                offset: 200,
                length: Some(50),
                format: ImageFormat::Png,
            },
        ];
        let artifacts = reassemble_ssd(candidates);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].offset, 0);
        assert_eq!(artifacts[0].length, 100);
        assert_eq!(artifacts[1].offset, 200);
        assert_eq!(artifacts[1].length, 50);
    }

    #[test]
    fn ssd_skips_open_candidates() {
        let candidates = vec![
            Candidate {
                offset: 0,
                length: None,
                format: ImageFormat::Jpeg,
            },
            Candidate {
                offset: 100,
                length: Some(50),
                format: ImageFormat::Jpeg,
            },
        ];
        let artifacts = reassemble_ssd(candidates);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].offset, 100);
    }

    #[test]
    fn hdd_placeholder_returns_empty() {
        let candidates = vec![];
        let artifacts = reassemble_hdd(candidates);
        assert!(artifacts.is_empty());
    }
}
