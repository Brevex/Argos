use argos::carve::ssd::Scanner;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn bench_ssd_scan_1mb(c: &mut Criterion) {
    let data = vec![0u8; 1024 * 1024];
    let mut scanner = Scanner::new().unwrap();

    c.bench_function("ssd_scan_1mb", |b| {
        b.iter(|| {
            let _ = scanner.scan_block(black_box(&data));
        });
    });
}

fn bench_ssd_scan_1mb_with_jpeg(c: &mut Criterion) {
    let mut data = vec![0u8; 1024 * 1024];
    data[100] = 0xFF;
    data[101] = 0xD8;
    data[200] = 0xFF;
    data[201] = 0xD9;
    let mut scanner = Scanner::new().unwrap();

    c.bench_function("ssd_scan_1mb_with_jpeg", |b| {
        b.iter(|| {
            let _ = scanner.scan_block(black_box(&data));
        });
    });
}

criterion_group!(benches, bench_ssd_scan_1mb, bench_ssd_scan_1mb_with_jpeg);
criterion_main!(benches);
