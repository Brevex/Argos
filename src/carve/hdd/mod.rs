use crate::carve::Candidate;
use crate::carve::hdd::pup::{Seed, run};
use crate::carve::ssd::patterns::{PatternKind, all_patterns};
use crate::error::ArgosError;

pub mod pup;
pub mod sht;

pub fn scan(data: &[u8], block_size: usize) -> Result<Vec<Candidate>, ArgosError> {
    let patterns = all_patterns();
    let pattern_bytes: Vec<&[u8]> = patterns.iter().map(|(p, _)| *p).collect();
    let ac = aho_corasick::AhoCorasick::new(&pattern_bytes)?;
    let pattern_kinds: Vec<PatternKind> = patterns.iter().map(|(_, k)| *k).collect();

    let mut seeds = Vec::new();
    for mat in ac.find_iter(data) {
        let pattern_id = mat.pattern().as_usize();
        let kind = pattern_kinds[pattern_id];
        if let PatternKind::Header(format) = kind {
            let block_index = (mat.start() / block_size) as u64;
            seeds.push(Seed { block_index, format });
        }
    }

    let candidates = run(&seeds, data, block_size, 10_000);
    Ok(candidates)
}
