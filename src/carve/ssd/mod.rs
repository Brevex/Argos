pub mod patterns;

use aho_corasick::AhoCorasick;

use crate::carve::{Candidate, ImageFormat};
use crate::carve::ssd::patterns::{all_patterns, PatternKind};
use crate::error::ArgosError;

pub struct Scanner {
    ac: AhoCorasick,
    pattern_kinds: Vec<PatternKind>,
    max_pattern_len: usize,
    overlap: Vec<u8>,
    concat_buf: Vec<u8>,
    offset_base: u64,
    open_candidates: Vec<OpenCandidate>,
}

#[derive(Debug)]
struct OpenCandidate {
    offset: u64,
    format: ImageFormat,
}

impl Scanner {
    pub fn new() -> Result<Self, ArgosError> {
        let patterns = all_patterns();
        let pattern_bytes: Vec<&[u8]> = patterns.iter().map(|(p, _)| *p).collect();
        let ac = AhoCorasick::new(&pattern_bytes)?;
        let max_pattern_len = pattern_bytes.iter().map(|p| p.len()).max().unwrap_or(0);
        let pattern_kinds: Vec<PatternKind> = patterns.iter().map(|(_, k)| *k).collect();

        Ok(Self {
            ac,
            pattern_kinds,
            max_pattern_len,
            overlap: Vec::with_capacity(max_pattern_len.saturating_sub(1)),
            concat_buf: Vec::with_capacity(1024 * 1024 + max_pattern_len),
            offset_base: 0,
            open_candidates: Vec::new(),
        })
    }

    pub fn scan_block(&mut self, block: &[u8]) -> Result<Vec<Candidate>, ArgosError> {
        let mut completed = Vec::new();

        self.concat_buf.clear();
        self.concat_buf.extend_from_slice(&self.overlap);
        self.concat_buf.extend_from_slice(block);

        for mat in self.ac.find_iter(&self.concat_buf) {
            let pattern_id = mat.pattern().as_usize();
            let mat_start = mat.start();
            let mat_end = mat.end();
            let pattern_len = mat_end - mat_start;

            if mat_start + pattern_len <= self.overlap.len() {
                continue;
            }

            let absolute_offset =
                self.offset_base - self.overlap.len() as u64 + mat_start as u64;
            let pattern_kind = self.pattern_kinds[pattern_id];

            match pattern_kind {
                PatternKind::Header(format) => {
                    self.open_candidates.push(OpenCandidate {
                        offset: absolute_offset,
                        format,
                    });
                }
                PatternKind::Footer(format) => {
                    if let Some(pos) =
                        self.open_candidates.iter().rposition(|c| c.format == format)
                    {
                        let open = self.open_candidates.remove(pos);
                        completed.push(Candidate {
                            offset: open.offset,
                            length: Some(
                                absolute_offset + pattern_len as u64 - open.offset,
                            ),
                            format,
                        });
                    }
                }
            }
        }

        let overlap_keep = self.max_pattern_len.saturating_sub(1);
        self.overlap.clear();
        if block.len() >= overlap_keep {
            self.overlap
                .extend_from_slice(&block[block.len() - overlap_keep..]);
        } else {
            let keep_from_overlap = overlap_keep.saturating_sub(block.len());
            if keep_from_overlap > 0 && self.concat_buf.len() >= keep_from_overlap {
                let start = self.concat_buf.len() - keep_from_overlap;
                self.overlap.extend_from_slice(&self.concat_buf[start..]);
            }
        }

        self.offset_base += block.len() as u64;

        Ok(completed)
    }
}

impl std::fmt::Debug for Scanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scanner")
            .field("max_pattern_len", &self.max_pattern_len)
            .field("offset_base", &self.offset_base)
            .field("open_count", &self.open_candidates.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_finds_jpeg() -> Result<(), ArgosError> {
        let mut scanner = Scanner::new()?;
        let data = [0xFF, 0xD8, 0x00, 0x01, 0x02, 0xFF, 0xD9];
        let candidates = scanner.scan_block(&data)?;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].offset, 0);
        assert_eq!(candidates[0].length, Some(7));
        assert_eq!(candidates[0].format, ImageFormat::Jpeg);
        Ok(())
    }

    #[test]
    fn scanner_finds_png() -> Result<(), ArgosError> {
        let mut scanner = Scanner::new()?;
        let mut data = vec![0x00; 100];
        data[10..18].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        data[90..94].copy_from_slice(&[0x49, 0x45, 0x4E, 0x44]);
        let candidates = scanner.scan_block(&data)?;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].offset, 10);
        assert_eq!(candidates[0].length, Some(84));
        assert_eq!(candidates[0].format, ImageFormat::Png);
        Ok(())
    }

    #[test]
    fn scanner_crosses_boundary() -> Result<(), ArgosError> {
        let mut scanner = Scanner::new()?;
        let block1 = [0xFF, 0xD8, 0x00, 0xFF];
        let block2 = [0xD9, 0x00, 0x00];

        let c1 = scanner.scan_block(&block1)?;
        assert!(c1.is_empty());

        let c2 = scanner.scan_block(&block2)?;
        assert_eq!(c2.len(), 1);
        assert_eq!(c2[0].offset, 0);
        assert_eq!(c2[0].length, Some(5));
        assert_eq!(c2[0].format, ImageFormat::Jpeg);
        Ok(())
    }

    #[test]
    fn scanner_multiple_jpegs() -> Result<(), ArgosError> {
        let mut scanner = Scanner::new()?;
        let data = [0xFF, 0xD8, 0xFF, 0xD9, 0xFF, 0xD8, 0xFF, 0xD9];
        let candidates = scanner.scan_block(&data)?;
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].offset, 0);
        assert_eq!(candidates[0].length, Some(4));
        assert_eq!(candidates[1].offset, 4);
        assert_eq!(candidates[1].length, Some(4));
        Ok(())
    }

    #[test]
    fn scanner_orphan_footer_ignored() -> Result<(), ArgosError> {
        let mut scanner = Scanner::new()?;
        let data = [0x00, 0xFF, 0xD9, 0x00];
        let candidates = scanner.scan_block(&data)?;
        assert!(candidates.is_empty());
        Ok(())
    }
}
