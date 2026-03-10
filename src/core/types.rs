use std::ops::Range;

pub type Offset = u64;

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const GB: u64 = MB * 1024;
const TB: u64 = GB * 1024;

const AVG_IMAGE_SIZE: u64 = 3 * MB;
const FRAGMENT_BUDGET_BYTES: usize = 256 * 1024 * 1024;
const MAX_FRAGMENT_CAPACITY: usize = FRAGMENT_BUDGET_BYTES / std::mem::size_of::<Fragment>();

pub const ICON_MAX_DIMENSION: u32 = 256;
pub const FAVICON_MAX_DIMENSION: u32 = 64;
pub const MIN_PHOTO_BYTES: u64 = 50 * KB;
pub const LOW_ENTROPY_THRESHOLD: f32 = 5.5;
pub const MIN_SCAN_DATA_ENTROPY: f32 = 6.5;
pub const EXTREME_ASPECT_RATIO: u32 = 5;
pub const CORRUPT_SECTOR_RATIO: usize = 4;
pub const VALIDATION_HEADER_SIZE: usize = 16;
pub const SMALL_BUFFER_SIZE: usize = 64 * 1024;
pub const MAX_CHAIN_DEPTH: u8 = 4;
pub const BREAK_DETECTION_READ_SIZE: usize = 256 * 1024;
pub const CONTINUATION_SCAN_CLUSTER_SIZE: u64 = 4096;
pub const CONTINUATION_MATCH_WINDOW: usize = 64;
pub const MIN_FRAGMENT_SIZE: u64 = 512;
pub const MAX_CONTINUATION_CANDIDATES: usize = 16;
pub const REASSEMBLY_MAX_GAP: u64 = 100 * MB;
pub const FINGERPRINT_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionVerdict {
    Photo,
    Asset,
    TooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceTier {
    High,
    Partial,
    Low,
}

impl ConfidenceTier {
    pub fn from_score(score: u8) -> Self {
        if score >= 60 {
            ConfidenceTier::High
        } else if score >= 30 {
            ConfidenceTier::Partial
        } else {
            ConfidenceTier::Low
        }
    }

    pub fn dirname(&self) -> &'static str {
        match self {
            ConfidenceTier::High => "high",
            ConfidenceTier::Partial => "partial",
            ConfidenceTier::Low => "low",
        }
    }
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
}

#[derive(Debug)]
pub enum FragmentRanges {
    Linear(Range<Offset>),
    Multi(Vec<Range<Offset>>),
}

impl FragmentRanges {
    pub fn as_slice(&self) -> &[Range<Offset>] {
        match self {
            FragmentRanges::Linear(r) => std::slice::from_ref(r),
            FragmentRanges::Multi(v) => v,
        }
    }

    pub fn start_offset(&self) -> Offset {
        match self {
            FragmentRanges::Linear(r) => r.start,
            FragmentRanges::Multi(v) => v[0].start,
        }
    }

    pub fn fragment_count(&self) -> usize {
        match self {
            FragmentRanges::Linear(_) => 1,
            FragmentRanges::Multi(v) => v.len(),
        }
    }
}

#[derive(Debug)]
pub struct RecoveredFile {
    pub fragments: FragmentRanges,
    pub method: RecoveryMethod,
    pub format: ImageFormat,
    pub header_entropy: f32,
    pub confidence: u8,
}

impl RecoveredFile {
    pub fn new(
        fragments: FragmentRanges,
        method: RecoveryMethod,
        format: ImageFormat,
        header_entropy: f32,
        confidence: u8,
    ) -> Self {
        Self {
            fragments,
            method,
            format,
            header_entropy,
            confidence,
        }
    }

    pub fn header_offset(&self) -> Offset {
        self.fragments.start_offset()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMethod {
    Linear,
    Reassembled { depth: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakConfidence {
    Definite,
    Probable,
}

#[derive(Debug, Clone)]
pub struct BreakPoint {
    pub break_offset: Offset,
    pub confidence: BreakConfidence,
    pub signature: ContinuationSignature,

    pub last_rst_index: Option<u8>,
}

#[derive(Debug, Clone)]
pub enum ContinuationSignature {
    JpegScanData,
    PngIdat,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct JpegMetadata {
    pub has_exif: bool,
    pub has_icc_profile: bool,
    pub has_jfif: bool,
    pub quantization_quality: QuantizationQuality,
    pub marker_count: u16,
    pub has_sos: bool,
    pub scan_data_entropy: f32,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuantizationQuality {
    High,
    Medium,
    Low,
    #[default]
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

pub struct ExtractionResult {
    pub zero_filled_sectors: usize,
    pub total_sectors: usize,
    pub head: [u8; VALIDATION_HEADER_SIZE],
    pub tail: [u8; VALIDATION_HEADER_SIZE],
    pub bytes_written: usize,
}

#[derive(Debug)]
pub enum ExtractionError {
    DiskFull,
    DeviceDisconnected,
    Io(std::io::Error),
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractionError::DiskFull => write!(f, "Destination disk is full"),
            ExtractionError::DeviceDisconnected => {
                write!(f, "Output device disconnected or I/O failure")
            }
            ExtractionError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl From<std::io::Error> for ExtractionError {
    fn from(e: std::io::Error) -> Self {
        #[cfg(unix)]
        {
            if e.raw_os_error() == Some(libc::ENOSPC) {
                return ExtractionError::DiskFull;
            }
            if e.raw_os_error() == Some(libc::EIO) || e.raw_os_error() == Some(libc::ENXIO) {
                return ExtractionError::DeviceDisconnected;
            }
        }
        if e.kind() == std::io::ErrorKind::WriteZero {
            return ExtractionError::DiskFull;
        }
        ExtractionError::Io(e)
    }
}

impl ExtractionError {
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            ExtractionError::DiskFull | ExtractionError::DeviceDisconnected
        )
    }
}

pub struct ExtractionReport {
    pub extracted: Vec<std::path::PathBuf>,
    pub failed: usize,
    pub corrupt_discarded: usize,
    pub dedup_skipped: usize,
    pub high_confidence: usize,
    pub partial_confidence: usize,
    pub low_confidence: usize,
    pub tail_check_failed: usize,
    pub head_validation_failed: usize,
    pub decode_failed: usize,
    pub halted_reason: Option<String>,
}

impl ExtractionReport {
    pub fn increment_tier(&mut self, score: u8) {
        match ConfidenceTier::from_score(score) {
            ConfidenceTier::High => self.high_confidence += 1,
            ConfidenceTier::Partial => self.partial_confidence += 1,
            ConfidenceTier::Low => self.low_confidence += 1,
        }
    }

    pub fn decrement_tier(&mut self, score: u8) {
        match ConfidenceTier::from_score(score) {
            ConfidenceTier::High => self.high_confidence = self.high_confidence.saturating_sub(1),
            ConfidenceTier::Partial => {
                self.partial_confidence = self.partial_confidence.saturating_sub(1)
            }
            ConfidenceTier::Low => self.low_confidence = self.low_confidence.saturating_sub(1),
        }
    }
}
