use std::collections::{BinaryHeap, HashSet};

use crate::carve::{Candidate, ImageFormat};
use crate::carve::hdd::sht::{Decision, SprtAccumulator};
use crate::validate::jpeg;
use crate::validate::png;

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
    jpeg_state: Option<jpeg::DecoderState>,
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
        self.weight.partial_cmp(&other.weight).unwrap_or(std::cmp::Ordering::Equal)
    }
}

pub fn run(
    seeds: &[Seed],
    data: &[u8],
    block_size: usize,
    max_blocks: usize,
) -> Vec<Candidate> {
    let mut consumed = HashSet::with_capacity(max_blocks);
    let mut queue = BinaryHeap::with_capacity(seeds.len());
    let mut completed = Vec::new();

    for seed in seeds {
        if consumed.contains(&seed.block_index) {
            continue;
        }
        let mut path = Path {
            blocks: vec![seed.block_index],
            format: seed.format,
            weight: 0.0,
            sprt: SprtAccumulator::new(),
            jpeg_state: None,
            png_partial: None,
        };
        initialize_state(&mut path, data, block_size);
        queue.push(path);
    }

    while let Some(mut path) = queue.pop() {
        let last = *path.blocks.last().unwrap();
        if path.blocks.len() >= max_blocks {
            continue;
        }

        let mut best_next: Option<(u64, f64)> = None;

        for next in [last + 1, last + 2] {
            if consumed.contains(&next) || next as usize * block_size >= data.len() {
                continue;
            }

            let end = ((next as usize + 1) * block_size).min(data.len());
            let block = &data[next as usize * block_size..end];
            let score = continuation_score(&path, block);

            if score > 0.0 {
                let weight = score as f64;
                if best_next.is_none_or(|(_, w)| weight > w) {
                    best_next = Some((next, weight));
                }
            }
        }

        if let Some((next, weight)) = best_next {
            path.blocks.push(next);
            path.weight = weight;
            let block = &data[next as usize * block_size..((next as usize + 1) * block_size).min(data.len())];
            update_sprt(&mut path, block);

            match path.sprt.decision() {
                Decision::H1 => {
                    let offset = path.blocks[0] * block_size as u64;
                    let length = (path.blocks.len() * block_size) as u64;
                    completed.push(Candidate {
                        offset,
                        length: Some(length),
                        format: path.format,
                    });
                    for b in &path.blocks {
                        consumed.insert(*b);
                    }
                }
                Decision::H0 => {
                    consumed.insert(next);
                    queue.push(path);
                }
                Decision::Continue => {
                    consumed.insert(next);
                    queue.push(path);
                }
            }
        } else {
            let offset = path.blocks[0] * block_size as u64;
            let length = (path.blocks.len() * block_size) as u64;
            completed.push(Candidate {
                offset,
                length: Some(length),
                format: path.format,
            });
            for b in &path.blocks {
                consumed.insert(*b);
            }
        }
    }

    completed
}

fn initialize_state(path: &mut Path, data: &[u8], block_size: usize) {
    match path.format {
        ImageFormat::Jpeg => {
            let start = path.blocks[0] as usize * block_size;
            let end = ((path.blocks[0] as usize + 1) * block_size).min(data.len());
            let block = &data[start..end];
            if let Ok(segments) = jpeg::parse_segments(block) {
                let mut state = jpeg::DecoderState::default();
                for seg in segments {
                    if seg.marker == jpeg::DHT {
                        if let Ok(table) = jpeg::parse_huffman_table(&seg.data) {
                            let slot = table.id & 0x03;
                            if table.class == 0 {
                                state.dc_tables[slot as usize] = Some(table);
                            } else {
                                state.ac_tables[slot as usize] = Some(table);
                            }
                        }
                    } else if seg.marker == jpeg::SOF0 && seg.data.len() >= 6 {
                        state.frame_height = u16::from_be_bytes([seg.data[1], seg.data[2]]);
                        state.frame_width = u16::from_be_bytes([seg.data[3], seg.data[4]]);
                        state.components = seg.data[5];
                    }
                }
                path.jpeg_state = Some(state);
            }
        }
        ImageFormat::Png => {
            path.png_partial = Some(png::PartialChunk::default());
        }
    }
}

fn continuation_score(path: &Path, block: &[u8]) -> f32 {
    match path.format {
        ImageFormat::Jpeg => {
            path.jpeg_state.as_ref().map_or(0.0, |state| {
                jpeg::continuation_score(state, block)
            })
        }
        ImageFormat::Png => {
            path.png_partial.as_ref().map_or(0.0, |_| {
                let mut partial = path.png_partial.clone().unwrap();
                png::continuation_score(&mut partial, block)
            })
        }
    }
}

fn update_sprt(path: &mut Path, block: &[u8]) {
    let score = continuation_score(path, block);
    if score > 0.0 && score < 1.0 {
        let ratio = (score as f64 + 0.01) / (1.01 - score as f64);
        path.sprt.update(ratio.ln());
    } else if score >= 1.0 {
        path.sprt.update(5.0);
    } else {
        path.sprt.update(-5.0);
    }
}
