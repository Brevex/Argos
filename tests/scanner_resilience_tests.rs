use argos::io::{DiskReader, DiskScanner, BUFFER_SIZE, SECTOR_SIZE};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_scanner_empty_file_yields_nothing() {
    let temp = NamedTempFile::new().unwrap();
    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    assert!(scanner.next_block().unwrap().is_none());
    assert!(scanner.bad_sectors().is_empty());
}

#[test]
fn test_scanner_one_sector_file() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&[0x42; SECTOR_SIZE]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let mut count = 0;
    while let Some((offset, data)) = scanner.next_block().unwrap() {
        assert_eq!(offset, 0);
        assert!(!data.is_empty());
        count += 1;
    }
    assert_eq!(count, 1);
}

#[test]
fn test_scanner_exact_buffer_boundary() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&vec![0xAA; BUFFER_SIZE]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let mut count = 0;
    while scanner.next_block().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 1, "Exactly BUFFER_SIZE should yield 1 block");
}

#[test]
fn test_scanner_buffer_plus_one_sector() {
    let mut temp = NamedTempFile::new().unwrap();
    let size = BUFFER_SIZE + SECTOR_SIZE;
    temp.write_all(&vec![0xBB; size]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let mut count = 0;
    while scanner.next_block().unwrap().is_some() {
        count += 1;
    }
    assert!(
        count >= 2,
        "BUFFER_SIZE + SECTOR_SIZE should yield ≥2 blocks"
    );
}

#[test]
fn test_scanner_preserves_data_content() {
    let mut temp = NamedTempFile::new().unwrap();
    let mut data = vec![0u8; SECTOR_SIZE * 2];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = ((i * 131 + 17) % 256) as u8;
    }
    temp.write_all(&data).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    if let Some((offset, block_data)) = scanner.next_block().unwrap() {
        assert_eq!(offset, 0);
        assert_eq!(&block_data[..SECTOR_SIZE], &data[..SECTOR_SIZE]);
    } else {
        panic!("Should yield at least one block");
    }
}

#[test]
fn test_scanner_into_reader_preserves_state() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&vec![0xCC; BUFFER_SIZE]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let expected_size = reader.size();
    let scanner = DiskScanner::new(reader);
    let reader_back = scanner.into_reader();

    assert_eq!(reader_back.size(), expected_size);
}

#[test]
fn test_scanner_disk_position_advances() {
    let mut temp = NamedTempFile::new().unwrap();
    let size = BUFFER_SIZE * 3;
    temp.write_all(&vec![0xDD; size]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let initial_pos = scanner.disk_position();
    scanner.next_block().unwrap();
    let after_one = scanner.disk_position();
    assert!(
        after_one > initial_pos,
        "Position should advance after reading a block"
    );
}
