use argos::error::ArgosError;
use argos::io::{AlignedBuf, OutputSink, SourceDevice};
use std::io::Write;
use tempfile::tempdir;

fn write_file(path: &std::path::Path, data: &[u8]) {
    let mut file = std::fs::File::create(path).expect("create");
    file.write_all(data).expect("write");
    file.flush().expect("flush");
}

fn skip_on_direct_io_unsupported<T>(result: Result<T, ArgosError>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(ArgosError::Io(ref e))
            if e.raw_os_error() == Some(libc_einval())
                || e.raw_os_error() == Some(libc_eopnotsupp()) =>
        {
            None
        }
        Err(e) => panic!("unexpected error: {e:?}"),
    }
}

fn libc_einval() -> i32 {
    22
}

fn libc_eopnotsupp() -> i32 {
    95
}

#[test]
fn aligned_buf_allocates_at_requested_alignment() {
    let buf = AlignedBuf::with_capacity(4096, 4096).expect("alloc");
    assert_eq!(buf.capacity(), 4096);
    assert_eq!(buf.as_slice().len(), 0);
}

#[test]
fn aligned_buf_rejects_non_power_of_two_alignment() {
    let err = AlignedBuf::with_capacity(4096, 3).expect_err("must reject");
    assert!(matches!(err, ArgosError::Allocation { .. }));
}

#[test]
fn aligned_buf_rejects_zero_alignment() {
    let err = AlignedBuf::with_capacity(4096, 0).expect_err("must reject");
    assert!(matches!(err, ArgosError::Allocation { .. }));
}

#[test]
fn aligned_buf_writes_and_reads_back() {
    let mut buf = AlignedBuf::with_capacity(4096, 4096).expect("alloc");
    buf.set_len(4096);
    buf.as_mut_slice().fill(0xCD);
    assert!(buf.as_slice().iter().all(|&b| b == 0xCD));
    buf.clear();
    assert_eq!(buf.as_slice().len(), 0);
}

#[test]
fn output_sink_creates_directory_and_writes_files() {
    let dir = tempdir().expect("tempdir");
    let nested = dir.path().join("a").join("b").join("c");
    let sink = OutputSink::create(&nested).expect("create sink with nested dirs");
    let mut writer = sink.create_file("artifact.jpg").expect("create file");
    writer.write_all(b"hello").expect("write");
    drop(writer);

    let content = std::fs::read(nested.join("artifact.jpg")).expect("read back");
    assert_eq!(content, b"hello");
}

#[test]
fn source_device_opens_regular_file_or_returns_einval() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("device.bin");
    write_file(&path, &vec![0u8; 16 * 1024]);

    if let Some(dev) = skip_on_direct_io_unsupported(SourceDevice::open(&path)) {
        assert_eq!(dev.sector_size(), 4096);
        let size = dev.size().expect("size");
        assert_eq!(size, 16 * 1024);
    }
}

#[test]
fn source_device_rejects_missing_path() {
    let dir = tempdir().expect("tempdir");
    let missing = dir.path().join("nope");
    let err = SourceDevice::open(&missing).expect_err("must error");
    assert!(matches!(err, ArgosError::Io(_)));
}

#[test]
fn source_device_size_handles_zero_length() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("empty.bin");
    write_file(&path, &[]);

    if let Some(dev) = skip_on_direct_io_unsupported(SourceDevice::open(&path)) {
        let size = dev.size().expect("size");
        assert_eq!(size, 0);
    }
}
