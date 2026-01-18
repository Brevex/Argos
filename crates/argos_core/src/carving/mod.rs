mod smart;

pub use smart::{CarveDecision, SkipReason, SmartCarveResult, SmartCarver, SmartCarverConfig};

use crate::jpeg::{JpegValidator, ValidationResult};
use crate::traits::BlockSource;

#[derive(Debug, Clone)]
pub struct FragmentCandidate {
    pub offset: u64,
    pub size: u64,
    pub confidence: f32,
    pub dc_continuity: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct CarveResult {
    pub head_offset: u64,
    pub head_size: u64,
    pub tail_offset: Option<u64>,
    pub tail_size: Option<u64>,
    pub gap_size: Option<u64>,
    pub is_fragmented: bool,
    pub validation_score: f32,
}

impl CarveResult {
    pub fn contiguous(offset: u64, size: u64) -> Self {
        Self {
            head_offset: offset,
            head_size: size,
            tail_offset: None,
            tail_size: None,
            gap_size: None,
            is_fragmented: false,
            validation_score: 1.0,
        }
    }

    pub fn fragmented(
        head_offset: u64,
        head_size: u64,
        tail_offset: u64,
        tail_size: u64,
        validation_score: f32,
    ) -> Self {
        let gap = tail_offset.saturating_sub(head_offset + head_size);
        Self {
            head_offset,
            head_size,
            tail_offset: Some(tail_offset),
            tail_size: Some(tail_size),
            gap_size: Some(gap),
            is_fragmented: true,
            validation_score,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.head_size + self.tail_size.unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub struct StitchValidation {
    pub is_valid: bool,
    pub dc_continuity_score: f32,
    pub huffman_valid: bool,
    pub visual_discontinuity: bool,
    pub overall_score: f32,
}

impl StitchValidation {
    pub fn failed() -> Self {
        Self {
            is_valid: false,
            dc_continuity_score: 0.0,
            huffman_valid: false,
            visual_discontinuity: true,
            overall_score: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BgcConfig {
    pub cluster_size: u64,
    pub max_gap: u64,
    pub min_confidence: f32,
    pub search_clusters: usize,
}

impl Default for BgcConfig {
    fn default() -> Self {
        Self {
            cluster_size: 4096,
            max_gap: 16 * 1024 * 1024,
            min_confidence: 0.6,
            search_clusters: 4096,
        }
    }
}

pub struct BifragmentCarver {
    config: BgcConfig,
    validator: JpegValidator,
}

impl BifragmentCarver {
    pub fn new() -> Self {
        Self {
            config: BgcConfig::default(),
            validator: JpegValidator::new(),
        }
    }

    pub fn with_config(config: BgcConfig) -> Self {
        Self {
            config,
            validator: JpegValidator::new(),
        }
    }

    pub fn carve_bifragment<S: BlockSource>(
        &self,
        head: &[u8],
        head_offset: u64,
        source: &mut S,
    ) -> Option<CarveResult> {
        let validation = self.validator.validate(head);

        let corruption_offset = match &validation {
            ValidationResult::CorruptedAt { offset, .. } => *offset,
            ValidationResult::Truncated {
                last_valid_offset, ..
            } => *last_valid_offset,
            ValidationResult::Valid(_) => {
                return Some(CarveResult::contiguous(head_offset, head.len() as u64));
            }
            ValidationResult::InvalidHeader => return None,
        };

        let search_start = head_offset + head.len() as u64;
        let search_end = (search_start + self.config.max_gap).min(source.size());

        let first_cluster = (search_start + self.config.cluster_size - 1)
            / self.config.cluster_size
            * self.config.cluster_size;

        let mut best_candidate: Option<(FragmentCandidate, Vec<u8>)> = None;
        let mut buffer = vec![0u8; 64 * 1024];

        let mut cluster_offset = first_cluster;
        while cluster_offset < search_end {
            let read_size = buffer.len().min((search_end - cluster_offset) as usize);
            let bytes_read = source
                .read_chunk(cluster_offset, &mut buffer[..read_size])
                .ok()?;

            if bytes_read == 0 {
                break;
            }

            if let Some(candidate) = self.try_stitch(
                head,
                &buffer[..bytes_read],
                corruption_offset as usize,
                cluster_offset,
            ) {
                let dominated = best_candidate
                    .as_ref()
                    .map(|(c, _)| candidate.confidence > c.confidence)
                    .unwrap_or(true);

                if dominated && candidate.confidence >= self.config.min_confidence {
                    best_candidate = Some((candidate, buffer[..bytes_read].to_vec()));
                }
            }

            cluster_offset += self.config.cluster_size;
        }

        best_candidate.map(|(candidate, _tail_data)| {
            CarveResult::fragmented(
                head_offset,
                corruption_offset,
                candidate.offset,
                candidate.size,
                candidate.confidence,
            )
        })
    }

    pub fn validate_stitch(&self, head: &[u8], tail: &[u8]) -> StitchValidation {
        let mut combined = Vec::with_capacity(head.len() + tail.len());
        combined.extend_from_slice(head);
        combined.extend_from_slice(tail);

        let validation = self.validator.validate(&combined);

        match validation {
            ValidationResult::Valid(_) => StitchValidation {
                is_valid: true,
                dc_continuity_score: 1.0,
                huffman_valid: true,
                visual_discontinuity: false,
                overall_score: 1.0,
            },
            ValidationResult::CorruptedAt { .. } => StitchValidation::failed(),
            ValidationResult::Truncated { .. } => StitchValidation {
                is_valid: false,
                dc_continuity_score: 0.5,
                huffman_valid: true,
                visual_discontinuity: false,
                overall_score: 0.4,
            },
            ValidationResult::InvalidHeader => StitchValidation::failed(),
        }
    }

    fn try_stitch(
        &self,
        head: &[u8],
        tail: &[u8],
        stitch_point: usize,
        tail_offset: u64,
    ) -> Option<FragmentCandidate> {
        if tail.is_empty() {
            return None;
        }

        let eoi_pos = self.find_eoi(tail)?;
        let head_part = &head[..stitch_point.min(head.len())];
        let tail_part = &tail[..eoi_pos + 2];
        let stitch_result = self.validate_stitch(head_part, tail_part);

        if stitch_result.is_valid || stitch_result.overall_score > 0.5 {
            Some(FragmentCandidate {
                offset: tail_offset,
                size: (eoi_pos + 2) as u64,
                confidence: stitch_result.overall_score,
                dc_continuity: Some(stitch_result.dc_continuity_score),
            })
        } else {
            None
        }
    }

    fn find_eoi(&self, data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(1) {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                return Some(i);
            }
        }
        None
    }
}

impl Default for BifragmentCarver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_carve_result_contiguous() {
        let result = CarveResult::contiguous(1000, 5000);
        assert!(!result.is_fragmented);
        assert_eq!(result.head_offset, 1000);
        assert_eq!(result.head_size, 5000);
        assert!(result.tail_offset.is_none());
        assert_eq!(result.total_size(), 5000);
    }

    #[test]
    fn test_carve_result_fragmented() {
        let result = CarveResult::fragmented(1000, 2000, 5000, 3000, 0.9);
        assert!(result.is_fragmented);
        assert_eq!(result.head_offset, 1000);
        assert_eq!(result.head_size, 2000);
        assert_eq!(result.tail_offset, Some(5000));
        assert_eq!(result.tail_size, Some(3000));
        assert_eq!(result.gap_size, Some(2000));
        assert_eq!(result.total_size(), 5000);
    }

    #[test]
    fn test_stitch_validation_failed() {
        let failed = StitchValidation::failed();
        assert!(!failed.is_valid);
        assert_eq!(failed.overall_score, 0.0);
    }

    #[test]
    fn test_bgc_config_default() {
        let config = BgcConfig::default();
        assert_eq!(config.cluster_size, 4096);
        assert_eq!(config.max_gap, 16 * 1024 * 1024);
        assert!(config.min_confidence > 0.0);
    }

    #[test]
    fn test_carver_creation() {
        let carver = BifragmentCarver::new();
        assert_eq!(carver.config.cluster_size, 4096);

        let custom_config = BgcConfig {
            cluster_size: 8192,
            ..Default::default()
        };
        let custom = BifragmentCarver::with_config(custom_config);
        assert_eq!(custom.config.cluster_size, 8192);
    }

    #[test]
    fn test_find_eoi() {
        let carver = BifragmentCarver::new();

        let data = [0x00, 0x11, 0x22, 0x33, 0x44, 0xFF, 0xD9, 0x00];
        assert_eq!(carver.find_eoi(&data), Some(5));

        let no_eoi = [0x00, 0xFF, 0xD8, 0x00, 0x00];
        assert_eq!(carver.find_eoi(&no_eoi), None);

        let eoi_start = [0xFF, 0xD9, 0x00, 0x00];
        assert_eq!(carver.find_eoi(&eoi_start), Some(0));
    }

    #[test]
    fn test_fragment_candidate() {
        let candidate = FragmentCandidate {
            offset: 1000,
            size: 500,
            confidence: 0.85,
            dc_continuity: Some(0.9),
        };
        assert_eq!(candidate.offset, 1000);
        assert!(candidate.confidence > 0.8);
    }
}
