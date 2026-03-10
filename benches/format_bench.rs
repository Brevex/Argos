use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use argos::core::{
    calculate_entropy, score_jpeg, score_png, JpegMetadata, PngMetadata, QuantizationQuality,
};
use argos::format::jpeg::validate_jpeg;
use argos::format::png::validate_png_header;

fn make_test_jpeg() -> Vec<u8> {
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]);
    jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x10]);
    jpeg.extend_from_slice(b"Exif\x00\x00");
    jpeg.extend_from_slice(&[0x00; 8]);
    jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    for _ in 0..64 {
        jpeg.push(10);
    }
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    jpeg.extend_from_slice(&[0x02, 0x00]);
    jpeg.extend_from_slice(&[0x02, 0x80]);
    jpeg.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
    jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x1F, 0x00]);
    for i in 0u8..28 {
        jpeg.push(i.wrapping_mul(37));
    }
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    while jpeg.len() < 55_000 {
        let idx = jpeg.len();
        jpeg.push(((idx.wrapping_mul(131).wrapping_add(17)) % 251) as u8);
    }
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    jpeg
}

fn make_test_png() -> Vec<u8> {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr_payload = Vec::new();
    ihdr_payload.extend_from_slice(&200u32.to_be_bytes());
    ihdr_payload.extend_from_slice(&200u32.to_be_bytes());
    ihdr_payload.push(8);
    ihdr_payload.push(2);
    ihdr_payload.extend_from_slice(&[0, 0, 0]);
    let ihdr = make_chunk(b"IHDR", &ihdr_payload);

    let mut idat_data = Vec::new();
    for i in 0..200 * 200 * 3 + 200 {
        idat_data.push(((i * 97 + 13) % 251) as u8);
    }
    let idat = make_chunk(b"IDAT", &idat_data);
    let iend = make_chunk(b"IEND", &[]);

    let mut png = Vec::new();
    png.extend_from_slice(&sig);
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&idat);
    png.extend_from_slice(&iend);
    png
}

fn make_chunk(chunk_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(payload);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(chunk_type);
    hasher.update(payload);
    let crc = hasher.finalize();
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

fn bench_validate_jpeg(c: &mut Criterion) {
    let jpeg = make_test_jpeg();
    c.bench_function("validate_jpeg_55KB", |b| {
        b.iter(|| validate_jpeg(black_box(&jpeg)));
    });
}

fn bench_validate_png(c: &mut Criterion) {
    let png = make_test_png();
    c.bench_function("validate_png_header", |b| {
        b.iter(|| validate_png_header(black_box(&png)));
    });
}

fn bench_entropy(c: &mut Criterion) {
    let mut group = c.benchmark_group("calculate_entropy");
    for &size in &[256, 1024, 4096, 65536] {
        let data: Vec<u8> = (0..size).map(|i| ((i * 131 + 17) % 251) as u8).collect();
        group.bench_with_input(BenchmarkId::new("random", size), &data, |b, data| {
            b.iter(|| calculate_entropy(black_box(data)));
        });
    }
    group.finish();
}

fn bench_score_jpeg(c: &mut Criterion) {
    let meta = JpegMetadata {
        has_exif: true,
        has_icc_profile: true,
        has_jfif: false,
        quantization_quality: QuantizationQuality::High,
        marker_count: 8,
        has_sos: true,
        scan_data_entropy: 7.5,
    };
    c.bench_function("score_jpeg_full_metadata", |b| {
        b.iter(|| score_jpeg(black_box(2048), black_box(1536), black_box(&meta)));
    });
}

fn bench_score_png(c: &mut Criterion) {
    let meta = PngMetadata {
        has_text_chunks: true,
        has_icc_profile: true,
        has_physical_dimensions: true,
        is_screen_resolution: false,
        chunk_variety: 6,
    };
    c.bench_function("score_png_rich_metadata", |b| {
        b.iter(|| {
            score_png(
                black_box(1920),
                black_box(1080),
                black_box(&meta),
                black_box(10),
            )
        });
    });
}

criterion_group!(
    benches,
    bench_validate_jpeg,
    bench_validate_png,
    bench_entropy,
    bench_score_jpeg,
    bench_score_png
);
criterion_main!(benches);
