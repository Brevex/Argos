use crate::traits::FileScanner;
use crate::types::FileType;
use memchr::memmem::Finder;

#[derive(Debug, Clone)]
pub struct JpegScanner {
    header_finder: Finder<'static>,
    footer_finder: Finder<'static>,
}

impl JpegScanner {
    const HEADER: &'static [u8] = &[0xFF, 0xD8, 0xFF];
    const FOOTER: &'static [u8] = &[0xFF, 0xD9];

    #[must_use]
    pub fn new() -> Self {
        Self {
            header_finder: Finder::new(Self::HEADER),
            footer_finder: Finder::new(Self::FOOTER),
        }
    }
}

impl Default for JpegScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl FileScanner for JpegScanner {
    fn scan_headers(&self, buffer: &[u8]) -> Vec<usize> {
        self.header_finder.find_iter(buffer).collect()
    }

    fn scan_footers(&self, buffer: &[u8]) -> Vec<usize> {
        self.footer_finder.find_iter(buffer).collect()
    }

    fn file_type(&self) -> FileType {
        FileType::Jpeg
    }

    #[inline]
    fn scan_headers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.header_finder.find_iter(buffer) {
            callback(offset);
        }
    }

    #[inline]
    fn scan_footers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.footer_finder.find_iter(buffer) {
            callback(offset);
        }
    }
}

#[derive(Debug, Clone)]
pub struct PngScanner {
    header_finder: Finder<'static>,
    footer_finder: Finder<'static>,
}

impl PngScanner {
    const HEADER: &'static [u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const FOOTER: &'static [u8] = &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];

    #[must_use]
    pub fn new() -> Self {
        Self {
            header_finder: Finder::new(Self::HEADER),
            footer_finder: Finder::new(Self::FOOTER),
        }
    }
}

impl Default for PngScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl FileScanner for PngScanner {
    fn scan_headers(&self, buffer: &[u8]) -> Vec<usize> {
        self.header_finder.find_iter(buffer).collect()
    }

    fn scan_footers(&self, buffer: &[u8]) -> Vec<usize> {
        self.footer_finder.find_iter(buffer).collect()
    }

    fn file_type(&self) -> FileType {
        FileType::Png
    }

    #[inline]
    fn scan_headers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.header_finder.find_iter(buffer) {
            callback(offset);
        }
    }

    #[inline]
    fn scan_footers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.footer_finder.find_iter(buffer) {
            callback(offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_scan_headers_single_match() {
        let scanner = JpegScanner::new();

        let buffer: Vec<u8> = [
            &[0x00, 0x11, 0x22, 0x33, 0x44][..],
            &[0xFF, 0xD8, 0xFF][..],
            &[0xE0, 0x00, 0x10][..],
            &[0xAA, 0xBB, 0xCC][..],
        ]
        .concat();

        let headers = scanner.scan_headers(&buffer);
        assert_eq!(headers, vec![5]);
    }

    #[test]
    fn jpeg_scan_footers_single_match() {
        let scanner = JpegScanner::new();

        let buffer: Vec<u8> = [
            &[0x00, 0x11, 0x22, 0x33][..],
            &[0xFF, 0xD9][..],
            &[0xAA, 0xBB, 0xCC][..],
        ]
        .concat();

        let footers = scanner.scan_footers(&buffer);
        assert_eq!(footers, vec![4]);
    }

    #[test]
    fn jpeg_scan_multiple_files() {
        let scanner = JpegScanner::new();

        let buffer: Vec<u8> = [
            &[0x00, 0x00][..],
            &[0xFF, 0xD8, 0xFF][..],
            &[0xE0, 0x00, 0x10, 0x4A, 0x46][..],
            &[0xFF, 0xD9][..],
            &[0x00, 0x00, 0x00][..],
            &[0xFF, 0xD8, 0xFF][..],
            &[0xE1, 0x00, 0x08][..],
            &[0xFF, 0xD9][..],
            &[0x00][..],
        ]
        .concat();

        let headers = scanner.scan_headers(&buffer);
        let footers = scanner.scan_footers(&buffer);

        assert_eq!(headers, vec![2, 15]);
        assert_eq!(footers, vec![10, 21]);
    }

    #[test]
    fn jpeg_scan_no_matches() {
        let scanner = JpegScanner::new();
        let buffer = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let headers = scanner.scan_headers(&buffer);
        let footers = scanner.scan_footers(&buffer);

        assert!(headers.is_empty());
        assert!(footers.is_empty());
    }

    #[test]
    fn jpeg_scan_empty_buffer() {
        let scanner = JpegScanner::new();

        let headers = scanner.scan_headers(&[]);
        let footers = scanner.scan_footers(&[]);

        assert!(headers.is_empty());
        assert!(footers.is_empty());
    }

    #[test]
    fn jpeg_file_type() {
        let scanner = JpegScanner::new();
        assert_eq!(scanner.file_type(), FileType::Jpeg);
    }

    #[test]
    fn png_scan_headers_single_match() {
        let scanner = PngScanner::new();

        let buffer: Vec<u8> = [
            &[0x00, 0x11, 0x22][..],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A][..],
            &[0x00, 0x00, 0x00, 0x0D][..],
        ]
        .concat();

        let headers = scanner.scan_headers(&buffer);
        assert_eq!(headers, vec![3]);
    }

    #[test]
    fn png_scan_footers_single_match() {
        let scanner = PngScanner::new();

        let buffer: Vec<u8> = [
            &[0x00, 0x00, 0x00, 0x00][..],
            &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82][..],
            &[0xAA, 0xBB][..],
        ]
        .concat();

        let footers = scanner.scan_footers(&buffer);
        assert_eq!(footers, vec![4]);
    }

    #[test]
    fn png_scan_multiple_files() {
        let scanner = PngScanner::new();

        let buffer: Vec<u8> = [
            &[0x00][..],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A][..],
            &[0x00, 0x00, 0x00, 0x00][..],
            &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82][..],
            &[0x00, 0x00][..],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A][..],
            &[0x00, 0x00, 0x00, 0x00][..],
            &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82][..],
        ]
        .concat();

        let headers = scanner.scan_headers(&buffer);
        let footers = scanner.scan_footers(&buffer);

        assert_eq!(headers, vec![1, 23]);
        assert_eq!(footers, vec![13, 35]);
    }

    #[test]
    fn png_scan_no_matches() {
        let scanner = PngScanner::new();
        let buffer = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let headers = scanner.scan_headers(&buffer);
        let footers = scanner.scan_footers(&buffer);

        assert!(headers.is_empty());
        assert!(footers.is_empty());
    }

    #[test]
    fn png_scan_empty_buffer() {
        let scanner = PngScanner::new();

        let headers = scanner.scan_headers(&[]);
        let footers = scanner.scan_footers(&[]);

        assert!(headers.is_empty());
        assert!(footers.is_empty());
    }

    #[test]
    fn png_file_type() {
        let scanner = PngScanner::new();
        assert_eq!(scanner.file_type(), FileType::Png);
    }

    #[test]
    fn jpeg_partial_header_not_matched() {
        let scanner = JpegScanner::new();
        let buffer = vec![0xFF, 0xD8, 0x00, 0x00];
        let headers = scanner.scan_headers(&buffer);

        assert!(headers.is_empty());
    }

    #[test]
    fn png_partial_header_not_matched() {
        let scanner = PngScanner::new();
        let buffer = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        let headers = scanner.scan_headers(&buffer);

        assert!(headers.is_empty());
    }

    #[test]
    fn scanners_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<JpegScanner>();
        assert_send_sync::<PngScanner>();
    }
}
