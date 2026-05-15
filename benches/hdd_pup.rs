use argos::carve::ImageFormat;
use argos::carve::hdd::pup::{self, Seed};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

const BLOCK_SIZE: usize = 4096;

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(0xFF);
    out.push(marker);
    let len = (body.len() + 2) as u16;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn single_symbol_dht(class: u8) -> Vec<u8> {
    let mut body = Vec::with_capacity(18);
    body.push(class << 4);
    body.push(0x01);
    body.extend_from_slice(&[0u8; 15]);
    body.push(0x00);
    body
}

fn minimal_baseline_jpeg() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xFF, 0xD8]);
    let mut dqt = vec![0x00];
    dqt.extend_from_slice(&[0x01; 64]);
    data.extend_from_slice(&segment(0xDB, &dqt));
    data.extend_from_slice(&segment(0xC4, &single_symbol_dht(0)));
    data.extend_from_slice(&segment(0xC4, &single_symbol_dht(1)));
    let mut sof = Vec::new();
    sof.push(0x08);
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.push(0x01);
    sof.extend_from_slice(&[0x01, 0x11, 0x00]);
    data.extend_from_slice(&segment(0xC0, &sof));
    let mut sos = Vec::new();
    sos.push(0x01);
    sos.extend_from_slice(&[0x01, 0x00]);
    sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
    data.extend_from_slice(&segment(0xDA, &sos));
    data.push(0x00);
    data.push(0xFF);
    data.push(0xD9);
    data
}

fn bench_pup_single_seed(c: &mut Criterion) {
    let jpeg = minimal_baseline_jpeg();
    let mut data = vec![0xABu8; BLOCK_SIZE * 4];
    let seed_block = 1usize;
    data[seed_block * BLOCK_SIZE..seed_block * BLOCK_SIZE + jpeg.len()].copy_from_slice(&jpeg);
    let seeds = vec![Seed {
        block_index: seed_block as u64,
        format: ImageFormat::Jpeg,
    }];

    c.bench_function("pup_single_seed", |b| {
        b.iter(|| {
            let _ = pup::run(black_box(&seeds), black_box(&data), BLOCK_SIZE, 10_000);
        });
    });
}

fn bench_pup_many_seeds(c: &mut Criterion) {
    let jpeg = minimal_baseline_jpeg();
    let mut data = vec![0xABu8; BLOCK_SIZE * 32];
    let mut seeds = Vec::new();
    for seed_block in (1..32).step_by(4) {
        let start = seed_block * BLOCK_SIZE;
        data[start..start + jpeg.len()].copy_from_slice(&jpeg);
        seeds.push(Seed {
            block_index: seed_block as u64,
            format: ImageFormat::Jpeg,
        });
    }

    c.bench_function("pup_eight_seeds", |b| {
        b.iter(|| {
            let _ = pup::run(black_box(&seeds), black_box(&data), BLOCK_SIZE, 10_000);
        });
    });
}

criterion_group!(benches, bench_pup_single_seed, bench_pup_many_seeds);
criterion_main!(benches);
