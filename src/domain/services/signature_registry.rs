//! Signature registry service
//!
//! Manages the collection of file signatures used for file carving.
//! This service is extensible to support new file types.

use crate::domain::entities::{FileSignature, FileType};
use std::collections::HashMap;

/// Registry of file signatures for file type detection
///
/// This service maintains a collection of known file signatures
/// and provides methods for matching data against these signatures.
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
}

impl SignatureRegistry {
    /// Creates an empty registry
    pub fn new() -> Self {
        Self {
            signatures: HashMap::new(),
            enabled_types: Vec::new(),
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
    }

    /// Finds all signatures that match the given data
    pub fn find_matches(&self, data: &[u8]) -> Vec<&FileSignature> {
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
