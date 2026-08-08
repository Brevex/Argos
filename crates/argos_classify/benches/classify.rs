//! What triage costs per artifact.
//!
//! Triage runs once per recovered artifact, not once per block, so it is not
//! on the scan's hot path — the signature sweep is. What these measure is
//! whether the stage is proportionate: a scan that reads a medium at
//! gigabytes a second must not then spend minutes labeling what it found
//! (`M-HOTPATH`).
//!
//! The four steps of one artifact are measured separately, because they have
//! very different costs and only one of them is the model.

use argos_classify::fixture::{Slice, sample};
use argos_classify::{Triage, net, phash, prefilter};
use argos_core::classify::{Classifier, PixelImage};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// A representative artifact: a camera-resolution photograph.
fn photograph() -> PixelImage {
    sample(Slice::Photograph, 12_345).image
}

/// The cheap statistics pass that settles most synthetic assets.
fn prefilter_pass(c: &mut Criterion) {
    let image = photograph();
    let mut group = c.benchmark_group("prefilter");
    group.throughput(criterion::Throughput::Elements(image.pixel_count()));
    group.bench_function("features", |b| {
        b.iter(|| black_box(prefilter::features(black_box(&image))));
    });
    group.finish();
}

/// Perceptual hashing, which every artifact pays before inference.
fn perceptual_hash(c: &mut Criterion) {
    let image = photograph();
    let mut group = c.benchmark_group("phash");
    group.throughput(criterion::Throughput::Elements(image.pixel_count()));
    group.bench_function("blockhash", |b| {
        b.iter(|| black_box(phash::perceptual_hash(black_box(&image))));
    });
    group.finish();
}

/// Reducing a full-size image to the model's 64x64 input.
fn preprocessing(c: &mut Criterion) {
    let image = photograph();
    c.bench_function("model_input", |b| {
        b.iter(|| black_box(net::model_input(black_box(&image))));
    });
}

/// Inference itself, at the batch sizes the engine's worker forms.
///
/// The per-image cost is what decides whether the engine's batch worker is
/// worth its complexity: if a batch of eight costs eight times a batch of
/// one, batching is buying nothing and the worker should hand images over one
/// at a time.
fn inference(c: &mut Criterion) {
    let Ok(mut triage) = Triage::new() else {
        // A benchmark cannot gate on the pinned model; the eval harness does
        // that. Nothing to measure if it did not load.
        return;
    };
    let mut group = c.benchmark_group("inference");
    for size in [1_usize, 4, 8] {
        let images: Vec<PixelImage> = (0..size)
            .map(|index| sample(Slice::Photograph, 900 + index as u64).image)
            .collect();
        group.throughput(criterion::Throughput::Elements(size as u64));
        group.bench_function(format!("batch_{size}"), |b| {
            b.iter_batched(
                || images.clone(),
                |images| black_box(triage.score_batch(black_box(&images)).ok()),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    prefilter_pass,
    perceptual_hash,
    preprocessing,
    inference
);
criterion_main!(benches);
