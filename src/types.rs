use std::ops::Range;

pub type Offset = u64;

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

const ENTROPY_WEIGHT: f32 = 0.35;
const METADATA_WEIGHT: f32 = 0.25;
const DIMENSION_WEIGHT: f32 = 0.20;
const STRUCTURE_WEIGHT: f32 = 0.20;

const HIGH_ENTROPY_THRESHOLD: f32 = 7.0;
const MEDIUM_ENTROPY_THRESHOLD: f32 = 6.0;
const LOW_ENTROPY_THRESHOLD: f32 = 5.5;

const MINIMUM_QUALITY_SCORE: f32 = 0.3;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentKind {
    JpegHeader = 0,
    JpegFooter = 1,
    PngHeader = 3,
    PngIend = 5,
}

#[repr(C, align(32))]
#[derive(Debug, Clone, Copy)]
pub struct Fragment {
    pub offset: Offset,
    pub kind: FragmentKind,
    pub entropy: f32,
    _padding: [u8; 15],
}

impl Fragment {
    pub fn new(offset: Offset, kind: FragmentKind, entropy: f32) -> Self {
        Self {
            offset,
            kind,
            entropy,
            _padding: [0; 15],
        }
    }

    pub fn has_viable_entropy(&self) -> bool {
        match self.kind {
            FragmentKind::JpegHeader | FragmentKind::PngHeader => {
                self.entropy >= LOW_ENTROPY_THRESHOLD
            }
            _ => true,
        }
    }
}

#[derive(Debug)]
pub struct RecoveredFile {
    pub fragments: Vec<Range<Offset>>,
    pub method: RecoveryMethod,
    pub format: ImageFormat,
    pub header_entropy: f32,
}

impl RecoveredFile {
    pub fn new(
        fragments: Vec<Range<Offset>>,
        method: RecoveryMethod,
        format: ImageFormat,
        header_entropy: f32,
    ) -> Self {
        Self {
            fragments,
            method,
            format,
            header_entropy,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMethod {
    Linear,
    Bifragment,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

impl ImageFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct JpegMetadata {
    pub has_exif: bool,
    pub has_icc_profile: bool,
    pub has_jfif: bool,
    pub quantization_quality: QuantizationQuality,
    pub marker_count: u16,
}

impl Default for JpegMetadata {
    fn default() -> Self {
        Self {
            has_exif: false,
            has_icc_profile: false,
            has_jfif: false,
            quantization_quality: QuantizationQuality::Unknown,
            marker_count: 0,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantizationQuality {
    High,
    Medium,
    Low,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PngMetadata {
    pub has_text_chunks: bool,
    pub has_gamma: bool,
    pub has_chromaticity: bool,
    pub has_icc_profile: bool,
    pub has_physical_dimensions: bool,
    pub is_screen_resolution: bool,
    pub chunk_variety: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageQualityScore {
    pub entropy_score: f32,
    pub metadata_score: f32,
    pub dimension_score: f32,
    pub structure_score: f32,
    pub corruption_penalty: f32,
    pub total: f32,
}

impl ImageQualityScore {
    pub fn for_jpeg(
        entropy: f32,
        width: u16,
        height: u16,
        metadata: &JpegMetadata,
        structure_valid: bool,
    ) -> Self {
        let entropy_score = entropy_to_score(entropy);
        let metadata_score = jpeg_metadata_score(metadata);
        let dimension_score = dimension_to_score(width as u32, height as u32);

        let structure_score = if structure_valid {
            let marker_density = (metadata.marker_count as f32 / 20.0).min(1.0);
            0.5 + marker_density * 0.5
        } else {
            0.0
        };

        let corruption_penalty = if structure_valid { 1.0 } else { 0.3 };

        let total = (entropy_score * ENTROPY_WEIGHT
            + metadata_score * METADATA_WEIGHT
            + dimension_score * DIMENSION_WEIGHT
            + structure_score * STRUCTURE_WEIGHT)
            * corruption_penalty;

        Self {
            entropy_score,
            metadata_score,
            dimension_score,
            structure_score,
            corruption_penalty,
            total,
        }
    }

    pub fn for_png(
        entropy: f32,
        width: u32,
        height: u32,
        metadata: &PngMetadata,
        idat_count: usize,
    ) -> Self {
        let entropy_score = entropy_to_score(entropy);
        let metadata_score = png_metadata_score(metadata);
        let dimension_score = dimension_to_score(width, height);

        let structure_score = {
            let chunk_diversity = (metadata.chunk_variety as f32 / 6.0).min(1.0);
            let idat_density = (idat_count as f32 / 5.0).min(1.0);
            chunk_diversity * 0.6 + idat_density * 0.4
        };

        let corruption_penalty = if idat_count >= 1 { 1.0 } else { 0.0 };

        let total = (entropy_score * ENTROPY_WEIGHT
            + metadata_score * METADATA_WEIGHT
            + dimension_score * DIMENSION_WEIGHT
            + structure_score * STRUCTURE_WEIGHT)
            * corruption_penalty;

        Self {
            entropy_score,
            metadata_score,
            dimension_score,
            structure_score,
            corruption_penalty,
            total,
        }
    }

    pub fn meets_minimum(&self) -> bool {
        self.total >= MINIMUM_QUALITY_SCORE
    }
}

fn entropy_to_score(entropy: f32) -> f32 {
    if entropy >= HIGH_ENTROPY_THRESHOLD {
        1.0
    } else if entropy >= MEDIUM_ENTROPY_THRESHOLD {
        0.5 + (entropy - MEDIUM_ENTROPY_THRESHOLD)
            / (HIGH_ENTROPY_THRESHOLD - MEDIUM_ENTROPY_THRESHOLD)
            * 0.5
    } else if entropy >= LOW_ENTROPY_THRESHOLD {
        0.2 + (entropy - LOW_ENTROPY_THRESHOLD) / (MEDIUM_ENTROPY_THRESHOLD - LOW_ENTROPY_THRESHOLD)
            * 0.3
    } else {
        (entropy / LOW_ENTROPY_THRESHOLD * 0.2).max(0.0)
    }
}

fn jpeg_metadata_score(metadata: &JpegMetadata) -> f32 {
    let mut score = 0.0f32;

    if metadata.has_exif {
        score += 0.45;
    }
    if metadata.has_icc_profile {
        score += 0.25;
    }
    if metadata.has_jfif && !metadata.has_exif {
        score += 0.1;
    }

    score += match metadata.quantization_quality {
        QuantizationQuality::High => 0.3,
        QuantizationQuality::Medium => 0.15,
        QuantizationQuality::Low => 0.0,
        QuantizationQuality::Unknown => 0.05,
    };

    score.min(1.0)
}

fn png_metadata_score(metadata: &PngMetadata) -> f32 {
    let mut score = 0.0f32;

    if metadata.has_icc_profile {
        score += 0.3;
    }
    if metadata.has_gamma {
        score += 0.15;
    }
    if metadata.has_chromaticity {
        score += 0.15;
    }
    if metadata.has_text_chunks {
        score += 0.1;
    }

    if metadata.has_physical_dimensions && metadata.is_screen_resolution {
        score -= 0.2;
    }

    score.clamp(0.0, 1.0)
}

fn dimension_to_score(width: u32, height: u32) -> f32 {
    let pixels = width as u64 * height as u64;
    let megapixels = pixels as f32 / 1_000_000.0;

    if megapixels >= 2.0 {
        1.0
    } else if megapixels >= 0.5 {
        0.5 + (megapixels - 0.5) / 1.5 * 0.5
    } else if megapixels >= 0.1 {
        0.2 + (megapixels - 0.1) / 0.4 * 0.3
    } else {
        (megapixels / 0.1 * 0.2).max(0.0)
    }
}

pub struct FragmentMap {
    pub fragments: Vec<Fragment>,
}

impl FragmentMap {
    pub fn new() -> Self {
        Self {
            fragments: Vec::with_capacity(1_000_000),
        }
    }

    #[inline]
    pub fn push(&mut self, fragment: Fragment) {
        self.fragments.push(fragment);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments.iter()
    }

    #[inline]
    pub fn jpeg_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::JpegHeader)
    }

    #[inline]
    pub fn jpeg_footers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::JpegFooter)
    }

    #[inline]
    pub fn png_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::PngHeader)
    }

    #[inline]
    pub fn png_footers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::PngIend)
    }

    #[inline]
    pub fn viable_jpeg_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.jpeg_headers().filter(|f| f.has_viable_entropy())
    }

    #[inline]
    pub fn viable_png_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.png_headers().filter(|f| f.has_viable_entropy())
    }

    pub fn sort_by_offset(&mut self) {
        self.fragments.sort_unstable_by_key(|f| f.offset);
    }

    pub fn dedup(&mut self) {
        self.fragments
            .dedup_by(|a, b| a.offset == b.offset && a.kind == b.kind);
    }
}

impl Default for FragmentMap {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct BlockDevice {
    pub name: String,
    pub device_type: DeviceType,
    pub size: u64,
    pub path: String,
}

impl BlockDevice {
    pub fn size_human(&self) -> String {
        if self.size >= TB {
            format!("{:.2} TB", self.size as f64 / TB as f64)
        } else if self.size >= GB {
            format!("{:.2} GB", self.size as f64 / GB as f64)
        } else if self.size >= MB {
            format!("{:.2} MB", self.size as f64 / MB as f64)
        } else if self.size >= KB {
            format!("{:.2} KB", self.size as f64 / KB as f64)
        } else {
            format!("{} B", self.size)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Hdd,
    Ssd,
    NVMe,
    Usb,
    Unknown,
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::Hdd => write!(f, "HDD"),
            DeviceType::Ssd => write!(f, "SSD"),
            DeviceType::NVMe => write!(f, "NVMe"),
            DeviceType::Usb => write!(f, "USB"),
            DeviceType::Unknown => write!(f, "Unknown"),
        }
    }
}

pub fn calculate_entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    for &byte in data {
        freq[byte as usize] += 1;
    }
    let len = data.len() as f32;
    -freq
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f32 / len;
            p * p.log2()
        })
        .sum::<f32>()
}
