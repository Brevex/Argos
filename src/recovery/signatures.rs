use aho_corasick::AhoCorasick;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Jpeg,
    Png,
    Gif,
    Bmp,
    WebP,
    Tiff,
    Unknown,
}

impl FileType {
    pub fn extension(&self) -> &'static str {
        match self {
            FileType::Jpeg => "jpg",
            FileType::Png => "png",
            FileType::Gif => "gif",
            FileType::Bmp => "bmp",
            FileType::WebP => "webp",
            FileType::Tiff => "tiff",
            FileType::Unknown => "bin",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            FileType::Jpeg => "JPEG Image",
            FileType::Png => "PNG Image",
            FileType::Gif => "GIF Image",
            FileType::Bmp => "BMP Image",
            FileType::WebP => "WebP Image",
            FileType::Tiff => "TIFF Image",
            FileType::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, Clone)]
pub struct FileSignature {
    file_type: FileType,
    header: Vec<u8>,
    footer: Option<Vec<u8>>,
    max_size: u64,
}

impl FileSignature {
    pub fn new(
        file_type: FileType,
        header: Vec<u8>,
        footer: Option<Vec<u8>>,
        max_size: u64,
    ) -> Self {
        Self {
            file_type,
            header,
            footer,
            max_size,
        }
    }

    pub fn file_type(&self) -> FileType {
        self.file_type
    }
    pub fn header(&self) -> &[u8] {
        &self.header
    }
    pub fn footer(&self) -> Option<&[u8]> {
        self.footer.as_deref()
    }
    pub fn max_size(&self) -> u64 {
        self.max_size
    }

    pub fn find_footer(&self, data: &[u8]) -> Option<usize> {
        self.footer.as_ref().and_then(|footer| {
            data.windows(footer.len())
                .position(|window| window == footer.as_slice())
                .map(|pos| pos + footer.len())
        })
    }
}

#[derive(Debug, Clone)]
pub struct SignatureMatch {
    file_type: FileType,
    start_offset: u64,
    end_offset: Option<u64>,
    estimated_size: u64,
}

impl SignatureMatch {
    pub fn new(
        file_type: FileType,
        start_offset: u64,
        end_offset: Option<u64>,
        estimated_size: u64,
    ) -> Self {
        Self {
            file_type,
            start_offset,
            end_offset,
            estimated_size,
        }
    }

    pub fn file_type(&self) -> FileType {
        self.file_type
    }
    pub fn start_offset(&self) -> u64 {
        self.start_offset
    }
    pub fn end_offset(&self) -> Option<u64> {
        self.end_offset
    }
    pub fn estimated_size(&self) -> u64 {
        self.estimated_size
    }
}

#[derive(Debug)]
pub struct SignatureRegistry {
    signatures: HashMap<FileType, Vec<FileSignature>>,
    enabled_types: Vec<FileType>,
    pattern_matcher: Option<AhoCorasick>,
    pattern_map: Vec<(FileType, usize)>,
}

impl SignatureRegistry {
    pub fn new() -> Self {
        Self {
            signatures: HashMap::new(),
            enabled_types: Vec::new(),
            pattern_matcher: None,
            pattern_map: Vec::new(),
        }
    }

    pub fn default_images() -> Self {
        let mut registry = Self::new();
        registry.register(FileSignature::new(
            FileType::Jpeg,
            vec![0xFF, 0xD8, 0xFF],
            Some(vec![0xFF, 0xD9]),
            50 * 1024 * 1024,
        ));
        registry.register(FileSignature::new(
            FileType::Png,
            vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            Some(vec![0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]),
            100 * 1024 * 1024,
        ));
        registry.build_pattern_matcher();
        registry
    }

    pub fn register(&mut self, signature: FileSignature) {
        let file_type = signature.file_type();
        if !self.enabled_types.contains(&file_type) {
            self.enabled_types.push(file_type);
        }
        self.signatures
            .entry(file_type)
            .or_default()
            .push(signature);
        self.pattern_matcher = None;
    }

    pub fn enabled_types(&self) -> &[FileType] {
        &self.enabled_types
    }

    pub fn get_signatures(&self, file_type: FileType) -> &[FileSignature] {
        self.signatures
            .get(&file_type)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn build_pattern_matcher(&mut self) {
        let mut patterns: Vec<Vec<u8>> = Vec::new();
        let mut pattern_map: Vec<(FileType, usize)> = Vec::new();

        for file_type in &self.enabled_types {
            if let Some(sigs) = self.signatures.get(file_type) {
                for (idx, sig) in sigs.iter().enumerate() {
                    patterns.push(sig.header().to_vec());
                    pattern_map.push((*file_type, idx));
                }
            }
        }

        if !patterns.is_empty() {
            self.pattern_matcher = AhoCorasick::new(&patterns).ok();
        }
        self.pattern_map = pattern_map;
    }

    pub fn find_all_matches_with_offsets(&self, data: &[u8]) -> Vec<(usize, &FileSignature)> {
        let matcher = match &self.pattern_matcher {
            Some(m) => m,
            None => return Vec::new(),
        };

        let mut results = Vec::new();
        for mat in matcher.find_overlapping_iter(data) {
            let (file_type, sig_idx) = self.pattern_map[mat.pattern().as_usize()];
            if self.enabled_types.contains(&file_type) {
                if let Some(sigs) = self.signatures.get(&file_type) {
                    if let Some(sig) = sigs.get(sig_idx) {
                        results.push((mat.start(), sig));
                    }
                }
            }
        }
        results
    }
}
