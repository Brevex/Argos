pub mod ext4;
pub mod fat32;
pub mod ntfs;

use std::collections::HashMap;

use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};

#[derive(Debug, Clone)]
pub struct FsHint {
    pub data_start: u64,
    pub data_size: u64,
    pub extents: Vec<(u64, u64)>,
}

pub type FsHintMap = HashMap<u64, FsHint>;

pub fn collect_hints(reader: &DiskReader) -> FsHintMap {
    let mut hints = FsHintMap::new();
    let mut buffer = AlignedBuffer::new();

    let partition_offsets = detect_partition_starts(reader, &mut buffer);

    for part_start in &partition_offsets {
        if let Some(info) = ntfs::detect_ntfs(reader, *part_start, &mut buffer) {
            ntfs::collect_mft_hints(reader, &info, &mut buffer, &mut hints);
        } else if let Some(info) = fat32::detect_fat32(reader, *part_start, &mut buffer) {
            fat32::collect_dir_hints(reader, &info, &mut buffer, &mut hints);
        } else if let Some(info) = ext4::detect_ext4(reader, *part_start, &mut buffer) {
            ext4::collect_ext4_hints_deep(reader, &info, &mut buffer, &mut hints);
        }
    }

    hints
}

fn detect_partition_starts(reader: &DiskReader, buffer: &mut AlignedBuffer) -> Vec<u64> {
    let mut offsets = Vec::new();

    if reader.read_at(0, buffer).unwrap_or(0) < 512 {
        offsets.push(0);
        return offsets;
    }

    let sector = buffer.as_slice();

    if sector[510] != 0x55 || sector[511] != 0xAA {
        offsets.push(0);
        return offsets;
    }

    for i in 0..4 {
        let base = 446 + i * 16;
        let part_type = sector[base + 4];
        if part_type == 0x00 {
            continue;
        }
        let lba_start = u32::from_le_bytes([
            sector[base + 8],
            sector[base + 9],
            sector[base + 10],
            sector[base + 11],
        ]);
        if lba_start > 0 {
            offsets.push(lba_start as u64 * 512);
        }
    }

    if offsets.is_empty() {
        offsets.push(0);
    }

    offsets
}

pub(crate) fn read_exact(
    reader: &DiskReader,
    offset: u64,
    dest: &mut [u8],
    buffer: &mut AlignedBuffer,
) -> bool {
    let mut remaining = dest.len();
    let mut dest_pos = 0usize;
    let mut disk_pos = offset;

    while remaining > 0 {
        let aligned = disk_pos & ALIGNMENT_MASK;
        let skip = (disk_pos - aligned) as usize;

        let n = match reader.read_at(aligned, buffer) {
            Ok(n) => n,
            Err(_) => return false,
        };

        let available = n.saturating_sub(skip);
        if available == 0 {
            return false;
        }

        let copy = available.min(remaining);
        dest[dest_pos..dest_pos + copy].copy_from_slice(&buffer.as_slice()[skip..skip + copy]);

        dest_pos += copy;
        disk_pos += copy as u64;
        remaining -= copy;
    }

    true
}
