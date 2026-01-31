use std::ops::Range;

pub type Offset = u64;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentKind {
    JpegHeader = 0,
    JpegFooter = 1,
    #[allow(dead_code)]
    JpegOrphan = 2,
    PngHeader = 3,
    #[allow(dead_code)]
    PngIdat = 4,
    PngIend = 5,
    #[allow(dead_code)]
    BadSector = 255,
}

impl Default for FragmentKind {
    fn default() -> Self {
        Self::JpegOrphan
    }
}

#[repr(C, align(32))]
#[derive(Debug, Clone, Copy)]
pub struct Fragment {
    pub offset: Offset,
    pub size: u32,
    pub kind: FragmentKind,
    pub entropy: f32,
    _padding: [u8; 11],
}

impl Default for Fragment {
    fn default() -> Self {
        Self {
            offset: 0,
            size: 0,
            kind: FragmentKind::default(),
            entropy: 0.0,
            _padding: [0; 11],
        }
    }
}

impl Fragment {
    pub fn new(offset: Offset, size: u32, kind: FragmentKind, entropy: f32) -> Self {
        Self {
            offset,
            size,
            kind,
            entropy,
            _padding: [0; 11],
        }
    }
}

#[derive(Debug)]
pub struct RecoveredFile {
    pub fragments: Vec<Range<Offset>>,
    pub method: RecoveryMethod,
    pub format: ImageFormat,
}

impl RecoveredFile {
    pub fn new(fragments: Vec<Range<Offset>>, method: RecoveryMethod, format: ImageFormat) -> Self {
        Self {
            fragments,
            method,
            format,
        }
    }

    #[allow(dead_code)]
    pub fn total_size(&self) -> u64 {
        self.fragments.iter().map(|r| r.end - r.start).sum()
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMethod {
    Linear,
    Bifragment,
    #[allow(dead_code)]
    Graph,
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
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    #[inline]
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments.iter()
    }

    #[inline]
    #[allow(dead_code)]
    pub fn jpeg_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::JpegHeader)
    }

    #[inline]
    #[allow(dead_code)]
    pub fn jpeg_footers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::JpegFooter)
    }

    #[inline]
    #[allow(dead_code)]
    pub fn png_headers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::PngHeader)
    }

    #[inline]
    #[allow(dead_code)]
    pub fn png_footers(&self) -> impl Iterator<Item = &Fragment> {
        self.fragments
            .iter()
            .filter(|f| f.kind == FragmentKind::PngIend)
    }

    pub fn sort_by_offset(&mut self) {
        self.fragments.sort_unstable_by_key(|f| f.offset);
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
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;
        const TB: u64 = GB * 1024;

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
