use crate::recovery::signatures::FileType;
use image::GenericImageView;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ValidationConfig {
    pub min_file_size_bytes: usize,
    pub min_width: u32,
    pub min_height: u32,
    pub strict_mode: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_file_size_bytes: 100 * 1024,
            min_width: 600,
            min_height: 600,
            strict_mode: true,
        }
    }
}

impl ValidationConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_size(mut self, size_kb: usize) -> Self {
        self.min_file_size_bytes = size_kb * 1024;
        self
    }

    pub fn with_min_dimensions(mut self, width: u32, height: u32) -> Self {
        self.min_width = width;
        self.min_height = height;
        self
    }

    pub fn with_strict_mode(mut self, enabled: bool) -> Self {
        self.strict_mode = enabled;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationResult {
    Valid,
    TooSmall,
    Corrupted,
    BelowMinDimensions,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            ValidationResult::Valid => "valid",
            ValidationResult::TooSmall => "file too small",
            ValidationResult::Corrupted => "corrupted or invalid",
            ValidationResult::BelowMinDimensions => "dimensions too small",
        }
    }
}

#[derive(Debug, Default)]
pub struct ValidationStats {
    pub total_candidates: AtomicUsize,
    pub too_small: AtomicUsize,
    pub corrupted: AtomicUsize,
    pub below_min_dimensions: AtomicUsize,
    pub valid: AtomicUsize,
}

impl ValidationStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record(&self, result: ValidationResult) {
        self.total_candidates.fetch_add(1, Ordering::Relaxed);
        match result {
            ValidationResult::Valid => {
                self.valid.fetch_add(1, Ordering::Relaxed);
            }
            ValidationResult::TooSmall => {
                self.too_small.fetch_add(1, Ordering::Relaxed);
            }
            ValidationResult::Corrupted => {
                self.corrupted.fetch_add(1, Ordering::Relaxed);
            }
            ValidationResult::BelowMinDimensions => {
                self.below_min_dimensions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn summary(&self) -> String {
        let total = self.total_candidates.load(Ordering::Relaxed);
        let valid = self.valid.load(Ordering::Relaxed);
        let too_small = self.too_small.load(Ordering::Relaxed);
        let corrupted = self.corrupted.load(Ordering::Relaxed);
        let below_dims = self.below_min_dimensions.load(Ordering::Relaxed);
        let discarded = too_small + corrupted + below_dims;

        format!(
            "Validation Summary:\n\
             - Total candidates: {}\n\
             - Valid files saved: {}\n\
             - Discarded files: {} ({:.1}%)\n\
               • Too small: {}\n\
               • Corrupted: {}\n\
               • Below min dimensions: {}",
            total,
            valid,
            discarded,
            if total > 0 {
                (discarded as f64 / total as f64) * 100.0
            } else {
                0.0
            },
            too_small,
            corrupted,
            below_dims
        )
    }
}

pub struct ImageValidator {
    config: ValidationConfig,
}

impl ImageValidator {
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(ValidationConfig::default())
    }

    pub fn validate_file_data(&self, data: &[u8], _file_type: FileType) -> ValidationResult {
        if data.len() < self.config.min_file_size_bytes {
            return ValidationResult::TooSmall;
        }

        if !self.config.strict_mode {
            return ValidationResult::Valid;
        }
        match image::load_from_memory(data) {
            Ok(img) => {
                // Check dimensions
                let (width, height) = img.dimensions();
                if width < self.config.min_width || height < self.config.min_height {
                    ValidationResult::BelowMinDimensions
                } else {
                    ValidationResult::Valid
                }
            }
            Err(_) => {
                ValidationResult::Corrupted
            }
        }
    }

    pub fn config(&self) -> &ValidationConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_config_defaults() {
        let config = ValidationConfig::default();
        assert_eq!(config.min_file_size_bytes, 100 * 1024);
        assert_eq!(config.min_width, 600);
        assert_eq!(config.min_height, 600);
        assert!(config.strict_mode);
    }

    #[test]
    fn test_validation_config_builder() {
        let config = ValidationConfig::new()
            .with_min_size(50)
            .with_min_dimensions(400, 400)
            .with_strict_mode(false);

        assert_eq!(config.min_file_size_bytes, 50 * 1024);
        assert_eq!(config.min_width, 400);
        assert_eq!(config.min_height, 400);
        assert!(!config.strict_mode);
    }

    #[test]
    fn test_validation_result_is_valid() {
        assert!(ValidationResult::Valid.is_valid());
        assert!(!ValidationResult::TooSmall.is_valid());
        assert!(!ValidationResult::Corrupted.is_valid());
        assert!(!ValidationResult::BelowMinDimensions.is_valid());
    }

    #[test]
    fn test_too_small_file() {
        let config = ValidationConfig::default();
        let validator = ImageValidator::new(config);
        let small_data = vec![0u8; 1024]; // 1KB

        let result = validator.validate_file_data(&small_data, FileType::Jpeg);
        assert_eq!(result, ValidationResult::TooSmall);
    }

    #[test]
    fn test_validation_stats() {
        let stats = ValidationStats::new();
        stats.record(ValidationResult::Valid);
        stats.record(ValidationResult::TooSmall);
        stats.record(ValidationResult::Corrupted);

        assert_eq!(stats.total_candidates.load(Ordering::Relaxed), 3);
        assert_eq!(stats.valid.load(Ordering::Relaxed), 1);
        assert_eq!(stats.too_small.load(Ordering::Relaxed), 1);
        assert_eq!(stats.corrupted.load(Ordering::Relaxed), 1);
    }
}
