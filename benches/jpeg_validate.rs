use argos::validate::jpeg;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const DQT: u8 = 0xDB;
const DHT: u8 = 0xC4;
const SOF0: u8 = 0xC0;
const SOS: u8 = 0xDA;

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
    data.extend_from_slice(&[0xFF, SOI]);
    let mut dqt = vec![0x00];
    dqt.extend_from_slice(&[0x01; 64]);
    data.extend_from_slice(&segment(DQT, &dqt));
    data.extend_from_slice(&segment(DHT, &single_symbol_dht(0)));
    data.extend_from_slice(&segment(DHT, &single_symbol_dht(1)));
    let mut sof = Vec::new();
    sof.push(0x08);
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.extend_from_slice(&8u16.to_be_bytes());
    sof.push(0x01);
    sof.extend_from_slice(&[0x01, 0x11, 0x00]);
    data.extend_from_slice(&segment(SOF0, &sof));
    let mut sos = Vec::new();
    sos.push(0x01);
    sos.extend_from_slice(&[0x01, 0x00]);
    sos.extend_from_slice(&[0x00, 0x3F, 0x00]);
    data.extend_from_slice(&segment(SOS, &sos));
    data.push(0x00);
    data.push(0xFF);
    data.push(EOI);
    data
}

fn bench_jpeg_validate_baseline(c: &mut Criterion) {
    let data = minimal_baseline_jpeg();
    c.bench_function("jpeg_validate_baseline", |b| {
        b.iter(|| {
            let _ = jpeg::validate(black_box(&data));
        });
    });
}

fn bench_jpeg_validate_garbage(c: &mut Criterion) {
    let data = vec![0u8; 4096];
    c.bench_function("jpeg_validate_garbage", |b| {
        b.iter(|| {
            let _ = jpeg::validate(black_box(&data));
        });
    });
}

fn bench_jpeg_continuation_score(c: &mut Criterion) {
    let block: Vec<u8> = (0..=255).cycle().take(4096).collect();
    c.bench_function("jpeg_continuation_score_4k", |b| {
        b.iter(|| {
            let _ = jpeg::continuation_score(black_box(&block));
        });
    });
}

criterion_group!(
    benches,
    bench_jpeg_validate_baseline,
    bench_jpeg_validate_garbage,
    bench_jpeg_continuation_score
);
criterion_main!(benches);
