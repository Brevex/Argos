//! What triage costs per artifact.
//!
//! Triage runs once per recovered artifact, not once per block, so it is not
//! on the scan's hot path — the signature sweep is. What these measure is
//! whether the stage is proportionate: a scan that reads a medium at gigabytes
//! a second must not then spend minutes labeling what it found (`M-HOTPATH`).
//!
//! That proportion is the reason there is no model here any more. A recovery
//! from a system disk produced twenty-three thousand artifacts, and inference
//! at 2.6 ms each spent a minute on them; the statistics below decide the same
//! question from the same decoded pixels.

use argos_classify::fixture::{Slice, sample};
use argos_classify::{Triage, phash, rules};
use argos_core::classify::PixelImage;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// A representative artifact: a camera-resolution photograph.
fn photograph() -> PixelImage {
    sample(Slice::Photograph, 12_345).image
}

/// The statistics pass, which is where the whole decision's time goes.
fn statistics(c: &mut Criterion) {
    let image = photograph();
    let mut group = c.benchmark_group("rules");
    group.throughput(criterion::Throughput::Elements(image.pixel_count()));
    group.bench_function("features", |b| {
        b.iter(|| black_box(rules::features(black_box(&image))));
    });
    group.finish();
}

/// The whole label, end to end, as the engine asks for it.
fn decision(c: &mut Criterion) {
    let image = photograph();
    c.bench_function("decide", |b| {
        b.iter(|| black_box(Triage::decide(black_box(&image))));
    });
}

/// Perceptual hashing, which every artifact pays for near-duplicate grouping.
fn perceptual_hash(c: &mut Criterion) {
    let image = photograph();
    let mut group = c.benchmark_group("phash");
    group.throughput(criterion::Throughput::Elements(image.pixel_count()));
    group.bench_function("perceptual_hash", |b| {
        b.iter(|| black_box(phash::perceptual_hash(black_box(&image))));
    });
    group.finish();
}

criterion_group!(benches, statistics, decision, perceptual_hash);
criterion_main!(benches);
