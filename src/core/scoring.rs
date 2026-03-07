use std::sync::LazyLock;

use super::types::{
    DimensionVerdict, JpegMetadata, PngMetadata, QuantizationQuality, EXTREME_ASPECT_RATIO,
    FAVICON_MAX_DIMENSION, ICON_MAX_DIMENSION, MIN_SCAN_DATA_ENTROPY,
};

pub fn categorize_dimensions(width: u32, height: u32) -> DimensionVerdict {
    if width == 0 || height == 0 {
        return DimensionVerdict::TooSmall;
    }

    if width <= FAVICON_MAX_DIMENSION && height <= FAVICON_MAX_DIMENSION {
        return DimensionVerdict::TooSmall;
    }

    if width <= ICON_MAX_DIMENSION && height <= ICON_MAX_DIMENSION {
        return DimensionVerdict::Asset;
    }

    if (width > height && width / height > EXTREME_ASPECT_RATIO)
        || (height > width && height / width > EXTREME_ASPECT_RATIO)
    {
        return DimensionVerdict::Asset;
    }

    DimensionVerdict::Photo
}

pub fn score_jpeg(width: u16, height: u16, metadata: &JpegMetadata) -> u8 {
    let w = width as u32;
    let h = height as u32;
    if w == 0 || h == 0 || (w <= FAVICON_MAX_DIMENSION && h <= FAVICON_MAX_DIMENSION) {
        return 0;
    }

    let mut score: i32 = 0;

    let max_dim = w.max(h);
    score += if w <= 128 && h <= 128 {
        5
    } else if w <= 256 && h <= 256 {
        10
    } else if max_dim <= 512 {
        15
    } else if max_dim <= 768 {
        20
    } else if max_dim <= 1280 {
        25
    } else {
        30
    };

    if metadata.has_exif {
        score += 20;
    }
    if metadata.has_icc_profile {
        score += 10;
    }
    if metadata.has_jfif {
        score += 5;
    }

    match metadata.quantization_quality {
        QuantizationQuality::High => score += 15,
        QuantizationQuality::Medium => score += 5,
        QuantizationQuality::Low => score -= 10,
        QuantizationQuality::Unknown => {}
    }

    score += metadata.marker_count.min(10) as i32;

    if metadata.has_sos && metadata.scan_data_entropy > 0.0 {
        if metadata.scan_data_entropy >= 7.0 {
            score += 10;
        } else if metadata.scan_data_entropy >= MIN_SCAN_DATA_ENTROPY {
            score += 5;
        } else {
            score -= 15;
        }
    }

    let is_standard_thumb = matches!(
        (w, h),
        (128, 128) | (160, 120) | (256, 256) | (96, 96) | (120, 120)
    );
    if is_standard_thumb
        && metadata.quantization_quality == QuantizationQuality::Low
        && !metadata.has_exif
    {
        score = score.min(15);
    }

    if (w > h && h > 0 && w / h > EXTREME_ASPECT_RATIO)
        || (h > w && w > 0 && h / w > EXTREME_ASPECT_RATIO)
    {
        score -= 15;
    }

    score.clamp(1, 100) as u8
}

pub fn score_png(width: u32, height: u32, metadata: &PngMetadata, idat_count: usize) -> u8 {
    if width == 0 || height == 0 || idat_count == 0 {
        return 0;
    }
    if width <= FAVICON_MAX_DIMENSION && height <= FAVICON_MAX_DIMENSION {
        return 0;
    }

    let mut score: i32 = 0;

    let max_dim = width.max(height);
    score += if width <= 128 && height <= 128 {
        5
    } else if width <= 256 && height <= 256 {
        10
    } else if max_dim <= 512 {
        15
    } else if max_dim <= 768 {
        20
    } else if max_dim <= 1280 {
        25
    } else {
        30
    };

    if metadata.has_text_chunks {
        score += 10;
    }
    if metadata.has_icc_profile {
        score += 10;
    }
    if metadata.has_physical_dimensions && !metadata.is_screen_resolution {
        score += 5;
    }

    if metadata.is_screen_resolution {
        score -= 10;
    }

    score += (metadata.chunk_variety.min(6) as i32) * 3;

    if (width > height && height > 0 && width / height > EXTREME_ASPECT_RATIO)
        || (height > width && width > 0 && height / width > EXTREME_ASPECT_RATIO)
    {
        score -= 15;
    }

    if width == height
        && width <= ICON_MAX_DIMENSION
        && !metadata.has_text_chunks
        && !metadata.has_icc_profile
    {
        score = score.min(15);
    }

    score.clamp(1, 100) as u8
}

const ENTROPY_LUT_SIZE: usize = 4097;

static ENTROPY_LUT: LazyLock<[f32; ENTROPY_LUT_SIZE]> = LazyLock::new(|| {
    let mut lut = [0.0f32; ENTROPY_LUT_SIZE];
    for (c, entry) in lut.iter_mut().enumerate().skip(1) {
        let cf = c as f32;
        *entry = cf * cf.log2();
    }
    lut
});

pub fn calculate_entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    for &byte in data {
        freq[byte as usize] += 1;
    }
    let n = data.len();
    let log2_n = (n as f32).log2();
    let lut = &*ENTROPY_LUT;
    let sum: f32 = freq
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let idx = c as usize;
            if idx < ENTROPY_LUT_SIZE {
                lut[idx]
            } else {
                let cf = c as f32;
                cf * cf.log2()
            }
        })
        .sum();
    log2_n - sum / n as f32
}
