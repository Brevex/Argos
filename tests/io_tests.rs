use argos::io::{AlignedBuffer, DiskReader, DiskScanner, BUFFER_SIZE};
use std::io::Write;
use tempfile::NamedTempFile;
#[test]
fn test_aligned_buffer_alignment() {
    let buffer = AlignedBuffer::new();
    let ptr = buffer.as_slice().as_ptr() as usize;
    assert_eq!(ptr % 4096, 0, "Buffer must be 4096-byte aligned");
}
#[test]
fn test_aligned_buffer_size() {
    let buffer = AlignedBuffer::new();
    assert_eq!(buffer.len(), BUFFER_SIZE);
}
#[test]
fn test_disk_reader_regular_file() {
    let mut temp = NamedTempFile::new().unwrap();
    let test_data = vec![0xAA; 4096];
    temp.write_all(&test_data).unwrap();
    temp.flush().unwrap();
    let mut reader = DiskReader::open_regular(temp.path()).unwrap();
    assert_eq!(reader.size(), 4096);
    let mut buffer = AlignedBuffer::new();
    let n = reader.read_at(0, &mut buffer).unwrap();
    assert_eq!(n, 4096);
}
#[test]
fn test_disk_scanner() {
    let mut temp = NamedTempFile::new().unwrap();
    let test_data = vec![0xBB; BUFFER_SIZE * 2 + 1000];
    temp.write_all(&test_data).unwrap();
    temp.flush().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut block_count = 0;
    while let Some((_, data)) = scanner.next_block().unwrap() {
        assert!(data.len() > 0);
        block_count += 1;
    }
    assert!(block_count >= 2, "Should read multiple blocks");
}
