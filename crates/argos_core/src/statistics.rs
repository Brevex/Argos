use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct ImageStatistics {
    pub entropy: f64,
    pub local_entropy_variance: f64,
    pub kurtosis: f64,
    pub edge_density: f64,
    pub color_diversity: f64,
    pub horizontal_discontinuity: f64,
    pub distinct_colors: usize,
    pub mean_value: f64,
    pub std_deviation: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageClassification {
    NaturalPhoto,
    ArtificialGraphic,
    Corrupted,
    Encrypted,
    TooSmall,
}

impl ImageClassification {
    #[inline]
    pub fn is_photo(self) -> bool {
        matches!(self, Self::NaturalPhoto)
    }
}

#[derive(Debug, Clone)]
pub struct ClassifierConfig {
    pub min_photo_entropy: f64,
    pub max_valid_entropy: f64,
    pub min_photo_kurtosis: f64,
    pub max_local_variance: f64,
    pub min_color_diversity: f64,
    pub max_discontinuity: f64,
    pub min_pixels: usize,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            min_photo_entropy: 5.5,
            max_valid_entropy: 7.99,
            min_photo_kurtosis: -1.0,
            max_local_variance: 2.0,
            min_color_diversity: 0.001,
            max_discontinuity: 0.3,
            min_pixels: 10000,
        }
    }
}

pub struct ImageClassifier {
    config: ClassifierConfig,
}

impl ImageClassifier {
    pub fn new() -> Self {
        Self {
            config: ClassifierConfig::default(),
        }
    }

    pub fn with_config(config: ClassifierConfig) -> Self {
        Self { config }
    }

    pub fn compute_statistics(
        &self,
        data: &[u8],
        width: usize,
        height: usize,
        channels: usize,
    ) -> ImageStatistics {
        let total_pixels = width * height;
        if data.is_empty() || total_pixels == 0 {
            return ImageStatistics::default();
        }

        let gray_values = to_grayscale(data, channels);
        let (mean, std_dev) = compute_mean_std(&gray_values);
        let entropy = compute_entropy(&gray_values);
        let kurtosis = compute_kurtosis(&gray_values, mean, std_dev);
        let local_entropy_variance = compute_local_entropy_variance(&gray_values, width, height);
        let edge_density = compute_edge_density(&gray_values, width, height);
        let horizontal_discontinuity =
            compute_horizontal_discontinuity(&gray_values, width, height);
        let (color_diversity, distinct_colors) = if channels >= 3 {
            compute_color_diversity(data, channels)
        } else {
            let unique: HashSet<_> = gray_values.iter().collect();
            (unique.len() as f64 / 256.0, unique.len())
        };

        ImageStatistics {
            entropy,
            local_entropy_variance,
            kurtosis,
            edge_density,
            color_diversity,
            horizontal_discontinuity,
            distinct_colors,
            mean_value: mean,
            std_deviation: std_dev,
        }
    }

    pub fn classify(&self, stats: &ImageStatistics, total_pixels: usize) -> ImageClassification {
        if total_pixels < self.config.min_pixels {
            return ImageClassification::TooSmall;
        }
        if stats.entropy > self.config.max_valid_entropy && stats.local_entropy_variance < 0.1 {
            return ImageClassification::Encrypted;
        }
        if stats.horizontal_discontinuity > self.config.max_discontinuity {
            return ImageClassification::Corrupted;
        }
        if (stats.entropy < self.config.min_photo_entropy
            || stats.color_diversity < self.config.min_color_diversity)
            && stats.kurtosis < self.config.min_photo_kurtosis
        {
            return ImageClassification::ArtificialGraphic;
        }
        if stats.local_entropy_variance < 0.5 && stats.entropy < 6.0 {
            return ImageClassification::ArtificialGraphic;
        }
        if stats.entropy >= self.config.min_photo_entropy
            && stats.kurtosis >= self.config.min_photo_kurtosis
            && stats.color_diversity >= self.config.min_color_diversity
        {
            return ImageClassification::NaturalPhoto;
        }
        ImageClassification::ArtificialGraphic
    }
}

impl Default for ImageClassifier {
    fn default() -> Self {
        Self::new()
    }
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

pub fn compute_mean_std(data: &[u8]) -> (f64, f64) {
    if data.is_empty() {
        return (0.0, 0.0);
    }
    let sum: u64 = data.iter().map(|&x| x as u64).sum();
    let mean = sum as f64 / data.len() as f64;
    let variance = data.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / data.len() as f64;
    (mean, variance.sqrt())
}

pub fn compute_kurtosis(data: &[u8], mean: f64, std_dev: f64) -> f64 {
    if data.is_empty() || std_dev == 0.0 {
        return 0.0;
    }
    let n = data.len() as f64;
    let fourth_moment = data
        .iter()
        .map(|&x| ((x as f64 - mean) / std_dev).powi(4))
        .sum::<f64>()
        / n;
    fourth_moment - 3.0
}

fn to_grayscale(data: &[u8], channels: usize) -> Vec<u8> {
    match channels {
        1 => data.to_vec(),
        3 => data
            .chunks_exact(3)
            .map(|rgb| {
                ((rgb[0] as u32 * 299 + rgb[1] as u32 * 587 + rgb[2] as u32 * 114) / 1000) as u8
            })
            .collect(),
        4 => data
            .chunks_exact(4)
            .map(|rgba| {
                ((rgba[0] as u32 * 299 + rgba[1] as u32 * 587 + rgba[2] as u32 * 114) / 1000) as u8
            })
            .collect(),
        _ => data.to_vec(),
    }
}

fn compute_local_entropy_variance(data: &[u8], width: usize, height: usize) -> f64 {
    const GRID_SIZE: usize = 4;
    if width < GRID_SIZE || height < GRID_SIZE {
        return 0.0;
    }
    let cell_w = width / GRID_SIZE;
    let cell_h = height / GRID_SIZE;
    let mut local_entropies = Vec::with_capacity(GRID_SIZE * GRID_SIZE);

    for gy in 0..GRID_SIZE {
        for gx in 0..GRID_SIZE {
            let mut cell_data = Vec::with_capacity(cell_w * cell_h);
            for y in 0..cell_h {
                let row_start = (gy * cell_h + y) * width + gx * cell_w;
                let row_end = row_start + cell_w;
                if row_end <= data.len() {
                    cell_data.extend_from_slice(&data[row_start..row_end]);
                }
            }
            if !cell_data.is_empty() {
                local_entropies.push(compute_entropy(&cell_data));
            }
        }
    }
    if local_entropies.is_empty() {
        return 0.0;
    }
    let mean = local_entropies.iter().sum::<f64>() / local_entropies.len() as f64;
    local_entropies
        .iter()
        .map(|e| (e - mean).powi(2))
        .sum::<f64>()
        / local_entropies.len() as f64
}

fn compute_edge_density(data: &[u8], width: usize, height: usize) -> f64 {
    if width < 3 || height < 3 || data.len() < width * height {
        return 0.0;
    }
    let mut edge_count = 0usize;
    let threshold = 30u8;

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = y * width + x;
            let gx = (data[idx + 1] as i16 - data[idx - 1] as i16).abs();
            let gy = (data[idx + width] as i16 - data[idx - width] as i16).abs();
            let gradient = ((gx * gx + gy * gy) as f64).sqrt() as u8;
            if gradient > threshold {
                edge_count += 1;
            }
        }
    }
    edge_count as f64 / ((width - 2) * (height - 2)) as f64
}

fn compute_horizontal_discontinuity(data: &[u8], width: usize, height: usize) -> f64 {
    if width < 2 || height < 2 || data.len() < width * height {
        return 0.0;
    }
    let mut row_gradients = Vec::with_capacity(height);

    for y in 0..height {
        let row_start = y * width;
        if row_start + width > data.len() {
            break;
        }
        let mut row_gradient = 0i64;
        for x in 0..width - 1 {
            row_gradient += (data[row_start + x + 1] as i64 - data[row_start + x] as i64).abs();
        }
        row_gradients.push(row_gradient as f64 / (width - 1) as f64);
    }
    if row_gradients.len() < 2 {
        return 0.0;
    }
    let mut max_jump = 0.0f64;
    for i in 1..row_gradients.len() {
        let jump = (row_gradients[i] - row_gradients[i - 1]).abs();
        max_jump = max_jump.max(jump);
    }
    let avg_gradient: f64 = row_gradients.iter().sum::<f64>() / row_gradients.len() as f64;
    if avg_gradient > 0.0 {
        max_jump / avg_gradient
    } else {
        0.0
    }
}

fn compute_color_diversity(data: &[u8], channels: usize) -> (f64, usize) {
    let mut color_seen = [0u64; 512];

    let total_pixels = data.len() / channels;
    let sample_step = if total_pixels > 100000 {
        total_pixels / 100000
    } else {
        1
    };

    for i in (0..data.len()).step_by(channels * sample_step) {
        if i + 2 < data.len() {
            let r = (data[i] >> 3) as usize;
            let g = (data[i + 1] >> 3) as usize;
            let b = (data[i + 2] >> 3) as usize;
            let index = (r << 10) | (g << 5) | b;
            color_seen[index / 64] |= 1 << (index % 64);
        }
    }

    let count: usize = color_seen.iter().map(|x| x.count_ones() as usize).sum();
    (count as f64 / 32768.0, count)
}

pub fn compute_entropy_delta(data: &[u8], window_size: usize) -> Vec<f64> {
    if data.len() < window_size * 2 {
        return Vec::new();
    }
    let mut deltas = Vec::with_capacity(data.len() / window_size);
    let mut prev_entropy = compute_entropy(&data[..window_size]);
    let mut offset = window_size;
    while offset + window_size <= data.len() {
        let current_entropy = compute_entropy(&data[offset..offset + window_size]);
        deltas.push(prev_entropy - current_entropy);
        prev_entropy = current_entropy;
        offset += window_size;
    }
    deltas
}

pub fn detect_entropy_boundary(data: &[u8], window_size: usize, threshold: f64) -> Option<usize> {
    let deltas = compute_entropy_delta(data, window_size);
    for (i, delta) in deltas.iter().enumerate() {
        if *delta > threshold {
            return Some((i + 1) * window_size);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy_uniform() {
        let data = vec![128u8; 1000];
        let entropy = compute_entropy(&data);
        assert!(entropy < 0.01);
    }

    #[test]
    fn test_entropy_random() {
        let data: Vec<u8> = (0..1000).map(|i| ((i * 17 + 31) % 256) as u8).collect();
        let entropy = compute_entropy(&data);
        assert!(entropy > 5.0);
    }

    #[test]
    fn test_mean_std() {
        let data = vec![0, 50, 100, 150, 200];
        let (mean, std) = compute_mean_std(&data);
        assert!((mean - 100.0).abs() < 0.1);
        assert!(std > 0.0);
    }

    #[test]
    fn test_classification_too_small() {
        let classifier = ImageClassifier::new();
        let stats = ImageStatistics::default();
        let result = classifier.classify(&stats, 100);
        assert_eq!(result, ImageClassification::TooSmall);
    }

    #[test]
    fn test_classification_encrypted() {
        let classifier = ImageClassifier::new();
        let stats = ImageStatistics {
            entropy: 7.999,
            local_entropy_variance: 0.01,
            ..Default::default()
        };
        let result = classifier.classify(&stats, 100000);
        assert_eq!(result, ImageClassification::Encrypted);
    }
}
