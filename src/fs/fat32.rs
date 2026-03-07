use super::FsHintMap;
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};

const DELETED_MARKER: u8 = 0xE5;
const MAX_DIR_ENTRIES: usize = 100_000;
const MAX_CHAIN_LENGTH: usize = 10_000;

#[derive(Debug, Clone)]
pub struct Fat32Info {
    pub partition_offset: u64,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub fat_size_sectors: u32,
    pub root_cluster: u32,
    pub cluster_size: u32,
    pub data_start: u64,
    pub fat_start: u64,
    pub total_data_clusters: u32,
}

impl Fat32Info {
    #[inline]
    pub fn cluster_to_offset(&self, cluster: u32) -> u64 {
        self.data_start + (cluster as u64 - 2) * self.cluster_size as u64
    }

    #[inline]
    pub fn is_valid_cluster(&self, cluster: u32) -> bool {
        cluster >= 2 && cluster < 0x0FFF_FFF8 && (cluster - 2) < self.total_data_clusters
    }
}

pub fn detect_fat32(
    reader: &DiskReader,
    partition_offset: u64,
    buffer: &mut AlignedBuffer,
) -> Option<Fat32Info> {
    let aligned = partition_offset & ALIGNMENT_MASK;
    let skip = (partition_offset - aligned) as usize;
    let n = reader.read_at(aligned, buffer).ok()?;
    if n < skip + 512 {
        return None;
    }
    let sector = &buffer.as_slice()[skip..skip + 512];

    if sector[510] != 0x55 || sector[511] != 0xAA {
        return None;
    }

    if &sector[82..87] != b"FAT32" {
        return None;
    }

    let bytes_per_sector = u16::from_le_bytes([sector[0x0B], sector[0x0C]]);
    let sectors_per_cluster = sector[0x0D];
    let reserved_sectors = u16::from_le_bytes([sector[0x0E], sector[0x0F]]);
    let num_fats = sector[0x10];

    if bytes_per_sector == 0
        || !bytes_per_sector.is_power_of_two()
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || num_fats == 0
    {
        return None;
    }

    let fat_size_sectors =
        u32::from_le_bytes([sector[0x24], sector[0x25], sector[0x26], sector[0x27]]);
    let root_cluster = u32::from_le_bytes([sector[0x2C], sector[0x2D], sector[0x2E], sector[0x2F]]);

    let total_sectors_32 =
        u32::from_le_bytes([sector[0x20], sector[0x21], sector[0x22], sector[0x23]]);

    let cluster_size = bytes_per_sector as u32 * sectors_per_cluster as u32;
    let fat_start = partition_offset + reserved_sectors as u64 * bytes_per_sector as u64;
    let data_start =
        fat_start + num_fats as u64 * fat_size_sectors as u64 * bytes_per_sector as u64;

    let data_sectors = total_sectors_32
        .saturating_sub(reserved_sectors as u32 + num_fats as u32 * fat_size_sectors);
    let total_data_clusters = data_sectors / sectors_per_cluster as u32;

    if root_cluster < 2 || root_cluster >= 0x0FFF_FFF8 || (root_cluster - 2) >= total_data_clusters {
        return None;
    }

    Some(Fat32Info {
        partition_offset,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        fat_size_sectors,
        root_cluster,
        cluster_size,
        data_start,
        fat_start,
        total_data_clusters,
    })
}

pub fn collect_dir_hints(
    reader: &DiskReader,
    info: &Fat32Info,
    buffer: &mut AlignedBuffer,
    hints: &mut FsHintMap,
) {
    let mut fat_cache = Vec::new();
    load_fat_cache(reader, info, buffer, &mut fat_cache);
    if fat_cache.is_empty() {
        return;
    }

    let mut dir_data = Vec::new();
    read_cluster_chain(
        reader,
        info,
        &fat_cache,
        info.root_cluster,
        buffer,
        &mut dir_data,
    );
    scan_directory_entries(&dir_data, info, &fat_cache, reader, buffer, hints);
}

fn load_fat_cache(
    reader: &DiskReader,
    info: &Fat32Info,
    buffer: &mut AlignedBuffer,
    cache: &mut Vec<u32>,
) {
    let fat_bytes = info.fat_size_sectors as u64 * info.bytes_per_sector as u64;
    let total_entries = (fat_bytes / 4) as usize;

    let entries = total_entries.min(16 * 1024 * 1024);
    cache.resize(entries, 0);

    let mut raw = vec![0u8; entries * 4];
    let mut pos = 0usize;
    let mut disk_offset = info.fat_start;

    while pos < raw.len() {
        let aligned = disk_offset & ALIGNMENT_MASK;
        let skip = (disk_offset - aligned) as usize;
        let n = match reader.read_at(aligned, buffer) {
            Ok(n) => n,
            Err(_) => break,
        };
        let available = n.saturating_sub(skip);
        if available == 0 {
            break;
        }
        let copy = available.min(raw.len() - pos);
        raw[pos..pos + copy].copy_from_slice(&buffer.as_slice()[skip..skip + copy]);
        pos += copy;
        disk_offset += copy as u64;
    }

    for (i, chunk) in raw.chunks_exact(4).enumerate() {
        cache[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) & 0x0FFF_FFFF;
    }
}

fn read_cluster_chain(
    reader: &DiskReader,
    info: &Fat32Info,
    fat: &[u32],
    start_cluster: u32,
    buffer: &mut AlignedBuffer,
    out: &mut Vec<u8>,
) {
    let cluster_bytes = info.cluster_size as usize;
    let mut cluster = start_cluster;
    let mut chain_len = 0usize;

    loop {
        if !info.is_valid_cluster(cluster) || chain_len >= MAX_CHAIN_LENGTH {
            break;
        }

        let offset = info.cluster_to_offset(cluster);
        let old_len = out.len();
        out.resize(old_len + cluster_bytes, 0);

        let mut disk_pos = offset;
        let mut dest_pos = old_len;
        let mut remaining = cluster_bytes;

        while remaining > 0 {
            let aligned = disk_pos & ALIGNMENT_MASK;
            let skip = (disk_pos - aligned) as usize;
            let n = match reader.read_at(aligned, buffer) {
                Ok(n) => n,
                Err(_) => break,
            };
            let available = n.saturating_sub(skip);
            if available == 0 {
                break;
            }
            let copy = available.min(remaining);
            out[dest_pos..dest_pos + copy].copy_from_slice(&buffer.as_slice()[skip..skip + copy]);
            dest_pos += copy;
            disk_pos += copy as u64;
            remaining -= copy;
        }

        let idx = cluster as usize;
        if idx >= fat.len() {
            break;
        }
        cluster = fat[idx];
        chain_len += 1;
    }
}

fn scan_directory_entries(
    dir_data: &[u8],
    info: &Fat32Info,
    fat: &[u32],
    _reader: &DiskReader,
    _buffer: &mut AlignedBuffer,
    hints: &mut FsHintMap,
) {
    let mut scanned = 0usize;

    for entry in dir_data.chunks_exact(32) {
        if scanned >= MAX_DIR_ENTRIES {
            break;
        }
        scanned += 1;

        if entry[0] == 0x00 {
            break;
        }

        if entry[0] != DELETED_MARKER {
            continue;
        }

        let attr = entry[0x0B];
        if attr == 0x0F {
            continue;
        }

        if attr & 0x10 != 0 || attr & 0x08 != 0 {
            continue;
        }

        let ext = &entry[8..11];
        let is_image = ext == b"JPG"
            || ext == b"jpg"
            || ext == b"JPE"
            || ext == b"jpe"
            || ext == b"PNG"
            || ext == b"png";

        if !is_image {
            continue;
        }

        let cluster_hi = u16::from_le_bytes([entry[0x14], entry[0x15]]) as u32;
        let cluster_lo = u16::from_le_bytes([entry[0x1A], entry[0x1B]]) as u32;
        let start_cluster = (cluster_hi << 16) | cluster_lo;
        let file_size =
            u32::from_le_bytes([entry[0x1C], entry[0x1D], entry[0x1E], entry[0x1F]]) as u64;

        if !info.is_valid_cluster(start_cluster) || file_size == 0 {
            continue;
        }

        let extents = follow_chain_extents(info, fat, start_cluster, file_size);
        if extents.is_empty() {
            continue;
        }

        let data_start = extents[0].0;
        let _ = hints.entry(data_start).or_insert(super::FsHint {
            data_start,
            data_size: file_size,
            extents,
        });
    }
}

fn follow_chain_extents(
    info: &Fat32Info,
    fat: &[u32],
    start_cluster: u32,
    max_size: u64,
) -> Vec<(u64, u64)> {
    let mut extents = Vec::new();
    let cluster_bytes = info.cluster_size as u64;
    let mut remaining = max_size;
    let mut cluster = start_cluster;
    let mut chain_len = 0usize;

    while info.is_valid_cluster(cluster) && remaining > 0 && chain_len < MAX_CHAIN_LENGTH {
        let offset = info.cluster_to_offset(cluster);
        let len = cluster_bytes.min(remaining);

        if let Some(last) = extents.last_mut() {
            let (last_off, last_len): &mut (u64, u64) = last;
            if *last_off + *last_len == offset {
                *last_len += len;
            } else {
                extents.push((offset, len));
            }
        } else {
            extents.push((offset, len));
        }

        remaining = remaining.saturating_sub(cluster_bytes);
        chain_len += 1;

        let idx = cluster as usize;
        if idx >= fat.len() {
            break;
        }
        cluster = fat[idx];
    }

    extents
}
