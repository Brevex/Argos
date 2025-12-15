//! Signature registry service
//!
//! Manages the collection of file signatures used for file carving.
//! Uses Aho-Corasick algorithm for efficient O(n+m+z) multi-pattern matching.

use crate::domain::entities::{FileSignature, FileType};
use aho_corasick::AhoCorasick;
use std::collections::HashMap;

/// Registry of file signatures for file type detection
///
/// This service maintains a collection of known file signatures
/// and provides methods for matching data against these signatures
/// using the Aho-Corasick algorithm for optimal performance.
///
/// # Example
///
/// ```
/// use argos::domain::services::SignatureRegistry;
/// use argos::domain::entities::FileType;
///
/// let registry = SignatureRegistry::default_images();
/// let jpeg_data = &[0xFF, 0xD8, 0xFF, 0xE0];
/// let matches = registry.find_matches(jpeg_data);
/// assert_eq!(matches.len(), 1);
/// assert_eq!(matches[0].file_type(), FileType::Jpeg);
/// ```
#[derive(Debug)]
pub struct SignatureRegistry {
    signatures: HashMap<FileType, Vec<FileSignature>>,
    enabled_types: Vec<FileType>,
    /// Aho-Corasick automaton for efficient multi-pattern matching
    pattern_matcher: Option<AhoCorasick>,
    /// Maps pattern index to (FileType, signature index within that type)
    pattern_map: Vec<(FileType, usize)>,
}

impl SignatureRegistry {
    /// Creates an empty registry
    pub fn new() -> Self {
        Self {
            signatures: HashMap::new(),
            enabled_types: Vec::new(),
            pattern_matcher: None,
            pattern_map: Vec::new(),
        }
    }

    /// Creates a registry with default image signatures
    pub fn default_images() -> Self {
        let mut registry = Self::new();

        // JPEG signature
        // Header: FF D8 FF (E0, E1, E2, etc.)
        // Footer: FF D9
        registry.register(FileSignature::new(
            FileType::Jpeg,
            vec![0xFF, 0xD8, 0xFF],
            Some(vec![0xFF, 0xD9]),
            50 * 1024 * 1024, // 50 MB max
        ));

        // PNG signature
        // Header: 89 50 4E 47 0D 0A 1A 0A
        // Footer: 49 45 4E 44 AE 42 60 82
        registry.register(FileSignature::new(
            FileType::Png,
            vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            Some(vec![0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]),
            100 * 1024 * 1024, // 100 MB max
        ));

        // GIF signature (GIF87a and GIF89a)
        // Header: 47 49 46 38 (37|39) 61
        // Footer: 00 3B
        registry.register(FileSignature::new(
            FileType::Gif,
            vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61], // GIF89a
            Some(vec![0x00, 0x3B]),
            50 * 1024 * 1024, // 50 MB max
        ));
        registry.register(FileSignature::new(
            FileType::Gif,
            vec![0x47, 0x49, 0x46, 0x38, 0x37, 0x61], // GIF87a
            Some(vec![0x00, 0x3B]),
            50 * 1024 * 1024,
        ));

        // BMP signature
        // Header: 42 4D (BM)
        // No reliable footer, use size from header
        registry.register(FileSignature::new(
            FileType::Bmp,
            vec![0x42, 0x4D],
            None,
            100 * 1024 * 1024, // 100 MB max
        ));

        // WebP signature
        // Header: 52 49 46 46 xx xx xx xx 57 45 42 50 (RIFF....WEBP)
        registry.register(FileSignature::new(
            FileType::WebP,
            vec![0x52, 0x49, 0x46, 0x46],
            None, // WebP uses RIFF container, size in header
            100 * 1024 * 1024,
        ));

        // TIFF signatures (little-endian and big-endian)
        // Little-endian: 49 49 2A 00
        // Big-endian: 4D 4D 00 2A
        registry.register(FileSignature::new(
            FileType::Tiff,
            vec![0x49, 0x49, 0x2A, 0x00],
            None,
            500 * 1024 * 1024, // 500 MB max (TIFFs can be large)
        ));
        registry.register(FileSignature::new(
            FileType::Tiff,
            vec![0x4D, 0x4D, 0x00, 0x2A],
            None,
            500 * 1024 * 1024,
        ));

        // Build the Aho-Corasick automaton
        registry.build_pattern_matcher();

        registry
    }

    /// Registers a new file signature
    pub fn register(&mut self, signature: FileSignature) {
        let file_type = signature.file_type();

        if !self.enabled_types.contains(&file_type) {
            self.enabled_types.push(file_type);
        }

        self.signatures
            .entry(file_type)
            .or_insert_with(Vec::new)
            .push(signature);

        // Invalidate the pattern matcher - needs rebuild
        self.pattern_matcher = None;
    }

    /// Builds the Aho-Corasick automaton from registered signatures
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

    /// Returns all registered signatures for a file type
    pub fn get_signatures(&self, file_type: FileType) -> &[FileSignature] {
        self.signatures
            .get(&file_type)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns all registered signatures
    pub fn all_signatures(&self) -> impl Iterator<Item = &FileSignature> {
        self.signatures.values().flatten()
    }

    /// Returns the enabled file types
    pub fn enabled_types(&self) -> &[FileType] {
        &self.enabled_types
    }

    /// Enables only specific file types
    pub fn filter_types(&mut self, types: &[FileType]) {
        self.enabled_types.retain(|t| types.contains(t));
        // Rebuild pattern matcher with filtered types
        self.build_pattern_matcher();
    }

    /// Finds all signatures that match the given data using Aho-Corasick
    ///
    /// This is O(n + m + z) where:
    /// - n = length of data
    /// - m = total length of all patterns
    /// - z = number of matches
    pub fn find_matches(&self, data: &[u8]) -> Vec<&FileSignature> {
        // Rebuild if needed
        let matcher = match &self.pattern_matcher {
            Some(m) => m,
            None => {
                // Fallback to linear search if no matcher built
                return self.find_matches_linear(data);
            }
        };

        // Use Aho-Corasick to find all matches at position 0
        let mut results = Vec::new();
        
        for mat in matcher.find_overlapping_iter(data) {
            // Only consider matches starting at position 0
            if mat.start() == 0 {
                let (file_type, sig_idx) = self.pattern_map[mat.pattern().as_usize()];
                if self.enabled_types.contains(&file_type) {
                    if let Some(sigs) = self.signatures.get(&file_type) {
                        if let Some(sig) = sigs.get(sig_idx) {
                            results.push(sig);
                        }
                    }
                }
            }
        }

        results
    }

    /// Finds all pattern matches in the data and returns (offset, signature) pairs
    ///
    /// This is the optimized method for scanning - returns all matches at any offset
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

    /// Fallback linear search for when pattern matcher is not built
    fn find_matches_linear(&self, data: &[u8]) -> Vec<&FileSignature> {
        self.signatures
            .iter()
            .filter(|(ft, _)| self.enabled_types.contains(ft))
            .flat_map(|(_, sigs)| sigs.iter())
            .filter(|sig| sig.matches_header(data))
            .collect()
    }

    /// Returns the number of registered signatures
    pub fn signature_count(&self) -> usize {
        self.signatures.values().map(|v| v.len()).sum()
    }

    /// Returns the number of enabled file types
    pub fn type_count(&self) -> usize {
        self.enabled_types.len()
    }
}

impl Default for SignatureRegistry {
    fn default() -> Self {
        Self::default_images()
    }
}

