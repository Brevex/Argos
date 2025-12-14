//! Local file writer implementation
//!
//! Writes recovered files to the local filesystem with optional PNG conversion.

use crate::domain::entities::{FileType, RecoveredFile};
use crate::domain::repositories::{
    FileWriterError, RecoveredFileWriter, WriteOptions, WriteResult,
};
use image::ImageFormat;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Local file system writer
///
/// Writes recovered files to the local filesystem.
/// Supports automatic PNG conversion for image files.
pub struct LocalFileWriter {
    output_dir: PathBuf,
    files_written: AtomicUsize,
    bytes_written: AtomicU64,
}

impl LocalFileWriter {
    /// Converts image data to PNG format
    fn convert_to_png(&self, data: &[u8], file_type: FileType) -> Result<Vec<u8>, FileWriterError> {
        let format = match file_type {
            FileType::Jpeg => ImageFormat::Jpeg,
            FileType::Png => return Ok(data.to_vec()), // Already PNG
            FileType::Gif => ImageFormat::Gif,
            FileType::Bmp => ImageFormat::Bmp,
            FileType::WebP => ImageFormat::WebP,
            FileType::Tiff => ImageFormat::Tiff,
            FileType::Unknown => {
                return Err(FileWriterError::ConversionError(
                    "Cannot convert unknown file type".to_string(),
                ));
            }
        };

        // Load image
        let img = image::load_from_memory_with_format(data, format)
            .map_err(|e| FileWriterError::ConversionError(e.to_string()))?;

        // Encode as PNG
        let mut png_data = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png_data), ImageFormat::Png)
            .map_err(|e| FileWriterError::ConversionError(e.to_string()))?;

        Ok(png_data)
    }

    /// Generates the output path for a file
    fn output_path(&self, file: &RecoveredFile, options: &WriteOptions) -> PathBuf {
        let mut path = self.output_dir.clone();

        // Organize by type if requested
        if options.organize_by_type {
            path.push(file.file_type().extension());
        }

        // Generate filename
        let extension = if options.convert_to_png {
            "png"
        } else {
            file.file_type().extension()
        };
        let filename = format!("{}_{:06}.{}", options.filename_prefix, file.id(), extension);
        path.push(filename);

        path
    }
}

impl RecoveredFileWriter for LocalFileWriter {
    fn new(output_dir: &Path) -> Result<Self, FileWriterError> {
        // Create output directory if it doesn't exist
        if !output_dir.exists() {
            fs::create_dir_all(output_dir).map_err(|e| {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    FileWriterError::PermissionDenied(output_dir.display().to_string())
                } else {
                    FileWriterError::IoError(e)
                }
            })?;
        }

        Ok(Self {
            output_dir: output_dir.to_path_buf(),
            files_written: AtomicUsize::new(0),
            bytes_written: AtomicU64::new(0),
        })
    }

    fn write(
        &self,
        file: &RecoveredFile,
        options: &WriteOptions,
    ) -> Result<WriteResult, FileWriterError> {
        let output_path = self.output_path(file, options);

        // Create parent directories
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Check if file exists
        if output_path.exists() && !options.overwrite {
            return Err(FileWriterError::FileExists(
                output_path.display().to_string(),
            ));
        }

        // Get data (possibly converted)
        let (data, was_converted) = if options.convert_to_png && file.file_type() != FileType::Png {
            match self.convert_to_png(file.data(), file.file_type()) {
                Ok(png_data) => (png_data, true),
                Err(e) => {
                    log::warn!("PNG conversion failed for file {}: {}", file.id(), e);
                    // Fall back to original data
                    (file.data().to_vec(), false)
                }
            }
        } else {
            (file.data().to_vec(), false)
        };

        // Write to file
        let mut output_file = File::create(&output_path)?;
        output_file.write_all(&data)?;
        output_file.sync_all()?;

        let saved_size = data.len() as u64;

        // Update counters
        self.files_written.fetch_add(1, Ordering::Relaxed);
        self.bytes_written.fetch_add(saved_size, Ordering::Relaxed);

        Ok(WriteResult {
            file_id: file.id(),
            saved_path: output_path,
            saved_size,
            was_converted,
        })
    }

    fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    fn files_written(&self) -> usize {
        self.files_written.load(Ordering::Relaxed)
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }
}
