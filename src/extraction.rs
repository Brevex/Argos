use crate::io::{AlignedBuffer, DiskReader};
use crate::types::{ImageFormat, RecoveredFile};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn extract_all(
    files: &[RecoveredFile],
    device_path: &Path,
    output_dir: &Path,
) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut extracted = Vec::with_capacity(files.len());
    let mut reader = DiskReader::open(device_path)?;

    for (i, file) in files.iter().enumerate() {
        let filename = generate_filename(i, file.format);
        let output_path = output_dir.join(&filename);

        match extract_single(file, &mut reader, &output_path) {
            Ok(_) => {
                extracted.push(output_path);
            }
            Err(e) => {
                eprintln!("Warning: Failed to extract {}: {}", filename, e);
            }
        }
    }
    Ok(extracted)
}

pub fn extract_single(
    file: &RecoveredFile,
    reader: &mut DiskReader,
    output_path: &Path,
) -> io::Result<()> {
    let mut out = File::create(output_path)?;
    let mut buffer = AlignedBuffer::new();

    for range in &file.fragments {
        let mut offset = range.start;

        while offset < range.end {
            let aligned_offset = offset & !4095;
            let skip = (offset - aligned_offset) as usize;

            let n = reader.read_at(aligned_offset, &mut buffer)?;
            if n == 0 {
                break;
            }

            let available = n.saturating_sub(skip);
            let remaining = (range.end - offset) as usize;
            let to_write = available.min(remaining);

            if to_write > 0 {
                out.write_all(&buffer.as_slice()[skip..skip + to_write])?;
            }

            offset += to_write as u64;
        }
    }
    out.sync_all()?;
    Ok(())
}

pub fn generate_filename(index: usize, format: ImageFormat) -> String {
    format!("recovered_{:06}.{}", index, format.extension())
}

#[allow(dead_code)]
pub fn validate_extracted_file(path: &Path) -> io::Result<bool> {
    let data = fs::read(path)?;

    if data.len() < 10 {
        return Ok(false);
    }

    if data[0] == 0xFF && data[1] == 0xD8 {
        if data.len() >= 2 {
            let last_two = &data[data.len() - 2..];
            if last_two[0] == 0xFF && last_two[1] == 0xD9 {
                return Ok(true);
            }
        }
        return Ok(true);
    }

    if data.len() >= 8 && &data[0..8] == &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Ok(true);
    }

    Ok(false)
}
