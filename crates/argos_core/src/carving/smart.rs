use crate::carving::{BgcConfig, BifragmentCarver, CarveResult};
use crate::jpeg::{JpegParser, JpegValidator, RestartMarkerScanner, ValidationResult};
use crate::png::{PngValidationResult, PngValidator};
use crate::statistics::{ImageClassification, ImageClassifier, ImageStatistics};
use crate::traits::BlockSource;
use crate::types::FileType;

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
    pub validation_notes: Vec<String>,
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
                result
                    .validation_notes
                    .push("Failed to parse JPEG structure".into());
                return result;
            }
        };

        if self.config.filter_thumbnails && structure.thumbnail.is_some() {
            result.is_thumbnail = false;
            result
                .validation_notes
                .push("Contains embedded Exif thumbnail".into());
        }

        if self.config.structural_validation {
            let validation = self.jpeg_validator.validate(data);

            match validation {
                ValidationResult::Valid(_) => {
                    result.validation_notes.push("Structure valid".into());
                }
                ValidationResult::CorruptedAt {
                    offset: corrupt_off,
                    ..
                } => {
                    result
                        .validation_notes
                        .push(format!("Corruption detected at offset {}", corrupt_off));

                    if self.config.bifragment_carving {
                        result.decision = CarveDecision::AttemptBgc;

                        if let Some(bgc) = self.bgc_carver.carve_bifragment(data, offset, source) {
                            if bgc.is_fragmented {
                                result.bgc_result = Some(bgc.clone());
                                result.size = bgc.total_size();
                                result.decision = CarveDecision::Extract;
                                result
                                    .validation_notes
                                    .push("Bifragment reconstruction successful".into());
                            }
                        } else {
                            result.decision = CarveDecision::ExtractPartial;
                            result
                                .validation_notes
                                .push("BGC failed, extracting partial".into());
                        }
                    } else {
                        result.decision = CarveDecision::ExtractPartial;
                    }
                }
                ValidationResult::Truncated { .. } => {
                    result.decision = CarveDecision::ExtractPartial;
                    result
                        .validation_notes
                        .push("File appears truncated".into());
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

        let validation = self.png_validator.validate(data);

        match validation {
            PngValidationResult::Valid(structure) => {
                result.size = structure.valid_end_offset;
                result.validation_notes.push("Structure valid".into());
            }
            PngValidationResult::RecoverableCrcErrors { structure, errors } => {
                result.size = structure.valid_end_offset;
                result.validation_notes.push(format!(
                    "{} chunks with CRC errors (recoverable)",
                    errors.len()
                ));
            }
            PngValidationResult::Truncated {
                last_valid_offset, ..
            } => {
                result.decision = CarveDecision::ExtractPartial;
                result.size = last_valid_offset;
                result
                    .validation_notes
                    .push("File appears truncated".into());
            }
            PngValidationResult::CorruptedAt {
                offset: corrupt_off,
                ..
            } => {
                result.decision = CarveDecision::ExtractPartial;
                result
                    .validation_notes
                    .push(format!("Corruption at offset {}", corrupt_off));
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
    fn test_config_default() {
        let config = SmartCarverConfig::default();
        assert!(config.structural_validation);
        assert!(config.bifragment_carving);
        assert!(config.filter_thumbnails);
        assert_eq!(config.cluster_size, 4096);
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
        let skip = CarveDecision::Skip(SkipReason::Thumbnail);
        let bgc = CarveDecision::AttemptBgc;

        assert_ne!(extract, partial);
        assert_ne!(skip, bgc);
    }

    #[test]
    fn test_skip_reason_variants() {
        let reasons = vec![
            SkipReason::Thumbnail,
            SkipReason::ArtificialGraphic,
            SkipReason::Encrypted,
            SkipReason::TooSmall,
            SkipReason::InvalidStructure,
            SkipReason::Duplicate,
        ];

        assert_eq!(reasons.len(), 6);
    }

    #[test]
    fn test_smart_carve_result_extract() {
        let result = SmartCarveResult::extract(FileType::Jpeg, 1000, 5000);
        assert_eq!(result.decision, CarveDecision::Extract);
        assert_eq!(result.offset, 1000);
        assert_eq!(result.size, 5000);
        assert!(!result.is_thumbnail);
    }

    #[test]
    fn test_smart_carve_result_skip() {
        let result = SmartCarveResult::skip(FileType::Jpeg, 1000, SkipReason::Thumbnail);
        assert!(matches!(
            result.decision,
            CarveDecision::Skip(SkipReason::Thumbnail)
        ));
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
        assert!(!carver.config().bifragment_carving);
        assert_eq!(carver.config().cluster_size, 8192);
    }
}
