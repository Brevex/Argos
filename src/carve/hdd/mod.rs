use crate::carve::Candidate;
use crate::carve::hdd::pup::{Seed, run};
use crate::carve::ssd::patterns::{PatternKind, all_patterns};
use crate::error::ArgosError;

pub mod pup;
pub mod sht;

const SCAN_CHUNK_SIZE: usize = 64 * 1024 * 1024;
const PUP_MAX_BLOCKS: usize = 10_000;

pub fn scan(
    data: &[u8],
    block_size: usize,
    mut on_progress: impl FnMut(u64) -> bool,
) -> Result<Vec<Candidate>, ArgosError> {
    let patterns = all_patterns();
    let pattern_bytes: Vec<&[u8]> = patterns.iter().map(|(p, _)| *p).collect();
    let ac = aho_corasick::AhoCorasick::new(&pattern_bytes)?;
    let max_pattern_len = pattern_bytes.iter().map(|p| p.len()).max().unwrap_or(0);
    let pattern_kinds: Vec<PatternKind> = patterns.iter().map(|(_, k)| *k).collect();
    let overlap = max_pattern_len.saturating_sub(1);

    let mut seeds = Vec::new();
    let mut pos: usize = 0;
    while pos < data.len() {
        let chunk_start = pos.saturating_sub(overlap);
        let chunk_end = (pos + SCAN_CHUNK_SIZE).min(data.len());
        let chunk = &data[chunk_start..chunk_end];
        for mat in ac.find_iter(chunk) {
            let absolute_end = chunk_start + mat.end();
            if absolute_end <= pos {
                continue;
            }
            let absolute_start = chunk_start + mat.start();
            let pattern_id = mat.pattern().as_usize();
            if let PatternKind::Header(format) = pattern_kinds[pattern_id] {
                let block_index = (absolute_start / block_size) as u64;
                seeds.push(Seed {
                    block_index,
                    format,
                });
            }
        }
        pos = chunk_end;
        if !on_progress(pos as u64) {
            break;
        }
    }

    let candidates = run(&seeds, data, block_size, PUP_MAX_BLOCKS);
    Ok(candidates)
}
