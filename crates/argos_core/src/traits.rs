use crate::error::Result;
use crate::types::FileType;

pub trait BlockSource {
    fn read_chunk(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    fn size(&self) -> u64;
}

pub trait FileScanner: Send + Sync {
    fn scan_headers(&self, buffer: &[u8]) -> Vec<usize>;

    fn scan_footers(&self, buffer: &[u8]) -> Vec<usize>;

    fn file_type(&self) -> FileType;

    #[inline]
    fn scan_headers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.scan_headers(buffer) {
            callback(offset);
        }
    }

    #[inline]
    fn scan_footers_callback<F>(&self, buffer: &[u8], mut callback: F)
    where
        F: FnMut(usize),
    {
        for offset in self.scan_footers(buffer) {
            callback(offset);
        }
    }
}
