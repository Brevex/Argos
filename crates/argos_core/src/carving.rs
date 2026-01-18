use crate::jpeg::{JpegParser, JpegValidator, RestartMarkerScanner, ValidationResult};
use crate::png::{PngValidationResult, PngValidator};
use crate::statistics::{ImageClassification, ImageClassifier, ImageStatistics};
use crate::{BlockSource, FileType};

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

#[derive(Debug, Clone)]
pub struct SmartCarverConfig {
    pub structural_validation: bool,
    pub bifragment_carving: bool,
    pub statistical_filtering: bool,
    pub filter_thumbnails: bool,
    pub filter_graphics: bool,
    pub cluster_size: u64,
    pub max_gap: u64,
    pub min_confidence: f32,
}

impl Default for SmartCarverConfig {
    fn default() -> Self {
        Self {
            structural_validation: true,
            bifragment_carving: true,
            statistical_filtering: true,
            filter_thumbnails: true,
            filter_graphics: true,
            cluster_size: 4096,
            max_gap: 16 * 1024 * 1024,
            min_confidence: 0.6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarveDecision {
    Extract,
    ExtractPartial,
    Skip(SkipReason),
    AttemptBgc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Thumbnail,
    ArtificialGraphic,
    Encrypted,
    TooSmall,
    InvalidStructure,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationNote {
    StructureValid,
    CorruptionAt(u64),
    BgcSuccessful,
    BgcFailed,
    Truncated,
    ContainsExifThumbnail,
    CrcErrors(usize),
    ParseFailed,
}

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

    #[inline]
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
pub struct SmartCarveResult {
    pub decision: CarveDecision,
    pub file_type: FileType,
    pub offset: u64,
    pub size: u64,
    pub bgc_result: Option<CarveResult>,
    pub statistics: Option<ImageStatistics>,
    pub classification: Option<ImageClassification>,
    pub is_thumbnail: bool,
    pub validation_notes: Vec<ValidationNote>,
}

impl SmartCarveResult {
    pub fn extract(file_type: FileType, offset: u64, size: u64) -> Self {
        Self {
            decision: CarveDecision::Extract,
            file_type,
            offset,
            size,
            bgc_result: None,
            statistics: None,
            classification: None,
            is_thumbnail: false,
            validation_notes: Vec::new(),
        }
    }

    pub fn skip(file_type: FileType, offset: u64, reason: SkipReason) -> Self {
        Self {
            decision: CarveDecision::Skip(reason),
            file_type,
            offset,
            size: 0,
            bgc_result: None,
            statistics: None,
            classification: None,
            is_thumbnail: false,
            validation_notes: Vec::new(),
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
                return Some(CarveResult::contiguous(head_offset, head.len() as u64))
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

        best_candidate.map(|(candidate, _)| {
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

        match self.validator.validate(&combined) {
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

pub struct SmartCarver {
    config: SmartCarverConfig,
    jpeg_validator: JpegValidator,
    jpeg_parser: JpegParser,
    png_validator: PngValidator,
    bgc_carver: BifragmentCarver,
    classifier: ImageClassifier,
    #[allow(dead_code)]
    rst_scanner: RestartMarkerScanner,
}

impl SmartCarver {
    pub fn new() -> Self {
        Self::with_config(SmartCarverConfig::default())
    }

    pub fn with_config(config: SmartCarverConfig) -> Self {
        let bgc_config = BgcConfig {
            cluster_size: config.cluster_size,
            max_gap: config.max_gap,
            min_confidence: config.min_confidence,
            ..Default::default()
        };
        Self {
            jpeg_validator: JpegValidator::new(),
            jpeg_parser: JpegParser::new(),
            png_validator: PngValidator::new(),
            bgc_carver: BifragmentCarver::with_config(bgc_config),
            classifier: ImageClassifier::new(),
            rst_scanner: RestartMarkerScanner::new(),
            config,
        }
    }

    #[inline]
    pub fn config(&self) -> &SmartCarverConfig {
        &self.config
    }

    pub fn analyze_jpeg<S: BlockSource>(
        &self,
        data: &[u8],
        offset: u64,
        source: &mut S,
    ) -> SmartCarveResult {
        let mut result = SmartCarveResult::extract(FileType::Jpeg, offset, data.len() as u64);

        let structure = match self.jpeg_parser.parse(data) {
            Ok(s) => s,
            Err(_) => {
                result.decision = CarveDecision::Skip(SkipReason::InvalidStructure);
                result.validation_notes.push(ValidationNote::ParseFailed);
                return result;
            }
        };

        if self.config.filter_thumbnails && structure.thumbnail.is_some() {
            result.is_thumbnail = false;
            result.validation_notes.push(ValidationNote::ContainsExifThumbnail);
        }

        if self.config.structural_validation {
            let validation = self.jpeg_validator.validate(data);
            match validation {
                ValidationResult::Valid(_) => {
                    result.validation_notes.push(ValidationNote::StructureValid);
                }
                ValidationResult::CorruptedAt {
                    offset: corrupt_off,
                    ..
                } => {
                    result.validation_notes.push(ValidationNote::CorruptionAt(corrupt_off));
                    if self.config.bifragment_carving {
                        result.decision = CarveDecision::AttemptBgc;
                        if let Some(bgc) = self.bgc_carver.carve_bifragment(data, offset, source) {
                            if bgc.is_fragmented {
                                result.bgc_result = Some(bgc.clone());
                                result.size = bgc.total_size();
                                result.decision = CarveDecision::Extract;
                                result.validation_notes.push(ValidationNote::BgcSuccessful);
                            }
                        } else {
                            result.decision = CarveDecision::ExtractPartial;
                            result.validation_notes.push(ValidationNote::BgcFailed);
                        }
                    } else {
                        result.decision = CarveDecision::ExtractPartial;
                    }
                }
                ValidationResult::Truncated { .. } => {
                    result.decision = CarveDecision::ExtractPartial;
                    result.validation_notes.push(ValidationNote::Truncated);
                }
                ValidationResult::InvalidHeader => {
                    result.decision = CarveDecision::Skip(SkipReason::InvalidStructure);
                    return result;
                }
            }
        }
        result
    }

    pub fn analyze_png(&self, data: &[u8], offset: u64) -> SmartCarveResult {
        let mut result = SmartCarveResult::extract(FileType::Png, offset, data.len() as u64);

        if !self.config.structural_validation {
            return result;
        }

        match self.png_validator.validate(data) {
            PngValidationResult::Valid(structure) => {
                result.size = structure.valid_end_offset;
                result.validation_notes.push(ValidationNote::StructureValid);
            }
            PngValidationResult::RecoverableCrcErrors { structure, errors } => {
                result.size = structure.valid_end_offset;
                result.validation_notes.push(ValidationNote::CrcErrors(errors.len()));
            }
            PngValidationResult::Truncated {
                last_valid_offset, ..
            } => {
                result.decision = CarveDecision::ExtractPartial;
                result.size = last_valid_offset;
                result.validation_notes.push(ValidationNote::Truncated);
            }
            PngValidationResult::CorruptedAt {
                offset: corrupt_off,
                ..
            } => {
                result.decision = CarveDecision::ExtractPartial;
                result.validation_notes.push(ValidationNote::CorruptionAt(corrupt_off));
            }
            PngValidationResult::InvalidHeader => {
                result.decision = CarveDecision::Skip(SkipReason::InvalidStructure);
            }
        }
        result
    }

    pub fn classify_image(
        &self,
        pixel_data: &[u8],
        width: usize,
        height: usize,
        channels: usize,
    ) -> (ImageClassification, ImageStatistics) {
        let stats = self
            .classifier
            .compute_statistics(pixel_data, width, height, channels);
        let classification = self.classifier.classify(&stats, width * height);
        (classification, stats)
    }

    pub fn filter_by_classification(
        &self,
        classification: ImageClassification,
    ) -> Option<SkipReason> {
        if !self.config.statistical_filtering {
            return None;
        }
        match classification {
            ImageClassification::NaturalPhoto => None,
            ImageClassification::ArtificialGraphic if self.config.filter_graphics => {
                Some(SkipReason::ArtificialGraphic)
            }
            ImageClassification::Encrypted => Some(SkipReason::Encrypted),
            ImageClassification::TooSmall => Some(SkipReason::TooSmall),
            _ => None,
        }
    }
}

impl Default for SmartCarver {
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
        assert_eq!(result.total_size(), 5000);
    }

    #[test]
    fn test_carve_result_fragmented() {
        let result = CarveResult::fragmented(1000, 2000, 5000, 3000, 0.9);
        assert!(result.is_fragmented);
        assert_eq!(result.total_size(), 5000);
        assert_eq!(result.gap_size, Some(2000));
    }

    #[test]
    fn test_smart_carver_creation() {
        let carver = SmartCarver::new();
        assert!(carver.config().structural_validation);
    }

    #[test]
    fn test_carve_decision_variants() {
        let extract = CarveDecision::Extract;
        let partial = CarveDecision::ExtractPartial;
        assert_ne!(extract, partial);
    }

    #[test]
    fn test_filter_by_classification() {
        let carver = SmartCarver::new();
        assert!(carver
            .filter_by_classification(ImageClassification::NaturalPhoto)
            .is_none());
        assert!(carver
            .filter_by_classification(ImageClassification::ArtificialGraphic)
            .is_some());
        assert!(carver
            .filter_by_classification(ImageClassification::Encrypted)
            .is_some());
    }

    #[test]
    fn test_custom_config() {
        let config = SmartCarverConfig {
            structural_validation: false,
            bifragment_carving: false,
            cluster_size: 8192,
            ..Default::default()
        };
        let carver = SmartCarver::with_config(config);
        assert!(!carver.config().structural_validation);
        assert_eq!(carver.config().cluster_size, 8192);
    }
}
