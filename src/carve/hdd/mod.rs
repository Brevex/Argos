use memmap2::MmapOptions;

use crate::carve::Candidate;
use crate::carve::hdd::pup::{run, Seed};
use crate::carve::ssd::patterns::{all_patterns, PatternKind};
use crate::error::ArgosError;

pub mod pup;
pub mod sht;

pub fn scan(source_path: &std::path::Path, block_size: usize) -> Result<Vec<Candidate>, ArgosError> {
    let file = std::fs::File::open(source_path)?;
    let mmap = unsafe { MmapOptions::new().map(&file)? };

    let patterns = all_patterns();
    let pattern_bytes: Vec<&[u8]> = patterns.iter().map(|(p, _)| *p).collect();
    let ac = aho_corasick::AhoCorasick::new(&pattern_bytes)?;
    let pattern_kinds: Vec<PatternKind> = patterns.iter().map(|(_, k)| *k).collect();

    let mut seeds = Vec::new();
    for mat in ac.find_iter(&mmap) {
        let pattern_id = mat.pattern().as_usize();
        let kind = pattern_kinds[pattern_id];
        if let PatternKind::Header(format) = kind {
            let block_index = (mat.start() / block_size) as u64;
            seeds.push(Seed { block_index, format });
        }
    }

    let candidates = run(&seeds, &mmap, block_size, 10_000);
    Ok(candidates)
}
