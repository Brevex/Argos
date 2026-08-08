//! Benchmarks for the carving hot paths.
//!
//! Two very different costs live here and they are measured separately on
//! purpose. Detection and classification run over **every byte of the medium**,
//! so they are judged in bytes per second. Validation and entropy decoding run
//! per candidate, and entropy decoding runs thousands of times per fragmented
//! candidate during reassembly, so they are judged per image.

use std::io::Cursor;

use argos_carve::classify;
use argos_carve::fixture::{self, Disk, photo_jpeg};
use argos_carve::mcu;
use argos_carve::reassemble;
use argos_carve::{Detector, Format, Scratch};
use argos_core::geometry::ByteOffset;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Surface a whole-medium pass is measured over.
const SURFACE_BYTES: usize = 8 * 1024 * 1024;

fn surface_passes(c: &mut Criterion) {
    let surface = Disk::noisy(SURFACE_BYTES, 0x51ED_2A11_0000_0001).into_bytes();

    let mut group = c.benchmark_group("surface");
    group.throughput(Throughput::Bytes(surface.len() as u64));

    group.bench_function("signature detection", |b| {
        let detector = Detector::new();
        let mut hits = Vec::new();
        b.iter(|| {
            hits.clear();
            detector.hits_in(&surface, ByteOffset::new(0), usize::MAX, &mut hits);
        });
    });

    group.bench_function("block classification", |b| {
        b.iter(|| {
            for block in surface.chunks(classify::BLOCK_BYTES) {
                std::hint::black_box(classify::classify(block));
            }
        });
    });
    group.finish();
}

fn per_image(c: &mut Criterion) {
    let image = photo_jpeg(640, 480, 0x0F1E_2D3C_4B5A_6978);

    let mut group = c.benchmark_group("image");
    group.throughput(Throughput::Bytes(image.len() as u64));

    group.bench_function("structural validation", |b| {
        let mut scratch = Scratch::new();
        b.iter(|| {
            let mut src = Cursor::new(&image);
            std::hint::black_box(argos_carve::validate(
                Format::Jpeg,
                &mut src,
                ByteOffset::new(0),
                image.len() as u64,
                &mut scratch,
            ))
        });
    });

    // The reassembly oracle: one of these runs per hypothesis, so its cost is
    // what bounds how wide a search can be.
    group.bench_function("entropy decode", |b| {
        let mut scratch = Scratch::new();
        b.iter(|| {
            let mut src = Cursor::new(&image);
            std::hint::black_box(mcu::scan(
                &mut src,
                ByteOffset::new(0),
                image.len() as u64,
                &mut scratch,
            ))
        });
    });
    group.finish();
}

/// The reassembly search itself.
///
/// This is what the stage's decode budget is spent on, so it is the number
/// that says whether reassembly can run by default. Both the case that
/// succeeds and the case that exhausts its budget are measured: a scan meets
/// far more of the second than the first.
fn reassembly_search(c: &mut Criterion) {
    let block = classify::BLOCK_BYTES;
    let image = photo_jpeg(320, 240, 0x51ED_2A11_0000_0001);
    let found = fixture::fragmented(64 * block, &image, &[4 * block, 20 * block], block);
    // The same image with nothing to find: the second fragment is absent, so
    // the search runs to its budget and gives up.
    let missing = Disk::noisy(64 * block, 0x1357_9BDF_2468_ACE0)
        .with(4 * block, &image[..2 * block])
        .into_bytes();

    let mut group = c.benchmark_group("reassembly");
    group.sample_size(20);

    for (name, disk) in [
        ("gap search, found", &found.disk),
        ("gap search, exhausted", &missing),
    ] {
        group.bench_function(name, |b| {
            let mut scratch = Scratch::new();
            b.iter(|| {
                let mut src = Cursor::new(disk.as_slice());
                let len = disk.len() as u64;
                let Some(broken) = reassemble::locate_break(
                    &mut src,
                    ByteOffset::new(4 * block as u64),
                    Format::Jpeg,
                    len,
                    &mut scratch,
                )
                .expect("in-memory read") else {
                    return;
                };
                std::hint::black_box(
                    reassemble::bifragment(
                        &mut src,
                        broken,
                        len,
                        reassemble::Limits::default(),
                        &mut scratch,
                    )
                    .expect("in-memory read"),
                );
            });
        });
    }
    group.finish();
}

criterion_group!(benches, surface_passes, per_image, reassembly_search);
criterion_main!(benches);
