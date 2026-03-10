use argos::io::{
    is_recoverable_io_error, zero_sector, AlignedBuffer, DiskReader, DiskScanner, ALIGNMENT_MASK,
    BUFFER_SIZE, SECTOR_SIZE,
};
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
    let reader = DiskReader::open_regular(temp.path()).unwrap();
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
        assert!(!data.is_empty());
        block_count += 1;
    }
    assert!(block_count >= 2, "Should read multiple blocks");
}

#[test]
fn test_aligned_buffer_zeroed_on_creation() {
    let buf = AlignedBuffer::new();
    assert!(
        buf.as_slice().iter().all(|&b| b == 0),
        "Buffer must be zeroed"
    );
}

#[test]
fn test_aligned_buffer_mutable_write() {
    let mut buf = AlignedBuffer::new();
    buf.as_mut_slice()[0] = 0xFF;
    buf.as_mut_slice()[BUFFER_SIZE - 1] = 0xAA;
    assert_eq!(buf.as_slice()[0], 0xFF);
    assert_eq!(buf.as_slice()[BUFFER_SIZE - 1], 0xAA);
}

#[test]
fn test_sector_size_constant() {
    assert_eq!(SECTOR_SIZE, 4096);
}

#[test]
fn test_buffer_size_constant() {
    assert_eq!(BUFFER_SIZE, 4 * 1024 * 1024);
}

#[test]
fn test_alignment_mask_aligns() {
    assert_eq!(4097u64 & ALIGNMENT_MASK, 4096u64);
    assert_eq!(4096u64 & ALIGNMENT_MASK, 4096u64);
    assert_eq!(0u64 & ALIGNMENT_MASK, 0u64);
    assert_eq!(8191u64 & ALIGNMENT_MASK, 4096u64);
}

#[test]
fn test_zero_sector_length() {
    assert_eq!(zero_sector().len(), SECTOR_SIZE);
}

#[test]
fn test_zero_sector_all_zeroes() {
    assert!(zero_sector().iter().all(|&b| b == 0));
}

#[test]
fn test_is_recoverable_io_error_eio() {
    let err = std::io::Error::from_raw_os_error(libc::EIO);
    assert!(is_recoverable_io_error(&err));
}

#[test]
fn test_is_recoverable_io_error_other_kind() {
    let err = std::io::Error::new(std::io::ErrorKind::Other, "unknown");
    assert!(is_recoverable_io_error(&err));
}

#[test]
fn test_is_not_recoverable_io_error_permission() {
    let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    assert!(!is_recoverable_io_error(&err));
}

#[test]
fn test_disk_reader_read_beyond_eof() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&[0xCC; 100]).unwrap();
    temp.flush().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut buf = AlignedBuffer::new();
    let n = reader.read_at(0, &mut buf).unwrap();
    assert_eq!(n, 100);
}

#[test]
fn test_disk_reader_read_at_offset_beyond_file() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&[0xDD; 100]).unwrap();
    temp.flush().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut buf = AlignedBuffer::new();
    let n = reader.read_at(200, &mut buf).unwrap();
    assert_eq!(n, 0, "Reading past EOF should return 0 bytes");
}

#[test]
fn test_disk_reader_try_clone() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&[0xEE; 4096]).unwrap();
    temp.flush().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let clone = reader.try_clone().unwrap();
    assert_eq!(clone.size(), reader.size());
}

#[test]
fn test_disk_reader_empty_file() {
    let temp = NamedTempFile::new().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    assert_eq!(reader.size(), 0);
}

#[test]
fn test_disk_scanner_empty_file() {
    let temp = NamedTempFile::new().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let block = scanner.next_block().unwrap();
    assert!(block.is_none(), "Empty file should yield no blocks");
}

#[test]
fn test_disk_scanner_exactly_one_buffer() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&vec![0xAA; BUFFER_SIZE]).unwrap();
    temp.flush().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);
    let mut count = 0;
    while scanner.next_block().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 1, "Exactly BUFFER_SIZE bytes should yield 1 block");
}

#[test]
fn test_disk_scanner_bad_sectors_initially_empty() {
    let temp = NamedTempFile::new().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let scanner = DiskScanner::new(reader);
    assert!(scanner.bad_sectors().is_empty());
}
