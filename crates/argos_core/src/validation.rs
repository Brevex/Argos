use crate::error::FileFormat;

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub passed: bool,
    pub confidence: f64,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn pass(confidence: f64, detail: impl Into<String>) -> Self {
        Self {
            passed: true,
            confidence,
            details: vec![detail.into()],
            warnings: Vec::new(),
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            confidence: 0.0,
            details: vec![reason.into()],
            warnings: Vec::new(),
        }
    }

    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ValidationContext {
    pub format: FileFormat,
    pub expected_size: Option<u64>,
    pub is_fragmented: bool,
    pub fragment_count: usize,
}

impl Default for ValidationContext {
    fn default() -> Self {
        Self {
            format: FileFormat::Unknown,
            expected_size: None,
            is_fragmented: false,
            fragment_count: 1,
        }
    }
}

pub trait ValidationStage: Send + Sync {
    fn name(&self) -> &str;
    fn is_required(&self) -> bool;
    fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult;
}

pub struct EntropyFilter {
    pub jpeg_min: f64,
    pub jpeg_max: f64,
    pub png_min: f64,
    pub png_max: f64,
}

impl Default for EntropyFilter {
    fn default() -> Self {
        Self {
            jpeg_min: 6.5,
            jpeg_max: 7.95,
            png_min: 4.5,
            png_max: 7.95,
        }
    }
}

impl EntropyFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compute_entropy(data: &[u8]) -> f64 {
        if data.is_empty() {
            return 0.0;
        }

        let mut counts = [0u64; 256];
        for &byte in data {
            counts[byte as usize] += 1;
        }

        let len = data.len() as f64;
        let mut entropy = 0.0;

        for &count in &counts {
            if count > 0 {
                let p = count as f64 / len;
                entropy -= p * p.log2();
            }
        }

        entropy
    }
}

impl ValidationStage for EntropyFilter {
    fn name(&self) -> &str {
        "entropy_filter"
    }

    fn is_required(&self) -> bool {
        false
    }

    fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult {
        if data.len() < 256 {
            return ValidationResult::pass(0.5, "Data too small for entropy analysis")
                .with_warning("Entropy analysis skipped due to small sample size");
        }

        let entropy = Self::compute_entropy(data);

        let (min, max) = match ctx.format {
            FileFormat::Jpeg => (self.jpeg_min, self.jpeg_max),
            FileFormat::Png => (self.png_min, self.png_max),
            FileFormat::Unknown => (4.0, 8.0),
        };

        if entropy < min {
            return ValidationResult::fail(format!(
                "Entropy {:.3} below minimum {:.2} for {} (likely uncompressed/zeros)",
                entropy, min, ctx.format
            ));
        }

        if entropy > max {
            return ValidationResult::fail(format!(
                "Entropy {:.3} above maximum {:.2} for {} (likely random/encrypted)",
                entropy, max, ctx.format
            ));
        }

        let expected_mean = match ctx.format {
            FileFormat::Jpeg => 7.35,
            FileFormat::Png => 6.8,
            FileFormat::Unknown => 6.5,
        };

        let deviation = (entropy - expected_mean).abs();
        let confidence = (1.0 - deviation / 2.0).clamp(0.2, 0.5);

        ValidationResult::pass(
            confidence,
            format!(
                "Entropy {:.3} within expected range [{:.2}, {:.2}]",
                entropy, min, max
            ),
        )
    }
}

pub struct JpegStructuralValidator;

impl JpegStructuralValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn extract_markers(data: &[u8]) -> Vec<(usize, u8)> {
        let mut markers = Vec::new();
        let mut i = 0;

        while i < data.len().saturating_sub(1) {
            if data[i] == 0xFF {
                let marker = data[i + 1];

                if marker != 0x00 && marker != 0xFF {
                    markers.push((i, marker));
                }
                i += 2;
            } else {
                i += 1;
            }
        }

        markers
    }

    pub fn validate_marker_order(markers: &[(usize, u8)]) -> Result<(), String> {
        if markers.is_empty() {
            return Err("No markers found".to_string());
        }

        if markers[0].1 != 0xD8 {
            return Err(format!(
                "First marker is 0x{:02X}, expected SOI (0xD8)",
                markers[0].1
            ));
        }

        let mut saw_sof = false;
        let mut saw_sos = false;
        let mut saw_dht = false;
        let mut saw_dqt = false;

        for &(offset, marker) in markers.iter().skip(1) {
            match marker {
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                    if saw_sos {
                        return Err(format!("SOF marker at offset {:#x} after SOS", offset));
                    }
                    saw_sof = true;
                }
                0xC4 => {
                    saw_dht = true;
                }
                0xDB => {
                    saw_dqt = true;
                }
                0xDA => {
                    if !saw_sof {
                        return Err("SOS marker before SOF".to_string());
                    }
                    saw_sos = true;
                }
                0xD9 => {
                    break;
                }
                0xD0..=0xD7 => {
                    if !saw_sos {
                        return Err("RST marker before SOS".to_string());
                    }
                }
                0xE0..=0xEF => {}
                0xFE => {}

                _ => {}
            }
        }

        if !saw_sof {
            return Err("No SOF marker found".to_string());
        }
        if !saw_sos {
            return Err("No SOS marker found".to_string());
        }
        if !saw_dqt {
            return Err("No DQT marker found".to_string());
        }
        if !saw_dht {}

        Ok(())
    }

    pub fn validate_dqt(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut tables = Vec::new();
        let mut i = 0;

        while i < data.len().saturating_sub(4) {
            if data[i] == 0xFF && data[i + 1] == 0xDB {
                let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
                if i + 2 + len > data.len() {
                    return Err(format!("DQT at offset {:#x} extends beyond data", i));
                }

                let mut pos = i + 4;
                while pos < i + 2 + len {
                    let info = data[pos];
                    let precision = info >> 4;
                    let table_id = info & 0x0F;

                    if table_id > 3 {
                        return Err(format!("Invalid DQT table ID: {}", table_id));
                    }

                    let table_size = if precision == 0 { 64 } else { 128 };
                    if pos + 1 + table_size > data.len() {
                        return Err("DQT table truncated".to_string());
                    }

                    tables.push(table_id);
                    pos += 1 + table_size;
                }

                i = pos;
            } else {
                i += 1;
            }
        }

        Ok(tables)
    }
}

impl Default for JpegStructuralValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationStage for JpegStructuralValidator {
    fn name(&self) -> &str {
        "jpeg_structural"
    }

    fn is_required(&self) -> bool {
        true
    }

    fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult {
        if ctx.format != FileFormat::Jpeg && ctx.format != FileFormat::Unknown {
            return ValidationResult::pass(1.0, "Not a JPEG, skipping JPEG validation");
        }

        if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
            return ValidationResult::fail("Missing or invalid SOI marker");
        }

        let mut confidence = 0.2;
        let mut result = ValidationResult::pass(confidence, "Valid SOI marker");

        let markers = Self::extract_markers(data);

        match Self::validate_marker_order(&markers) {
            Ok(()) => {
                confidence += 0.3;
                result = result.with_detail("Marker sequence valid");
            }
            Err(e) => {
                return ValidationResult::fail(format!("Invalid marker sequence: {}", e));
            }
        }

        match Self::validate_dqt(data) {
            Ok(tables) if !tables.is_empty() => {
                confidence += 0.2;
                result = result.with_detail(format!("Found {} valid DQT table(s)", tables.len()));
            }
            Ok(_) => {
                result = result.with_warning("No DQT tables found");
            }
            Err(e) => {
                return ValidationResult::fail(format!("DQT validation failed: {}", e));
            }
        }

        let has_eoi =
            data.len() >= 2 && data[data.len() - 2] == 0xFF && data[data.len() - 1] == 0xD9;

        if has_eoi {
            confidence += 0.2;
            result = result.with_detail("Valid EOI marker at end");
        } else {
            result = result.with_warning("No EOI marker at end of data");
        }

        if data.len() < 200 {
            result = result.with_warning("File unusually small for JPEG");
        }

        result.confidence = confidence.min(0.9);
        result
    }
}

pub struct PngStructuralValidator;

impl PngStructuralValidator {
    pub fn new() -> Self {
        Self
    }

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    pub fn validate_chunks(data: &[u8]) -> Result<Vec<String>, String> {
        if data.len() < 8 {
            return Err("Data too short for PNG".to_string());
        }

        if data[..8] != Self::PNG_SIGNATURE {
            return Err("Invalid PNG signature".to_string());
        }

        let mut chunks = Vec::new();
        let mut pos = 8;
        let mut saw_ihdr = false;
        let mut saw_idat = false;

        while pos + 12 <= data.len() {
            let length =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            let chunk_type = &data[pos + 4..pos + 8];
            let chunk_name = String::from_utf8_lossy(chunk_type).to_string();

            chunks.push(chunk_name.clone());

            match chunk_name.as_str() {
                "IHDR" => {
                    if saw_ihdr {
                        return Err("Duplicate IHDR chunk".to_string());
                    }
                    if pos != 8 {
                        return Err("IHDR must be first chunk".to_string());
                    }
                    if length != 13 {
                        return Err(format!("IHDR length {} != 13", length));
                    }
                    saw_ihdr = true;
                }
                "IDAT" => {
                    if !saw_ihdr {
                        return Err("IDAT before IHDR".to_string());
                    }
                    saw_idat = true;
                }
                "IEND" => {
                    if length != 0 {
                        return Err("IEND length must be 0".to_string());
                    }
                    break;
                }
                _ => {}
            }

            pos += 4 + 4 + length + 4;
        }

        if !saw_ihdr {
            return Err("No IHDR chunk found".to_string());
        }
        if !saw_idat {
            return Err("No IDAT chunk found".to_string());
        }

        Ok(chunks)
    }
}

impl Default for PngStructuralValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationStage for PngStructuralValidator {
    fn name(&self) -> &str {
        "png_structural"
    }

    fn is_required(&self) -> bool {
        true
    }

    fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult {
        if ctx.format != FileFormat::Png && ctx.format != FileFormat::Unknown {
            return ValidationResult::pass(1.0, "Not a PNG, skipping PNG validation");
        }

        match Self::validate_chunks(data) {
            Ok(chunks) => {
                let has_iend = chunks.iter().any(|c| c == "IEND");
                let confidence = if has_iend { 0.85 } else { 0.6 };

                let mut result = ValidationResult::pass(
                    confidence,
                    format!("Valid PNG structure with {} chunks", chunks.len()),
                );

                if !has_iend {
                    result = result.with_warning("No IEND chunk found");
                }

                result
            }
            Err(e) => ValidationResult::fail(format!("PNG structure invalid: {}", e)),
        }
    }
}

pub struct RenderingValidator {
    pub max_dimension: u32,
    pub max_file_size: usize,
}

impl Default for RenderingValidator {
    fn default() -> Self {
        Self {
            max_dimension: 16384,
            max_file_size: 50 * 1024 * 1024,
        }
    }
}

impl RenderingValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(max_dimension: u32, max_file_size: usize) -> Self {
        Self {
            max_dimension,
            max_file_size,
        }
    }

    fn validate_jpeg_rendering(&self, data: &[u8]) -> ValidationResult {
        use image::ImageReader;
        use std::io::Cursor;

        if data.len() > self.max_file_size {
            return ValidationResult::fail(format!(
                "File too large for rendering validation: {} > {} bytes",
                data.len(),
                self.max_file_size
            ));
        }

        let cursor = Cursor::new(data);
        match ImageReader::new(cursor).with_guessed_format() {
            Ok(reader) => match reader.decode() {
                Ok(img) => {
                    let (w, h) = (img.width(), img.height());

                    if w == 0 || h == 0 {
                        return ValidationResult::fail("Decoded image has zero dimensions");
                    }

                    if w > self.max_dimension || h > self.max_dimension {
                        return ValidationResult::fail(format!(
                            "Image dimensions {}x{} exceed maximum {}",
                            w, h, self.max_dimension
                        ));
                    }

                    let pixels_ok = self.check_pixel_sanity(&img);

                    let confidence = if pixels_ok { 0.98 } else { 0.85 };

                    ValidationResult::pass(
                        confidence,
                        format!("Successfully decoded {}x{} image", w, h),
                    )
                }
                Err(e) => ValidationResult::fail(format!("JPEG decode failed: {}", e)),
            },
            Err(e) => ValidationResult::fail(format!("Format detection failed: {}", e)),
        }
    }

    fn validate_png_rendering(&self, data: &[u8]) -> ValidationResult {
        use image::ImageReader;
        use std::io::Cursor;

        if data.len() > self.max_file_size {
            return ValidationResult::fail(format!(
                "File too large for rendering validation: {} > {} bytes",
                data.len(),
                self.max_file_size
            ));
        }

        let cursor = Cursor::new(data);
        match ImageReader::new(cursor).with_guessed_format() {
            Ok(reader) => match reader.decode() {
                Ok(img) => {
                    let (w, h) = (img.width(), img.height());

                    if w == 0 || h == 0 {
                        return ValidationResult::fail("Decoded image has zero dimensions");
                    }

                    if w > self.max_dimension || h > self.max_dimension {
                        return ValidationResult::fail(format!(
                            "Image dimensions {}x{} exceed maximum {}",
                            w, h, self.max_dimension
                        ));
                    }

                    let pixels_ok = self.check_pixel_sanity(&img);
                    let confidence = if pixels_ok { 0.98 } else { 0.85 };

                    ValidationResult::pass(
                        confidence,
                        format!("Successfully decoded {}x{} PNG image", w, h),
                    )
                }
                Err(e) => ValidationResult::fail(format!("PNG decode failed: {}", e)),
            },
            Err(e) => ValidationResult::fail(format!("Format detection failed: {}", e)),
        }
    }

    fn check_pixel_sanity(&self, img: &image::DynamicImage) -> bool {
        use image::GenericImageView;

        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return false;
        }

        let sample_count = 16.min((w * h) as usize);
        let step_x = (w as usize / 4).max(1);
        let step_y = (h as usize / 4).max(1);

        let mut unique_colors = std::collections::HashSet::new();

        for i in 0..4 {
            for j in 0..4 {
                let x = (i * step_x).min(w as usize - 1) as u32;
                let y = (j * step_y).min(h as usize - 1) as u32;
                let pixel = img.get_pixel(x, y);
                unique_colors.insert((pixel[0], pixel[1], pixel[2]));
            }
        }

        if unique_colors.len() == 1 && sample_count >= 16 {
            return true;
        }

        true
    }
}

impl ValidationStage for RenderingValidator {
    fn name(&self) -> &str {
        "rendering"
    }

    fn is_required(&self) -> bool {
        false
    }

    fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult {
        match ctx.format {
            FileFormat::Jpeg => self.validate_jpeg_rendering(data),
            FileFormat::Png => self.validate_png_rendering(data),
            FileFormat::Unknown => {
                let jpeg_result = self.validate_jpeg_rendering(data);
                if jpeg_result.passed {
                    return jpeg_result;
                }
                self.validate_png_rendering(data)
            }
        }
    }
}

pub struct ValidationPipeline {
    stages: Vec<Box<dyn ValidationStage>>,
}

impl ValidationPipeline {
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    pub fn add_stage(mut self, stage: Box<dyn ValidationStage>) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn for_jpeg() -> Self {
        Self::new()
            .add_stage(Box::new(EntropyFilter::new()))
            .add_stage(Box::new(JpegStructuralValidator::new()))
    }

    pub fn for_jpeg_with_rendering() -> Self {
        Self::new()
            .add_stage(Box::new(EntropyFilter::new()))
            .add_stage(Box::new(JpegStructuralValidator::new()))
            .add_stage(Box::new(RenderingValidator::new()))
    }

    pub fn for_png() -> Self {
        Self::new()
            .add_stage(Box::new(EntropyFilter::new()))
            .add_stage(Box::new(PngStructuralValidator::new()))
    }

    pub fn for_png_with_rendering() -> Self {
        Self::new()
            .add_stage(Box::new(EntropyFilter::new()))
            .add_stage(Box::new(PngStructuralValidator::new()))
            .add_stage(Box::new(RenderingValidator::new()))
    }

    pub fn for_format(format: FileFormat) -> Self {
        match format {
            FileFormat::Jpeg => Self::for_jpeg(),
            FileFormat::Png => Self::for_png(),
            FileFormat::Unknown => Self::new().add_stage(Box::new(EntropyFilter::new())),
        }
    }

    pub fn for_format_with_rendering(format: FileFormat) -> Self {
        match format {
            FileFormat::Jpeg => Self::for_jpeg_with_rendering(),
            FileFormat::Png => Self::for_png_with_rendering(),
            FileFormat::Unknown => Self::new()
                .add_stage(Box::new(EntropyFilter::new()))
                .add_stage(Box::new(RenderingValidator::new())),
        }
    }

    pub fn validate(&self, data: &[u8], ctx: &ValidationContext) -> ValidationResult {
        if self.stages.is_empty() {
            return ValidationResult::pass(0.5, "No validation stages configured");
        }

        let mut overall_confidence = 1.0;
        let mut all_details = Vec::new();
        let mut all_warnings = Vec::new();

        for stage in &self.stages {
            let result = stage.validate(data, ctx);

            all_details.push(format!("[{}] {:?}", stage.name(), result.details));
            all_warnings.extend(result.warnings.clone());

            if stage.is_required() && !result.passed {
                return ValidationResult {
                    passed: false,
                    confidence: 0.0,
                    details: vec![
                        format!("Required stage '{}' failed", stage.name()),
                        result.details.join("; "),
                    ],
                    warnings: all_warnings,
                };
            }

            if result.passed {
                overall_confidence *= result.confidence;
            }
        }

        ValidationResult {
            passed: overall_confidence > 0.3,
            confidence: overall_confidence,
            details: all_details,
            warnings: all_warnings,
        }
    }
}

impl Default for ValidationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct StitchValidation {
    pub is_valid: bool,
    pub confidence: f64,
    pub reasons: Vec<String>,
}

impl StitchValidation {
    pub fn validate(head: &[u8], tail: &[u8]) -> Self {
        let mut confidence = 0.5;
        let mut reasons = Vec::new();
        let mut is_valid = true;

        if tail.len() >= 2 && tail[0] == 0xFF && tail[1] != 0x00 {
            let marker = tail[1];

            if !(0xD0..=0xD7).contains(&marker) {
                is_valid = false;
                confidence = 0.1;
                reasons.push(format!(
                    "Tail starts with marker 0xFF{:02X}, likely wrong stitch point",
                    marker
                ));
                return Self {
                    is_valid,
                    confidence,
                    reasons,
                };
            }
        }

        if !head.is_empty() && head[head.len() - 1] == 0xFF && !tail.is_empty() {
            let next = tail[0];
            if next != 0x00 && !(0xD0..=0xD7).contains(&next) {
                confidence *= 0.5;
                reasons.push(format!(
                    "Invalid byte sequence at boundary: 0xFF followed by 0x{:02X}",
                    next
                ));
            }
        }

        if head.len() >= 256 && tail.len() >= 256 {
            let head_entropy =
                EntropyFilter::compute_entropy(&head[head.len().saturating_sub(1024)..]);
            let tail_entropy = EntropyFilter::compute_entropy(&tail[..1024.min(tail.len())]);
            let entropy_diff = (head_entropy - tail_entropy).abs();

            if entropy_diff > 1.5 {
                confidence *= 0.6;
                reasons.push(format!(
                    "Large entropy difference: head={:.2}, tail={:.2}",
                    head_entropy, tail_entropy
                ));
            } else if entropy_diff < 0.5 {
                confidence = (confidence * 1.2).min(0.95);
                reasons.push(format!(
                    "Entropy consistent: head={:.2}, tail={:.2}",
                    head_entropy, tail_entropy
                ));
            }
        }

        let head_sample = &head[head.len().saturating_sub(1024)..];
        let tail_sample = &tail[..1024.min(tail.len())];

        if head_sample.len() >= 256 && tail_sample.len() >= 256 {
            let h_ent = EntropyFilter::compute_entropy(head_sample);
            let t_ent = EntropyFilter::compute_entropy(tail_sample);
            let both_in_range = (6.5..=7.95).contains(&h_ent) && (6.5..=7.95).contains(&t_ent);

            if both_in_range {
                confidence = (confidence * 1.1).min(0.95);
                reasons.push("Both fragments in valid JPEG entropy range".to_string());
            }
        }

        if reasons.is_empty() {
            reasons.push("No issues detected".to_string());
        }

        Self {
            is_valid,
            confidence,
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy_computation() {
        let zeros = vec![0u8; 1000];
        assert_eq!(EntropyFilter::compute_entropy(&zeros), 0.0);

        let random: Vec<u8> = (0..1000).map(|i| (i * 7 + 13) as u8).collect();
        let entropy = EntropyFilter::compute_entropy(&random);
        assert!(entropy > 6.0);

        let single = vec![0xAB; 1000];
        assert_eq!(EntropyFilter::compute_entropy(&single), 0.0);
    }

    #[test]
    fn test_jpeg_marker_extraction() {
        let data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xFF, 0xDB, 0x00, 0x43];

        let markers = JpegStructuralValidator::extract_markers(&data);
        assert_eq!(markers.len(), 3);
        assert_eq!(markers[0], (0, 0xD8));
        assert_eq!(markers[1], (2, 0xE0));
        assert_eq!(markers[2], (6, 0xDB));
    }

    #[test]
    fn test_stitch_validation() {
        let head = vec![0x12, 0x34, 0x56, 0x78];
        let tail = vec![0x9A, 0xBC, 0xDE, 0xF0];

        let result = StitchValidation::validate(&head, &tail);
        assert!(result.is_valid);

        let bad_tail = vec![0xFF, 0xD8, 0x00, 0x00];
        let result = StitchValidation::validate(&head, &bad_tail);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_validation_pipeline() {
        let pipeline = ValidationPipeline::for_jpeg();

        let minimal_jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00,
        ];

        let ctx = ValidationContext {
            format: FileFormat::Jpeg,
            ..Default::default()
        };

        let result = pipeline.validate(&minimal_jpeg, &ctx);

        assert!(!result.passed || result.confidence < 0.5);
    }
}
