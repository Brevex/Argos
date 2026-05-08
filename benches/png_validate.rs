use argos::validate::png;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn valid_png() -> Vec<u8> {
    let signature = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut data = Vec::new();
    data.extend_from_slice(&signature);

    let ihdr_len = 13u32;
    let ihdr_type = b"IHDR";
    let ihdr_data = [0x00; 13];
    let ihdr_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(ihdr_type);
        h.update(&ihdr_data);
        h.finalize()
    };
    data.extend_from_slice(&ihdr_len.to_be_bytes());
    data.extend_from_slice(ihdr_type);
    data.extend_from_slice(&ihdr_data);
    data.extend_from_slice(&ihdr_crc.to_be_bytes());

    let idat_len = 10u32;
    let idat_type = b"IDAT";
    let idat_data = [0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01];
    let idat_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(idat_type);
        h.update(&idat_data);
        h.finalize()
    };
    data.extend_from_slice(&idat_len.to_be_bytes());
    data.extend_from_slice(idat_type);
    data.extend_from_slice(&idat_data);
    data.extend_from_slice(&idat_crc.to_be_bytes());

    let iend_len = 0u32;
    let iend_type = b"IEND";
    let iend_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(iend_type);
        h.finalize()
    };
    data.extend_from_slice(&iend_len.to_be_bytes());
    data.extend_from_slice(iend_type);
    data.extend_from_slice(&iend_crc.to_be_bytes());

    data
}

fn bench_png_validate(c: &mut Criterion) {
    let data = valid_png();

    c.bench_function("png_validate", |b| {
        b.iter(|| {
            let _ = png::validate(black_box(&data));
        });
    });
}

fn bench_png_validate_garbage(c: &mut Criterion) {
    let data = vec![0u8; 4096];

    c.bench_function("png_validate_garbage", |b| {
        b.iter(|| {
            let _ = png::validate(black_box(&data));
        });
    });
}

criterion_group!(benches, bench_png_validate, bench_png_validate_garbage);
criterion_main!(benches);
