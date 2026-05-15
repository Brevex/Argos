use std::collections::{BinaryHeap, HashSet};

use crate::carve::hdd::sht::{Decision, SprtAccumulator};
use crate::carve::{Candidate, ImageFormat};
use crate::validate::jpeg;
use crate::validate::png;

const SEARCH_WINDOW_BLOCKS: u64 = 1;
const JPEG_ACCEPTANCE_THRESHOLD: f32 = 0.25;
const PNG_ACCEPTANCE_THRESHOLD: f32 = 0.25;
const PNG_IEND_CHUNK: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[derive(Debug, Clone)]
pub struct Seed {
    pub block_index: u64,
    pub format: ImageFormat,
}

#[derive(Debug, Clone)]
struct Path {
    blocks: Vec<u64>,
    format: ImageFormat,
    weight: f64,
    sprt: SprtAccumulator,
    png_partial: Option<png::PartialChunk>,
}

#[derive(Debug, Clone)]
struct NextBlock {
    index: u64,
    score: f32,
    weight: f64,
    footer_end: Option<usize>,
    png_partial: Option<png::PartialChunk>,
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.weight == other.weight
    }
}

impl Eq for Path {}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.weight
            .partial_cmp(&other.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

pub fn run(seeds: &[Seed], data: &[u8], block_size: usize, max_blocks: usize) -> Vec<Candidate> {
    let mut consumed = HashSet::with_capacity(max_blocks);
    let mut queue = BinaryHeap::with_capacity(seeds.len());
    let mut completed = Vec::new();

    for seed in seeds {
        if consumed.contains(&seed.block_index) {
            continue;
        }
        consumed.insert(seed.block_index);
        let Some((start, end)) = block_bounds(data.len(), block_size, seed.block_index) else {
            continue;
        };
        let png_partial = match seed.format {
            ImageFormat::Png => Some(png::PartialChunk::default()),
            ImageFormat::Jpeg => None,
        };
        let path = Path {
            blocks: vec![seed.block_index],
            format: seed.format,
            weight: 0.0,
            sprt: SprtAccumulator::new(),
            png_partial,
        };
        if let Some(footer_end) = footer_end(seed.format, &data[start..end]) {
            if let Some(candidate) =
                candidate_from_path(&path, block_size, seed.block_index, footer_end)
            {
                completed.push(candidate);
            }
        } else {
            queue.push(path);
        }
    }

    while let Some(mut path) = queue.pop() {
        let Some(&last) = path.blocks.last() else {
            continue;
        };
        if path.blocks.len() >= max_blocks {
            if let Some(candidate) = candidate_from_blocks(&path, block_size) {
                completed.push(candidate);
            }
            continue;
        }

        if let Some(next) = best_next_block(&path, data, block_size, last, &consumed) {
            path.blocks.push(next.index);
            path.weight = next.weight;
            if next.png_partial.is_some() {
                path.png_partial = next.png_partial;
            }
            consumed.insert(next.index);

            if let Some(footer_end) = next.footer_end {
                if let Some(candidate) =
                    candidate_from_path(&path, block_size, next.index, footer_end)
                {
                    completed.push(candidate);
                }
                continue;
            }

            update_sprt(&mut path, next.score);

            if path.sprt.decision() == Decision::H1 {
                if let Some(candidate) = candidate_from_blocks(&path, block_size) {
                    completed.push(candidate);
                }
                continue;
            }

            queue.push(path);
        } else if let Some(candidate) = candidate_from_blocks(&path, block_size) {
            completed.push(candidate);
        }
    }

    completed
}

fn best_next_block(
    path: &Path,
    data: &[u8],
    block_size: usize,
    last: u64,
    consumed: &HashSet<u64>,
) -> Option<NextBlock> {
    let mut best = None;
    for index in last + 1..=last.saturating_add(SEARCH_WINDOW_BLOCKS) {
        if consumed.contains(&index) {
            continue;
        }
        let Some((start, end)) = block_bounds(data.len(), block_size, index) else {
            break;
        };
        let block = &data[start..end];
        let footer = footer_end(path.format, block);
        let (score, png_partial) = continuation_score(path, block);
        if footer.is_none() && score < acceptance_threshold(path.format) {
            continue;
        }
        let weight = if footer.is_some() {
            2.0 + score as f64
        } else {
            score as f64
        };
        if best
            .as_ref()
            .is_none_or(|current: &NextBlock| weight > current.weight)
        {
            best = Some(NextBlock {
                index,
                score,
                weight,
                footer_end: footer,
                png_partial,
            });
        }
    }
    best
}

fn continuation_score(path: &Path, block: &[u8]) -> (f32, Option<png::PartialChunk>) {
    match path.format {
        ImageFormat::Jpeg => (jpeg::continuation_score(block), None),
        ImageFormat::Png => path.png_partial.as_ref().map_or((0.0, None), |_| {
            let mut partial = path.png_partial.clone().unwrap_or_default();
            let score = png::continuation_score(&mut partial, block);
            (score, Some(partial))
        }),
    }
}

fn update_sprt(path: &mut Path, score: f32) {
    if score > 0.0 && score < 1.0 {
        let ratio = (1.01 - score as f64) / (score as f64 + 0.01);
        path.sprt.update(ratio.ln());
    } else if score >= 1.0 {
        path.sprt.update(-5.0);
    } else {
        path.sprt.update(5.0);
    }
}

fn acceptance_threshold(format: ImageFormat) -> f32 {
    match format {
        ImageFormat::Jpeg => JPEG_ACCEPTANCE_THRESHOLD,
        ImageFormat::Png => PNG_ACCEPTANCE_THRESHOLD,
    }
}

fn footer_end(format: ImageFormat, block: &[u8]) -> Option<usize> {
    match format {
        ImageFormat::Jpeg => block
            .windows(2)
            .position(|w| w[0] == 0xFF && w[1] == 0xD9)
            .map(|pos| pos + 2),
        ImageFormat::Png => block
            .windows(PNG_IEND_CHUNK.len())
            .position(|w| w == PNG_IEND_CHUNK)
            .map(|pos| pos + PNG_IEND_CHUNK.len()),
    }
}

fn block_bounds(data_len: usize, block_size: usize, index: u64) -> Option<(usize, usize)> {
    let index = usize::try_from(index).ok()?;
    let start = index.checked_mul(block_size)?;
    if start >= data_len {
        return None;
    }
    let end = start.saturating_add(block_size).min(data_len);
    Some((start, end))
}

fn candidate_from_path(
    path: &Path,
    block_size: usize,
    last_block: u64,
    footer_end: usize,
) -> Option<Candidate> {
    let first = *path.blocks.first()?;
    let offset = first.checked_mul(block_size as u64)?;
    let end = last_block
        .checked_mul(block_size as u64)?
        .checked_add(footer_end as u64)?;
    Some(Candidate {
        offset,
        length: Some(end.checked_sub(offset)?),
        format: path.format,
    })
}

fn candidate_from_blocks(path: &Path, block_size: usize) -> Option<Candidate> {
    let first = *path.blocks.first()?;
    Some(Candidate {
        offset: first.checked_mul(block_size as u64)?,
        length: Some((path.blocks.len() as u64).checked_mul(block_size as u64)?),
        format: path.format,
    })
}
