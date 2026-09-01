//! The reassembly search timed at the size the field runs it.
//!
//! `benches/carve.rs` measures the search over a 320x240 frame in a 256 KiB
//! disk. The queue a real medium produces is neither: its cost is concentrated
//! in 4128x3096 frames sitting in regions packed with other photographs, and
//! `docs/defects/07` measured two orders of magnitude between those and the
//! same code over noise. A ratio measured on the small fixture therefore says
//! nothing about the stage, which is how a 3x decoder speedup was reported as
//! 61 h becoming 21 h.
//!
//! This harness replays the search at field dimensions: a region of
//! [`REGION_BYTES`] filled with whole photographs the tool recovered from the
//! medium under audit, one of them cut at a block boundary with its
//! continuation absent, so the search runs to its ceiling and assembles
//! nothing — the case the field spends its time in.
//!
//! # Supplying a corpus
//!
//! The photographs are real evidence and not this project's to commit, so the
//! harness reads them from `ARGOS_REPLAY_CORPUS`: a directory of whole `.jpg`
//! files a scan already wrote. Without that variable it reports what it needs
//! and exits, the way `corpus_recall.rs` does — an absent corpus is a corpus
//! that was not supplied.
//!
//! # Reading the numbers
//!
//! The ceiling is `ARGOS_REPLAY_CEILING` (default [`BENCH_CEILING`]) rather
//! than the shipped [`reassemble::MAX_HYPOTHESES`], because one field item at
//! the shipped ceiling took 4112 s and nothing is iterated on in units of an
//! hour. That is a property of *this instrument*, not of the search: the
//! shipped ceiling is untouched, and `linearity` measures whether cost per
//! hypothesis is flat, which is what makes a cost at this ceiling stand for a
//! cost at that one. Read `linearity` before reading anything else.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use argos_carve::reassemble::{self, Broken, Limits};
use argos_carve::{Format, Scratch};
use argos_core::ByteOffset;

/// Fragment grid the search steps on, and the medium's allocation unit.
const BLOCK: usize = argos_carve::classify::BLOCK_BYTES;

/// [`BLOCK`] as the search's own offsets count it.
const BLOCK_BYTES: u64 = argos_carve::classify::BLOCK_BYTES as u64;

/// Bytes one region holds, as `argos_engine::pipeline::region::REGION_BYTES` sets it.
const REGION_BYTES: usize = 256 * 1024 * 1024;

/// Hypotheses one stratum is timed over.
///
/// Two orders below the shipped ceiling so a cycle is minutes. Sound only
/// while `linearity` reports a ratio near the ceiling ratio.
const BENCH_CEILING: u32 = 2_048;

/// What one stratum of the queue costs.
struct Timing {
    stratum: String,
    /// Frame the candidate declares, as the manifest records it.
    declared: Option<(u32, u32)>,
    /// MCUs the candidate's prefix accounted for — the queue's sort key.
    decoded: u32,
    hypotheses: u32,
    elapsed: Duration,
    /// Whether the search assembled anything, which the field case does not.
    assembled: bool,
}

impl Timing {
    fn per_hypothesis(&self) -> f64 {
        if self.hypotheses == 0 {
            return 0.0;
        }
        self.elapsed.as_secs_f64() / f64::from(self.hypotheses)
    }

    /// What one item would cost at the shipped ceiling, if cost per hypothesis
    /// is flat. `linearity` is what licenses this.
    fn at_shipped_ceiling(&self) -> f64 {
        self.per_hypothesis() * f64::from(reassemble::MAX_HYPOTHESES)
    }
}

fn main() {
    let Some(dir) = std::env::var_os("ARGOS_REPLAY_CORPUS").map(PathBuf::from) else {
        println!(
            "ARGOS_REPLAY_CORPUS is not set.\n\
             Point it at a directory of whole .jpg files a scan recovered from the medium \
             under audit; the harness fills a {REGION_BYTES}-byte region with them and cuts \
             one, which is the shape the field queue's expensive items have."
        );
        return;
    };

    let mut photos = load_photos(&dir);
    if photos.is_empty() {
        println!("{} holds no readable .jpg", dir.display());
        return;
    }
    // Largest first: the tail item is the one the queue sorts to the front.
    photos.sort_by_key(|photo| std::cmp::Reverse(photo.len()));
    println!(
        "corpus  {} photographs from {}, largest {} bytes",
        photos.len(),
        dir.display(),
        photos[0].len()
    );

    let ceiling: u32 = std::env::var("ARGOS_REPLAY_CEILING")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(BENCH_CEILING);
    println!(
        "ceiling {ceiling} hypotheses (shipped: {})\n",
        reassemble::MAX_HYPOTHESES
    );

    let region = region_of(&photos);
    let blocks = blocks_of(&region);
    println!(
        "region  {} bytes of whole photographs, {} blocks the walk may continue into\n",
        region.len(),
        blocks.len()
    );

    let mut timings = Vec::new();
    for (stratum, index) in [
        ("tail (largest frame)", 0),
        ("median (smallest frame)", photos.len() - 1),
    ] {
        if let Some(timing) = time_stratum(stratum, &photos, index, &region, ceiling) {
            timings.push(timing);
        }
        if let Some(timing) = time_walk(stratum, &photos, index, &region, &blocks, ceiling) {
            timings.push(timing);
        }
    }

    println!(
        "{:<38} {:>11} {:>8} {:>12} {:>13} {:>11}",
        "stratum", "declared", "decoded", "hypotheses", "wall", "per hyp"
    );
    for timing in &timings {
        let declared = timing
            .declared
            .map_or_else(|| "-".to_owned(), |(w, h)| format!("{w}x{h}"));
        println!(
            "{:<38} {declared:>11} {:>8} {:>12} {:>12.3}s {:>9.1}us{}",
            timing.stratum,
            timing.decoded,
            timing.hypotheses,
            timing.elapsed.as_secs_f64(),
            timing.per_hypothesis() * 1e6,
            if timing.assembled {
                "  (assembled)"
            } else {
                ""
            }
        );
    }

    println!(
        "\nprojected at the shipped ceiling of {} hypotheses:",
        reassemble::MAX_HYPOTHESES
    );
    for timing in &timings {
        println!(
            "  {:<38} {:>10.1}s per item",
            timing.stratum,
            timing.at_shipped_ceiling()
        );
    }

    linearity(&photos, &region, ceiling);
    scaling(&photos, &region, ceiling);
}

/// Throughput as workers are added, over one shared region.
///
/// `argos_engine::pipeline::reassembly::in_parallel` gives every worker its own view of
/// the held bytes and its own buffers, so nothing is locked and the ideal is
/// linear. What it does not give them is their own memory bandwidth: each
/// hypothesis copies trial bytes out of a 256 MiB region, and twelve of those
/// at once is a different machine from one. The field ran twelve workers on
/// six physical cores, so what this measures is whether the stage's projected
/// cost may be divided by twelve, by six, or by less.
fn scaling(photos: &[Vec<u8>], region: &[u8], ceiling: u32) {
    let Some((disk, broken)) = planted(photos, 0, region) else {
        return;
    };
    let medium_len = disk.len() as u64;
    println!("\nworkers   items    wall    items/s   speedup");
    let mut baseline = 0.0_f64;
    for workers in [1_usize, 2, 6, 12] {
        let items = f64::from(u32::try_from(workers).unwrap_or(u32::MAX));
        let started = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let disk = disk.as_slice();
                scope.spawn(move || {
                    let mut scratch = Scratch::new();
                    let mut src = Cursor::new(disk);
                    reassemble::bifragment(
                        &mut src,
                        broken,
                        medium_len,
                        limits(ceiling),
                        &mut scratch,
                    )
                    .expect("the region is memory")
                });
            }
        });
        let elapsed = started.elapsed().as_secs_f64();
        let rate = items / elapsed;
        if workers == 1 {
            baseline = rate;
        }
        println!(
            "{workers:>7} {items:>7.0} {elapsed:>7.3}s {rate:>10.3} {:>8.2}x",
            rate / baseline
        );
    }
}

/// Whether cost per hypothesis is flat, which is what lets a cost measured at
/// the bench ceiling stand for a cost at the shipped one.
///
/// Two ceilings an octave apart over the same candidate. A ratio near two says
/// the search spends its budget at a constant rate; anything else says the
/// instrument does not extrapolate and the numbers above are only about
/// themselves.
fn linearity(photos: &[Vec<u8>], region: &[u8], ceiling: u32) {
    let Some(low) = time_stratum("linearity/1x", photos, 0, region, ceiling) else {
        return;
    };
    let Some(high) = time_stratum("linearity/2x", photos, 0, region, ceiling * 2) else {
        return;
    };
    let hyp_ratio = f64::from(high.hypotheses) / f64::from(low.hypotheses.max(1));
    let time_ratio = high.elapsed.as_secs_f64() / low.elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    println!(
        "\nlinearity  {}x the hypotheses cost {:.3}x the time  ({:.1}us vs {:.1}us per hypothesis)",
        hyp_ratio,
        time_ratio,
        low.per_hypothesis() * 1e6,
        high.per_hypothesis() * 1e6
    );
}

/// Times one search over `region` with `photos[index]` cut and its
/// continuation absent.
fn time_stratum(
    stratum: &'static str,
    photos: &[Vec<u8>],
    index: usize,
    region: &[u8],
    ceiling: u32,
) -> Option<Timing> {
    planted(photos, index, region).map(|(disk, broken)| {
        let medium_len = disk.len() as u64;
        let mut scratch = Scratch::new();
        let mut src = Cursor::new(disk.as_slice());
        let started = Instant::now();
        let attempt =
            reassemble::bifragment(&mut src, broken, medium_len, limits(ceiling), &mut scratch)
                .expect("the region is memory");
        let elapsed = started.elapsed();
        Timing {
            stratum: format!("{stratum} / gap search"),
            declared: broken.declared,
            decoded: broken.decoded,
            hypotheses: attempt.hypotheses,
            elapsed,
            assembled: attempt.reassembly.is_some(),
        }
    })
}

/// Times the graph walk over the same candidate.
///
/// The stage counts every header twice — the gap search, then the walk over
/// the region's classified blocks — so half the queue's steps are this, and a
/// measurement of the search that omits it measures half a stage.
fn time_walk(
    stratum: &'static str,
    photos: &[Vec<u8>],
    index: usize,
    region: &[u8],
    blocks: &[reassemble::Candidate],
    ceiling: u32,
) -> Option<Timing> {
    planted(photos, index, region).map(|(disk, broken)| {
        let medium_len = disk.len() as u64;
        let mut scratch = Scratch::new();
        let mut src = Cursor::new(disk.as_slice());
        let started = Instant::now();
        let walk = reassemble::parallel_unique_path(
            &mut src,
            std::slice::from_ref(&broken),
            blocks,
            &[],
            medium_len,
            limits(ceiling),
            &mut scratch,
        )
        .expect("the region is memory");
        let elapsed = started.elapsed();
        Timing {
            stratum: format!("{stratum} / graph walk"),
            declared: broken.declared,
            decoded: broken.decoded,
            hypotheses: walk.hypotheses,
            elapsed,
            assembled: !walk.assembled.is_empty(),
        }
    })
}

/// The region with `photos[index]` cut into it, and where the decoder gives up
/// on it — the `Broken` a validation pass would have recorded.
fn planted(photos: &[Vec<u8>], index: usize, region: &[u8]) -> Option<(Vec<u8>, Broken)> {
    let photo = photos.get(index)?;
    // The prefix the medium still holds: whole blocks, half the frame. The
    // field's tail items break at `progress` 0.9998 over a 4-5 MB span, which
    // is what an 8 MB photograph cut here produces.
    let keep = (photo.len() / 2 / BLOCK * BLOCK).max(BLOCK);
    let at = plant_at(region.len(), keep);
    let mut disk = region.to_vec();
    disk[at..at + keep].copy_from_slice(&photo[..keep]);

    let medium_len = disk.len() as u64;
    let mut scratch = Scratch::new();
    let mut src = Cursor::new(disk.as_slice());
    let broken = reassemble::locate_break(
        &mut src,
        ByteOffset::new(at as u64),
        Format::Jpeg,
        medium_len,
        &mut scratch,
    )
    .expect("the region is memory")?;
    Some((disk, broken))
}

/// The search bounded to `ceiling`, on the grid the medium allocates in.
fn limits(ceiling: u32) -> Limits {
    Limits {
        max_hypotheses: ceiling,
        search_floor: 0,
        block_bytes: BLOCK_BYTES,
        block_origin: 0,
        ..Limits::default()
    }
}

/// The region's blocks that could hold image data, as `pipeline::region::Region` picks
/// them: every 4 KiB chunk whose profile can carry a compressed stream.
fn blocks_of(region: &[u8]) -> Vec<reassemble::Candidate> {
    region
        .as_chunks::<BLOCK>()
        .0
        .iter()
        .enumerate()
        .filter_map(|(index, chunk)| {
            let profile = argos_carve::classify::classify(chunk);
            profile
                .class
                .can_hold_image_data()
                .then(|| reassemble::Candidate {
                    start: ByteOffset::new(offset_of(index)),
                    profile,
                })
        })
        .collect()
}

/// Where the cut photograph is planted: on the block grid, far enough in that
/// the search sweeps whole photographs either side of it.
fn plant_at(region_len: usize, keep: usize) -> usize {
    let at = region_len / 3 / BLOCK * BLOCK;
    at.min(region_len.saturating_sub(keep))
}

/// A region of [`REGION_BYTES`] packed with whole photographs, which is what
/// makes a rejected hypothesis walk instead of stopping on an illegal code.
fn region_of(photos: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(REGION_BYTES);
    let mut which = 0_usize;
    while out.len() < REGION_BYTES {
        let photo = &photos[which % photos.len()];
        let take = photo.len().min(REGION_BYTES - out.len());
        out.extend_from_slice(&photo[..take]);
        // Photographs land on allocation boundaries, as a filesystem writes
        // them; the padding is what a real region has between files.
        while !out.len().is_multiple_of(BLOCK) && out.len() < REGION_BYTES {
            out.push(0);
        }
        which += 1;
    }
    out.truncate(REGION_BYTES);
    out
}

/// Photographs enough to fill a region, plus small ones for the other end of
/// the queue's size distribution.
///
/// Reading the whole directory would be gigabytes for a 256 MiB region, so the
/// largest [`REGION_PHOTOS`] are taken for the filler and the tail stratum and
/// the smallest few for the median one.
fn load_photos(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sized: Vec<(u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg"))
        })
        .filter_map(|path| {
            std::fs::metadata(&path)
                .ok()
                .map(|meta| (meta.len(), path))
                .filter(|(len, _)| *len > BLOCK_BYTES * 2)
        })
        .collect();
    sized.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut wanted: Vec<&PathBuf> = sized.iter().take(REGION_PHOTOS).map(|(_, at)| at).collect();
    wanted.extend(sized.iter().rev().take(SMALL_PHOTOS).map(|(_, at)| at));
    wanted
        .into_iter()
        .filter_map(|path| std::fs::read(path).ok())
        .collect()
}

/// Where a block's bytes start, in the region's own coordinates.
fn offset_of(index: usize) -> u64 {
    u64::try_from(index.saturating_mul(BLOCK)).unwrap_or(u64::MAX)
}

/// Largest photographs read, which fill the region and provide the tail item.
const REGION_PHOTOS: usize = 48;

/// Smallest photographs read, which provide the other end of the queue.
const SMALL_PHOTOS: usize = 4;
