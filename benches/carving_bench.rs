use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use argos::core::{Fragment, FragmentKind, FragmentMap};
use argos::scan::scan_block;

fn make_disk_block(size: usize, seed: u8) -> Vec<u8> {
    let mut data = vec![0u8; size];
    for (i, b) in data.iter_mut().enumerate() {
        *b = ((i.wrapping_mul(131).wrapping_add(seed as usize * 7 + 17)) % 251) as u8;
    }
    data
}

fn make_jpeg_block(size: usize) -> Vec<u8> {
    let mut jpeg = vec![0u8; size];

    jpeg[0] = 0xFF;
    jpeg[1] = 0xD8;
    jpeg[2] = 0xFF;

    for i in 3..size.saturating_sub(2) {
        jpeg[i] = ((i.wrapping_mul(131).wrapping_add(17)) % 251) as u8;
    }

    if size >= 2 {
        jpeg[size - 2] = 0xFF;
        jpeg[size - 1] = 0xD9;
    }
    jpeg
}

fn bench_scan_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_block");

    for &size in &[4096, 65536, 1048576, 4194304] {
        let data = make_disk_block(size, 0);
        group.bench_with_input(BenchmarkId::new("noise", size), &data, |b, data| {
            b.iter(|| {
                let mut map = FragmentMap::new();
                scan_block(black_box(0), black_box(data), &mut map);
                map
            });
        });
    }

    let data = make_jpeg_block(4 * 1024 * 1024);
    group.bench_function("with_jpeg_4MB", |b| {
        b.iter(|| {
            let mut map = FragmentMap::new();
            scan_block(black_box(0), black_box(&data), &mut map);
            map
        });
    });

    group.finish();
}

fn bench_fragment_map_sort_dedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_map");

    for &count in &[100, 1000, 10000] {
        group.bench_with_input(BenchmarkId::new("sort_dedup", count), &count, |b, &n| {
            b.iter(|| {
                let mut map = FragmentMap::new();
                for i in 0..n {
                    let offset = ((n - i) * 4096) as u64;
                    map.push(Fragment::new(offset, FragmentKind::JpegHeader, 7.5));
                }

                for i in 0..(n / 10) {
                    map.push(Fragment::new(
                        (i * 4096) as u64,
                        FragmentKind::JpegHeader,
                        7.0,
                    ));
                }
                map.sort_by_offset();
                map.dedup();
                map
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_scan_block, bench_fragment_map_sort_dedup);
criterion_main!(benches);
