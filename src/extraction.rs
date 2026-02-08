use crate::formats::jpeg::validate_jpeg;
use crate::formats::png::validate_png_header;
use crate::io::{AlignedBuffer, DiskReader};
use crate::types::{ImageFormat, ImageQualityScore, RecoveredFile};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MIN_JPEG_WRITTEN_BYTES: u64 = 2048;
const MIN_PNG_WRITTEN_BYTES: u64 = 4096;

pub fn extract_all(
    files: &[RecoveredFile],
    device_path: &Path,
    output_dir: &Path,
    progress_callback: Option<&dyn Fn(usize, usize)>,
) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut extracted = Vec::with_capacity(files.len());
    let mut reader = DiskReader::open(device_path)?;
    let total = files.len();

    for (i, file) in files.iter().enumerate() {
        if let Some(cb) = progress_callback {
            cb(i, total);
        }

        let filename = generate_filename(i, file.format);
        let output_path = output_dir.join(&filename);

        match extract_single(file, &mut reader, &output_path) {
            Ok(valid) => {
                if valid {
                    extracted.push(output_path);
                } else {
                    let _ = fs::remove_file(&output_path);
                }
            }
            Err(_) => {
                let _ = fs::remove_file(&output_path);
            }
        }
    }
    Ok(extracted)
}

pub fn extract_single(
    file: &RecoveredFile,
    reader: &mut DiskReader,
    output_path: &Path,
) -> io::Result<bool> {
    let mut out = File::create(output_path)?;
    let mut buffer = AlignedBuffer::new();
    let mut total_written = 0u64;

    for range in &file.fragments {
        let mut offset = range.start;

        while offset < range.end {
            let aligned_offset = offset & !4095;
            let skip = (offset - aligned_offset) as usize;

            let n = match reader.read_at(aligned_offset, &mut buffer) {
                Ok(n) => n,
                Err(e) => {
                    if matches!(e.raw_os_error(), Some(5) | Some(61)) {
                        offset += 4096;
                        continue;
                    }
                    return Err(e);
                }
            };

            if n == 0 {
                break;
            }

            let available = n.saturating_sub(skip);
            let remaining = (range.end - offset) as usize;
            let to_write = available.min(remaining);

            if to_write > 0 {
                out.write_all(&buffer.as_slice()[skip..skip + to_write])?;
                total_written += to_write as u64;
            }

            offset += to_write as u64;
        }
    }
    out.sync_all()?;

    let min_bytes = match file.format {
        ImageFormat::Jpeg => MIN_JPEG_WRITTEN_BYTES,
        ImageFormat::Png => MIN_PNG_WRITTEN_BYTES,
    };

    if total_written < min_bytes {
        return Ok(false);
    }

    Ok(score_recovered_image(output_path, file))
}

pub fn generate_filename(index: usize, format: ImageFormat) -> String {
    format!("recovered_{:06}.{}", index, format.extension())
}

fn score_recovered_image(path: &Path, recovered: &RecoveredFile) -> bool {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };

    match recovered.format {
        ImageFormat::Jpeg => score_recovered_jpeg(&data, recovered.header_entropy),
        ImageFormat::Png => score_recovered_png(&data, recovered.header_entropy),
    }
}

fn score_recovered_jpeg(data: &[u8], header_entropy: f32) -> bool {
    let info = match validate_jpeg(data) {
        Some(info) => info,
        None => return false,
    };

    let structure_valid =
        data.len() >= 4 && data[0..2] == [0xFF, 0xD8] && data[data.len() - 2..] == [0xFF, 0xD9];

    let score = ImageQualityScore::for_jpeg(
        header_entropy,
        info.width,
        info.height,
        &info.metadata,
        structure_valid,
    );

    score.meets_minimum()
}

fn score_recovered_png(data: &[u8], header_entropy: f32) -> bool {
    let info = match validate_png_header(data) {
        Some(info) => info,
        None => return false,
    };

    let score = ImageQualityScore::for_png(
        header_entropy,
        info.width,
        info.height,
        &info.metadata,
        info.idat_count,
    );

    score.meets_minimum()
}
