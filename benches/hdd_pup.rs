use std::io::Write;

use argos::carve::hdd::pup;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::tempdir;

fn minimal_jpeg() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xFF, 0xD8]);

    let mut dht = Vec::new();
    dht.push(0x00);
    dht.extend_from_slice(&[0x01; 16]);
    dht.push(0x00);
    let dht_len = (dht.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC4);
    data.extend_from_slice(&dht_len.to_be_bytes());
    data.extend_from_slice(&dht);

    let mut dht_ac = Vec::new();
    dht_ac.push(0x10);
    dht_ac.extend_from_slice(&[0x01; 16]);
    dht_ac.push(0x00);
    let dht_ac_len = (dht_ac.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC4);
    data.extend_from_slice(&dht_ac_len.to_be_bytes());
    data.extend_from_slice(&dht_ac);

    let mut sof = Vec::new();
    sof.push(0x08);
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.push(0x01);
    sof.extend_from_slice(&[0x01, 0x11, 0x00]);
    let sof_len = (sof.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xC0);
    data.extend_from_slice(&sof_len.to_be_bytes());
    data.extend_from_slice(&sof);

    let mut sos = Vec::new();
    sos.push(0x01);
    sos.extend_from_slice(&[0x01, 0x00]);
    sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
    let sos_len = (sos.len() + 2) as u16;
    data.push(0xFF);
    data.push(0xDA);
    data.extend_from_slice(&sos_len.to_be_bytes());
    data.extend_from_slice(&sos);

    data.push(0x00);
    data.push(0x00);
    data.push(0xFF);
    data.push(0xD9);
    data
}

fn bench_pup_single_seed(c: &mut Criterion) {
    let jpeg = minimal_jpeg();
    let garbage = vec![0xABu8; 8192];
    let dir = tempdir().unwrap();
    let path = dir.path().join("device.bin");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&garbage).unwrap();
        file.write_all(&jpeg).unwrap();
        file.write_all(&garbage).unwrap();
    }

    let file = std::fs::File::open(&path).unwrap();
    let mmap = unsafe { memmap2::MmapOptions::new().map(&file).unwrap() };
    let seeds = vec![pup::Seed {
        block_index: (garbage.len() / 4096) as u64,
        format: argos::carve::ImageFormat::Jpeg,
    }];

    c.bench_function("pup_single_seed", |b| {
        b.iter(|| {
            let _ = pup::run(
                black_box(&seeds),
                black_box(&mmap),
                4096,
                10_000,
            );
        });
    });
}

criterion_group!(benches, bench_pup_single_seed);
criterion_main!(benches);
