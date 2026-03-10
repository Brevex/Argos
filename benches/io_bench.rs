use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use argos::io::{AlignedBuffer, DiskReader, DiskScanner, BUFFER_SIZE};
use std::io::Write;
use tempfile::NamedTempFile;

fn create_temp_file(size: usize) -> NamedTempFile {
    let mut temp = NamedTempFile::new().unwrap();
    let mut remaining = size;
    let chunk_size = 1024 * 1024; // write 1MB at a time
    while remaining > 0 {
        let n = remaining.min(chunk_size);
        let data: Vec<u8> = (0..n).map(|i| ((i * 131 + 17) % 251) as u8).collect();
        temp.write_all(&data).unwrap();
        remaining -= n;
    }
    temp.flush().unwrap();
    temp
}

fn bench_aligned_buffer_creation(c: &mut Criterion) {
    c.bench_function("AlignedBuffer::new", |b| {
        b.iter(|| {
            let buf = AlignedBuffer::new();
            black_box(&buf);
        });
    });
}

fn bench_disk_reader_read_at(c: &mut Criterion) {
    let temp = create_temp_file(BUFFER_SIZE * 2);
    let reader = DiskReader::open_regular(temp.path()).unwrap();

    let mut group = c.benchmark_group("DiskReader_read_at");
    for &offset in &[0usize, BUFFER_SIZE / 2, BUFFER_SIZE] {
        group.bench_with_input(BenchmarkId::new("offset", offset), &offset, |b, &off| {
            let mut buf = AlignedBuffer::new();
            b.iter(|| {
                let _ = reader.read_at(black_box(off as u64), &mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_disk_scanner_throughput(c: &mut Criterion) {
    let temp = create_temp_file(BUFFER_SIZE * 4);

    c.bench_function("DiskScanner_full_scan_16MB", |b| {
        b.iter(|| {
            let reader = DiskReader::open_regular(temp.path()).unwrap();
            let mut scanner = DiskScanner::new(reader);
            let mut total = 0usize;
            while let Some((_, data)) = scanner.next_block().unwrap() {
                total += data.len();
            }
            black_box(total);
        });
    });
}

criterion_group!(
    benches,
    bench_aligned_buffer_creation,
    bench_disk_reader_read_at,
    bench_disk_scanner_throughput
);
criterion_main!(benches);
