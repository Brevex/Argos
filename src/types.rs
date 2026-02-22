use std::ops::Range;
use std::sync::LazyLock;

pub type Offset = u64;

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

const AVG_IMAGE_SIZE: u64 = 3 * MB;
const FRAGMENT_BUDGET_BYTES: usize = 256 * 1024 * 1024;
const MAX_FRAGMENT_CAPACITY: usize = FRAGMENT_BUDGET_BYTES / std::mem::size_of::<Fragment>();

pub const ICON_MAX_DIMENSION: u32 = 256;
pub const ASSET_UPPER_DIMENSION: u32 = 512;
pub const MIN_PHOTO_WIDTH: u32 = 640;
pub const MIN_PHOTO_HEIGHT: u32 = 480;
pub const MIN_PHOTO_MEGAPIXELS: f32 = 0.2;
pub const MIN_PHOTO_BYTES: u64 = 50 * KB;
pub const LOW_ENTROPY_THRESHOLD: f32 = 5.5;
pub const MIN_SCAN_DATA_ENTROPY: f32 = 7.0;
pub const EXTREME_ASPECT_RATIO: u32 = 5;
pub const LOW_MARKER_COUNT_THRESHOLD: u16 = 6;
pub const LOW_QUALITY_MAX_DIMENSION: u32 = 1280;
pub const MIN_PNG_CHUNK_VARIETY: u8 = 3;
pub const MIN_PNG_VARIETY_DIMENSION: u32 = 512;
pub const CORRUPT_SECTOR_RATIO: usize = 4;
pub const VALIDATION_HEADER_SIZE: usize = 16;
pub const SMALL_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionVerdict {
    Photo,
    Asset,
    TooSmall,
}

pub fn categorize_dimensions(width: u32, height: u32) -> DimensionVerdict {
    if width == 0 || height == 0 {
        return DimensionVerdict::TooSmall;
    }

    if width <= ICON_MAX_DIMENSION && height <= ICON_MAX_DIMENSION {
        return DimensionVerdict::Asset;
    }

    if width < MIN_PHOTO_WIDTH && height < MIN_PHOTO_HEIGHT {
        return DimensionVerdict::TooSmall;
    }

    let megapixels = (width as f32 * height as f32) / 1_000_000.0;
    if megapixels < MIN_PHOTO_MEGAPIXELS {
        return DimensionVerdict::TooSmall;
    }

    if (width > height && width / height > EXTREME_ASPECT_RATIO)
        || (height > width && height / width > EXTREME_ASPECT_RATIO)
    {
        return DimensionVerdict::Asset;
    }

    DimensionVerdict::Photo
}

pub fn is_metadata_asset_jpeg(width: u32, height: u32, metadata: &JpegMetadata) -> bool {
    width <= ASSET_UPPER_DIMENSION
        && height <= ASSET_UPPER_DIMENSION
        && (width == height || (!metadata.has_exif && !metadata.has_icc_profile))
}

pub fn is_metadata_asset_png(width: u32, height: u32, metadata: &PngMetadata) -> bool {
    if metadata.has_physical_dimensions && metadata.is_screen_resolution {
        return true;
    }
    width <= ASSET_UPPER_DIMENSION && height <= ASSET_UPPER_DIMENSION && width == height
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentKind {
    JpegHeader = 0,
    JpegFooter = 1,
    PngHeader = 2,
    PngIend = 3,
}

#[derive(Debug, Clone, Copy)]
pub struct Fragment {
    pub offset: Offset,
    pub kind: FragmentKind,
    pub entropy: f32,
    pub verdict: DimensionVerdict,
}

impl Fragment {
    pub fn new(offset: Offset, kind: FragmentKind, entropy: f32) -> Self {
        Self {
            offset,
            kind,
            entropy,
            verdict: DimensionVerdict::Photo,
        }
    }

    pub fn with_verdict(
        offset: Offset,
        kind: FragmentKind,
        entropy: f32,
        verdict: DimensionVerdict,
    ) -> Self {
        Self {
            offset,
            kind,
            entropy,
            verdict,
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

    pub fn is_header(&self) -> bool {
        matches!(
            self.kind,
            FragmentKind::JpegHeader | FragmentKind::PngHeader
        )
    }
}

#[derive(Debug)]
pub enum FragmentRanges {
    Linear(Range<Offset>),
    Bifragment([Range<Offset>; 2]),
}

impl FragmentRanges {
    pub fn as_slice(&self) -> &[Range<Offset>] {
        match self {
            FragmentRanges::Linear(r) => std::slice::from_ref(r),
            FragmentRanges::Bifragment(arr) => arr,
        }
    }

    pub fn start_offset(&self) -> Offset {
        match self {
            FragmentRanges::Linear(r) => r.start,
            FragmentRanges::Bifragment(arr) => arr[0].start,
        }
    }
}

#[derive(Debug)]
pub struct RecoveredFile {
    pub fragments: FragmentRanges,
    pub method: RecoveryMethod,
    pub format: ImageFormat,
    pub header_entropy: f32,
}

impl RecoveredFile {
    pub fn new(
        fragments: FragmentRanges,
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

    pub fn header_offset(&self) -> Offset {
        self.fragments.start_offset()
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
    pub has_sos: bool,
    pub scan_data_entropy: f32,
}

impl Default for JpegMetadata {
    fn default() -> Self {
        Self {
            has_exif: false,
            has_icc_profile: false,
            has_jfif: false,
            quantization_quality: QuantizationQuality::Unknown,
            marker_count: 0,
            has_sos: false,
            scan_data_entropy: 0.0,
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
    pub has_icc_profile: bool,
    pub has_physical_dimensions: bool,
    pub is_screen_resolution: bool,
    pub chunk_variety: u8,
}

pub trait FragmentCollector {
    fn collect(&mut self, fragment: Fragment);
}

impl FragmentCollector for Vec<Fragment> {
    #[inline]
    fn collect(&mut self, fragment: Fragment) {
        self.push(fragment);
    }
}

pub struct FragmentMap {
    jpeg_headers: Vec<Fragment>,
    jpeg_footers: Vec<Fragment>,
    png_headers: Vec<Fragment>,
    png_footers: Vec<Fragment>,
}

impl FragmentMap {
    pub fn new() -> Self {
        Self {
            jpeg_headers: Vec::new(),
            jpeg_footers: Vec::new(),
            png_headers: Vec::new(),
            png_footers: Vec::new(),
        }
    }

    pub fn with_disk_estimate(disk_size: u64) -> Self {
        let estimated = ((disk_size / AVG_IMAGE_SIZE) as usize).min(MAX_FRAGMENT_CAPACITY);
        let per_kind = estimated / 4;
        Self {
            jpeg_headers: Vec::with_capacity(per_kind),
            jpeg_footers: Vec::with_capacity(per_kind),
            png_headers: Vec::with_capacity(per_kind),
            png_footers: Vec::with_capacity(per_kind),
        }
    }

    #[inline]
    pub fn push(&mut self, fragment: Fragment) {
        if self.len() < MAX_FRAGMENT_CAPACITY {
            match fragment.kind {
                FragmentKind::JpegHeader => self.jpeg_headers.push(fragment),
                FragmentKind::JpegFooter => self.jpeg_footers.push(fragment),
                FragmentKind::PngHeader => self.png_headers.push(fragment),
                FragmentKind::PngIend => self.png_footers.push(fragment),
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.jpeg_headers.len()
            + self.jpeg_footers.len()
            + self.png_headers.len()
            + self.png_footers.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.jpeg_headers.is_empty()
            && self.jpeg_footers.is_empty()
            && self.png_headers.is_empty()
            && self.png_footers.is_empty()
    }

    #[inline]
    pub fn jpeg_headers(&self) -> &[Fragment] {
        &self.jpeg_headers
    }

    #[inline]
    pub fn jpeg_footers(&self) -> &[Fragment] {
        &self.jpeg_footers
    }

    #[inline]
    pub fn png_headers(&self) -> &[Fragment] {
        &self.png_headers
    }

    #[inline]
    pub fn png_footers(&self) -> &[Fragment] {
        &self.png_footers
    }

    #[inline]
    pub fn viable_jpeg_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.jpeg_headers.iter().filter(|f| f.has_viable_entropy())
    }

    #[inline]
    pub fn viable_png_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.png_headers.iter().filter(|f| f.has_viable_entropy())
    }

    pub fn sort_by_offset(&mut self) {
        self.jpeg_headers.sort_unstable_by_key(|f| f.offset);
        self.jpeg_footers.sort_unstable_by_key(|f| f.offset);
        self.png_headers.sort_unstable_by_key(|f| f.offset);
        self.png_footers.sort_unstable_by_key(|f| f.offset);
    }

    pub fn dedup(&mut self) {
        self.jpeg_headers.dedup_by(|a, b| a.offset == b.offset);
        self.jpeg_footers.dedup_by(|a, b| a.offset == b.offset);
        self.png_headers.dedup_by(|a, b| a.offset == b.offset);
        self.png_footers.dedup_by(|a, b| a.offset == b.offset);
    }

    pub fn count_by_kind(&self) -> FragmentCounts {
        FragmentCounts {
            jpeg_headers: self.jpeg_headers.len(),
            jpeg_footers: self.jpeg_footers.len(),
            png_headers: self.png_headers.len(),
            png_footers: self.png_footers.len(),
        }
    }
}

impl Default for FragmentMap {
    fn default() -> Self {
        Self::new()
    }
}

impl FragmentCollector for FragmentMap {
    #[inline]
    fn collect(&mut self, fragment: Fragment) {
        self.push(fragment);
    }
}

#[derive(Default)]
pub struct FragmentCounts {
    pub jpeg_headers: usize,
    pub jpeg_footers: usize,
    pub png_headers: usize,
    pub png_footers: usize,
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

pub struct ExtractionResult {
    pub zero_filled_sectors: usize,
    pub total_sectors: usize,
    pub head: [u8; VALIDATION_HEADER_SIZE],
    pub tail: [u8; VALIDATION_HEADER_SIZE],
    pub bytes_written: usize,
}

pub struct ExtractionReport {
    pub extracted: Vec<std::path::PathBuf>,
    pub failed: usize,
    pub corrupt_discarded: usize,
}
