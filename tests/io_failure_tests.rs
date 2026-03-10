use argos::core::ExtractionError;
use argos::io::{
    is_recoverable_io_error, AlignedBuffer, DiskReader, DiskScanner, BUFFER_SIZE, SECTOR_SIZE,
};
use std::io::Write;
use tempfile::{tempdir, NamedTempFile};

#[cfg(unix)]
#[test]
fn test_open_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let path = dir.path().join("no_read.img");
    std::fs::write(&path, &[0u8; 4096]).unwrap();

    let perms = std::fs::Permissions::from_mode(0o000);
    std::fs::set_permissions(&path, perms).unwrap();

    let result = DiskReader::open_regular(&path);
    assert!(result.is_err(), "Should fail to open unreadable file");

    let perms = std::fs::Permissions::from_mode(0o644);
    std::fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn test_open_nonexistent_file() {
    let result = DiskReader::open_regular(std::path::Path::new(
        "/tmp/argos_test_nonexistent_file_12345.img",
    ));
    assert!(result.is_err(), "Should fail to open non-existent file");
}

#[test]
fn test_recoverable_eio() {
    let err = std::io::Error::from_raw_os_error(libc::EIO);
    assert!(is_recoverable_io_error(&err));
}

#[test]
fn test_recoverable_enodata() {
    let err = std::io::Error::from_raw_os_error(libc::ENODATA);
    assert!(is_recoverable_io_error(&err));
}

#[test]
fn test_recoverable_other_kind() {
    let err = std::io::Error::new(std::io::ErrorKind::Other, "generic");
    assert!(is_recoverable_io_error(&err));
}

#[test]
fn test_not_recoverable_not_found() {
    let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    assert!(!is_recoverable_io_error(&err));
}

#[test]
fn test_not_recoverable_broken_pipe() {
    let err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe");
    assert!(!is_recoverable_io_error(&err));
}

#[test]
fn test_extraction_error_write_zero_maps_to_disk_full() {
    let err = std::io::Error::new(std::io::ErrorKind::WriteZero, "");
    let ext_err = ExtractionError::from(err);
    assert!(matches!(ext_err, ExtractionError::DiskFull));
    assert!(ext_err.is_fatal());
}

#[cfg(unix)]
#[test]
fn test_extraction_error_enospc_maps_to_disk_full() {
    let err = std::io::Error::from_raw_os_error(libc::ENOSPC);
    let ext_err = ExtractionError::from(err);
    assert!(matches!(ext_err, ExtractionError::DiskFull));
}

#[cfg(unix)]
#[test]
fn test_extraction_error_eio_maps_to_device_disconnected() {
    let err = std::io::Error::from_raw_os_error(libc::EIO);
    let ext_err = ExtractionError::from(err);
    assert!(matches!(ext_err, ExtractionError::DeviceDisconnected));
    assert!(ext_err.is_fatal());
}

#[cfg(unix)]
#[test]
fn test_extraction_error_enxio_maps_to_device_disconnected() {
    let err = std::io::Error::from_raw_os_error(libc::ENXIO);
    let ext_err = ExtractionError::from(err);
    assert!(matches!(ext_err, ExtractionError::DeviceDisconnected));
}

#[test]
fn test_extraction_error_generic_io_not_fatal() {
    let err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
    let ext_err = ExtractionError::from(err);
    assert!(matches!(ext_err, ExtractionError::Io(_)));
    assert!(!ext_err.is_fatal());
}

#[test]
fn test_scanner_very_small_file() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(&[0x42; 10]).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let mut scanner = DiskScanner::new(reader);

    let mut blocks = 0;
    while let Some((_off, data)) = scanner.next_block().unwrap() {
        assert!(!data.is_empty());
        blocks += 1;
    }

    assert!(blocks <= 1);
}

#[test]
fn test_reader_concurrent_clones_read_same_data() {
    let mut temp = NamedTempFile::new().unwrap();
    let data = vec![0xAB; SECTOR_SIZE * 4];
    temp.write_all(&data).unwrap();
    temp.flush().unwrap();

    let reader = DiskReader::open_regular(temp.path()).unwrap();
    let clone1 = reader.try_clone().unwrap();
    let clone2 = reader.try_clone().unwrap();

    let mut buf1 = AlignedBuffer::new();
    let mut buf2 = AlignedBuffer::new();

    let n1 = clone1.read_at(0, &mut buf1).unwrap();
    let n2 = clone2.read_at(0, &mut buf2).unwrap();

    assert_eq!(n1, n2);
    assert_eq!(&buf1.as_slice()[..n1], &buf2.as_slice()[..n2]);
}

#[test]
fn test_reader_handles_file_shorter_than_expected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("shrink.img");

    std::fs::write(&path, &vec![0xCC; BUFFER_SIZE]).unwrap();
    let reader = DiskReader::open_regular(&path).unwrap();
    assert_eq!(reader.size(), BUFFER_SIZE as u64);

    std::fs::write(&path, &[0xDD; 100]).unwrap();

    let mut buf = AlignedBuffer::new();
    let n = reader.read_at(0, &mut buf).unwrap();

    assert!(n <= BUFFER_SIZE);
}
