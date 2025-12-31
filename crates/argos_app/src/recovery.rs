//! File recovery module for reconstructing files from scan events.
//!
//! This module processes header/footer events and extracts complete files
//! using a stack-based approach to handle nested files (e.g., JPEG thumbnails).

use crate::engine::ScanEvent;
use argos_core::{BlockSource, FileType};
use argos_io::DiskReader;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const MIN_FILE_SIZE: u64 = 64 * 1024;
const EXTRACTION_BUFFER_SIZE: usize = 64 * 1024;
const MIN_RESOLUTION: usize = 600;
const FALLBACK_SIZE: u64 = 500 * 1024;

#[derive(Debug, Clone, Copy)]
struct Candidate {
    offset_start: u64,
    file_type: FileType,
}

/// Manages file recovery using a stack-based approach.
/// The stack structure correctly handles nested files like JPEG thumbnails:
/// - Header1 found → push
/// - Header2 (thumbnail) found → push
/// - Footer2 found → pop, save thumbnail
/// - Footer1 found → pop, save main image
pub struct RecoveryManager {
    stack: Vec<Candidate>,
    reader: DiskReader,
    output_dir: PathBuf,
    files_recovered: u64,
    files_skipped: u64,
}

impl RecoveryManager {
    /// Creates a new RecoveryManager.
    pub fn new(device_path: &str, output_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(output_dir)?;
        let reader = DiskReader::new(device_path)?;

        Ok(Self {
            stack: Vec::new(),
            reader,
            output_dir: output_dir.to_path_buf(),
            files_recovered: 0,
            files_skipped: 0,
        })
    }

    /// Processes a scan event, potentially recovering a file.
    pub fn process_event(&mut self, event: &ScanEvent) {
        match event {
            ScanEvent::HeaderFound { offset, ftype } => {
                self.stack.push(Candidate {
                    offset_start: *offset,
                    file_type: *ftype,
                });
            }
            ScanEvent::FooterFound { offset, ftype } => {
                let should_pop = self
                    .stack
                    .last()
                    .map(|c| c.file_type == *ftype)
                    .unwrap_or(false);

                if should_pop {
                    let candidate = self.stack.pop().expect("stack verified non-empty");
                    self.attempt_recovery(&candidate, *offset, *ftype);
                }
            }
            ScanEvent::WorkerDone => {}
        }
    }

    fn attempt_recovery(&mut self, candidate: &Candidate, footer_offset: u64, ftype: FileType) {
        let footer_size = ftype.footer_size();
        let file_size = footer_offset
            .saturating_sub(candidate.offset_start)
            .saturating_add(footer_size);

        if file_size < MIN_FILE_SIZE {
            self.files_skipped += 1;
            return;
        }

        if file_size > MAX_FILE_SIZE {
            self.files_skipped += 1;
            return;
        }

        let mut header_buf = vec![0u8; 4096];
        let header_read = match self
            .reader
            .read_chunk(candidate.offset_start, &mut header_buf)
        {
            Ok(n) => n,
            Err(e) => {
                eprintln!(
                    "[Recovery] Failed to read header at offset {}: {}",
                    candidate.offset_start, e
                );
                self.files_skipped += 1;
                return;
            }
        };

        if header_read > 0 {
            if let Some((width, height)) =
                argos_core::get_image_dimensions(&header_buf[..header_read])
            {
                if width < MIN_RESOLUTION || height < MIN_RESOLUTION {
                    self.files_skipped += 1;
                    return;
                }
            } else {
                if file_size < FALLBACK_SIZE {
                    self.files_skipped += 1;
                    return;
                }
            }
        }

        let extension = ftype.extension();
        let filename = format!(
            "{}_{:016X}.{}",
            ftype.name(),
            candidate.offset_start,
            extension
        );
        let output_path = self.output_dir.join(&filename);

        match self.save_file(candidate.offset_start, file_size, &output_path) {
            Ok(()) => {
                self.files_recovered += 1;
            }
            Err(e) => {
                eprintln!(
                    "[Recovery] Failed to save file {}: {}",
                    output_path.display(),
                    e
                );
                self.files_skipped += 1;
            }
        }
    }

    fn save_file(&mut self, start: u64, size: u64, output_path: &Path) -> anyhow::Result<()> {
        let file = File::create(output_path)?;
        let mut writer = BufWriter::with_capacity(131_072, file);
        let mut remaining = size;
        let mut offset = start;
        let mut buffer = vec![0u8; EXTRACTION_BUFFER_SIZE];

        while remaining > 0 {
            let to_read = std::cmp::min(remaining as usize, EXTRACTION_BUFFER_SIZE);
            let bytes_read = self.reader.read_chunk(offset, &mut buffer[..to_read])?;

            if bytes_read == 0 {
                break;
            }

            writer.write_all(&buffer[..bytes_read])?;

            offset += bytes_read as u64;
            remaining -= bytes_read as u64;
        }

        writer.flush()?;
        Ok(())
    }

    pub fn files_recovered(&self) -> u64 {
        self.files_recovered
    }

    pub fn files_skipped(&self) -> u64 {
        self.files_skipped
    }

    pub fn pending_candidates(&self) -> usize {
        self.stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_stack_based_matching() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let valid_size = 512 * 1024;
        let mut data = Vec::with_capacity(valid_size + 200);

        data.extend_from_slice(&[0x00; 100]);
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE0; valid_size]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        data.extend_from_slice(&[0x00; 100]);

        temp_file.write_all(&data).unwrap();
        temp_file.flush().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: 524391,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 0);
        assert_eq!(manager.files_recovered(), 1);

        let recovered_files: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(recovered_files.len(), 1);
    }

    #[test]
    fn test_nested_files_thumbnail() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let chunk_size = 512 * 1024;
        let mut data = Vec::with_capacity(chunk_size * 3);

        data.extend_from_slice(&[0x00; 100]);
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE0; 1024]);

        let h2_offset = 100 + 3 + 1024;
        data.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        data.extend_from_slice(&vec![0xE1; chunk_size]);

        let f2_offset = h2_offset + 3 + chunk_size as u64;
        data.extend_from_slice(&[0xFF, 0xD9]);

        data.extend_from_slice(&vec![0xE2; chunk_size]);

        let f1_offset = f2_offset + 2 + chunk_size as u64;
        data.extend_from_slice(&[0xFF, 0xD9]);
        data.extend_from_slice(&[0x00; 100]);

        temp_file.write_all(&data).unwrap();
        temp_file.flush().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);

        manager.process_event(&ScanEvent::HeaderFound {
            offset: h2_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 2);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f2_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 1);
        assert_eq!(manager.files_recovered(), 1);

        manager.process_event(&ScanEvent::FooterFound {
            offset: f1_offset,
            ftype: FileType::Jpeg,
        });
        assert_eq!(manager.stack.len(), 0);
        assert_eq!(manager.files_recovered(), 2);
    }

    #[test]
    fn test_orphan_footer_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Jpeg,
        });

        assert_eq!(manager.stack.len(), 0);
        assert_eq!(manager.files_recovered(), 0);
    }

    #[test]
    fn test_mismatched_types_ignored() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut manager =
            RecoveryManager::new(temp_file.path().to_str().unwrap(), temp_dir.path()).unwrap();

        manager.process_event(&ScanEvent::HeaderFound {
            offset: 0,
            ftype: FileType::Jpeg,
        });

        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Png,
        });
        manager.process_event(&ScanEvent::FooterFound {
            offset: 100,
            ftype: FileType::Png,
        });

        assert_eq!(manager.stack.len(), 1);
        assert_eq!(manager.files_recovered(), 0);
    }
}
