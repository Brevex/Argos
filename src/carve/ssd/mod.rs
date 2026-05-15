pub mod patterns;

use aho_corasick::AhoCorasick;

use crate::carve::ssd::patterns::{PatternKind, all_patterns};
use crate::carve::{Candidate, ImageFormat};
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

            let absolute_offset = self.offset_base - self.overlap.len() as u64 + mat_start as u64;
            let pattern_kind = self.pattern_kinds[pattern_id];

            match pattern_kind {
                PatternKind::Header(format) => {
                    if !self.open_candidates.iter().any(|c| c.format == format) {
                        self.open_candidates.push(OpenCandidate {
                            offset: absolute_offset,
                            format,
                        });
                    }
                }
                PatternKind::Footer(format) => {
                    if let Some(pos) = self
                        .open_candidates
                        .iter()
                        .rposition(|c| c.format == format)
                    {
                        let open = self.open_candidates.remove(pos);
                        completed.push(Candidate {
                            offset: open.offset,
                            length: Some(absolute_offset + pattern_len as u64 - open.offset),
                            format,
                        });
                    }
                }
            }
        }

        let overlap_keep = self.max_pattern_len.saturating_sub(1);
        self.overlap.clear();
        let keep = overlap_keep.min(self.concat_buf.len());
        if keep > 0 {
            let start = self.concat_buf.len() - keep;
            self.overlap.extend_from_slice(&self.concat_buf[start..]);
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
