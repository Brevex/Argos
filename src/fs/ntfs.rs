use super::{read_exact, FsHintMap};
use crate::io::{AlignedBuffer, DiskReader, ALIGNMENT_MASK};

const NTFS_OEM_ID: &[u8; 8] = b"NTFS    ";
const FILE_SIGNATURE: &[u8; 4] = b"FILE";
const ATTR_TYPE_DATA: u32 = 0x80;
const ATTR_TYPE_END: u32 = 0xFFFF_FFFF;
const MAX_MFT_ENTRIES: usize = 500_000;

#[derive(Debug, Clone)]
pub struct NtfsInfo {
    pub partition_offset: u64,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub cluster_size: u32,
    pub mft_cluster: u64,
    pub mft_record_size: u32,
}

impl NtfsInfo {
    #[inline]
    pub fn cluster_to_offset(&self, cluster: u64) -> u64 {
        self.partition_offset + cluster * self.cluster_size as u64
    }
}

pub fn detect_ntfs(
    reader: &DiskReader,
    partition_offset: u64,
    buffer: &mut AlignedBuffer,
) -> Option<NtfsInfo> {
    let aligned = partition_offset & ALIGNMENT_MASK;
    let skip = (partition_offset - aligned) as usize;
    let n = reader.read_at(aligned, buffer).ok()?;
    if n < skip + 512 {
        return None;
    }
    let sector = &buffer.as_slice()[skip..skip + 512];

    if sector[3..11] != *NTFS_OEM_ID {
        return None;
    }

    let bytes_per_sector = u16::from_le_bytes([sector[0x0B], sector[0x0C]]);
    let sectors_per_cluster = sector[0x0D];

    if bytes_per_sector == 0
        || !bytes_per_sector.is_power_of_two()
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
    {
        return None;
    }

    let cluster_size = bytes_per_sector as u32 * sectors_per_cluster as u32;
    let mft_cluster = u64::from_le_bytes([
        sector[0x30],
        sector[0x31],
        sector[0x32],
        sector[0x33],
        sector[0x34],
        sector[0x35],
        sector[0x36],
        sector[0x37],
    ]);

    let record_size_raw = sector[0x40] as i8;
    let mft_record_size = if record_size_raw > 0 {
        record_size_raw as u32 * cluster_size
    } else {
        1u32 << (-record_size_raw as u32)
    };

    if !(256..=65536).contains(&mft_record_size) {
        return None;
    }

    Some(NtfsInfo {
        partition_offset,
        bytes_per_sector,
        sectors_per_cluster,
        cluster_size,
        mft_cluster,
        mft_record_size,
    })
}

pub fn collect_mft_hints(
    reader: &DiskReader,
    info: &NtfsInfo,
    buffer: &mut AlignedBuffer,
    hints: &mut FsHintMap,
) {
    let mft_offset = info.cluster_to_offset(info.mft_cluster);
    let record_size = info.mft_record_size as usize;

    let mut record_buf = vec![0u8; record_size];
    let mut entries_scanned = 0usize;

    loop {
        if entries_scanned >= MAX_MFT_ENTRIES {
            break;
        }

        let record_offset = mft_offset + (entries_scanned as u64 * record_size as u64);
        if record_offset + record_size as u64 > reader.size() {
            break;
        }

        if !read_exact(reader, record_offset, &mut record_buf, buffer) {
            break;
        }

        entries_scanned += 1;

        if record_buf[..4] != *FILE_SIGNATURE {
            continue;
        }

        let flags = u16::from_le_bytes([record_buf[0x16], record_buf[0x17]]);
        let in_use = flags & 0x01 != 0;
        let is_dir = flags & 0x02 != 0;

        if in_use || is_dir {
            continue;
        }

        let first_attr_offset = u16::from_le_bytes([record_buf[0x14], record_buf[0x15]]) as usize;
        if first_attr_offset >= record_size {
            continue;
        }

        if let Some(hint) = parse_data_attribute(&record_buf, first_attr_offset, info) {
            if hint.data_size > 0 && !hint.extents.is_empty() {
                hints.entry(hint.data_start).or_insert(hint);
            }
        }
    }
}

fn parse_data_attribute(
    record: &[u8],
    mut offset: usize,
    info: &NtfsInfo,
) -> Option<super::FsHint> {
    let record_len = record.len();

    loop {
        if offset + 4 > record_len {
            return None;
        }

        let attr_type = u32::from_le_bytes([
            record[offset],
            record[offset + 1],
            record[offset + 2],
            record[offset + 3],
        ]);

        if attr_type == ATTR_TYPE_END || attr_type == 0 {
            return None;
        }

        if offset + 8 > record_len {
            return None;
        }

        let attr_len = u32::from_le_bytes([
            record[offset + 4],
            record[offset + 5],
            record[offset + 6],
            record[offset + 7],
        ]) as usize;

        if attr_len < 16 || offset + attr_len > record_len {
            return None;
        }

        if attr_type == ATTR_TYPE_DATA {
            let non_resident = record[offset + 8];

            if non_resident == 0 {
                return None;
            }

            if offset + 0x40 > record_len {
                return None;
            }

            let data_size = u64::from_le_bytes([
                record[offset + 0x30],
                record[offset + 0x31],
                record[offset + 0x32],
                record[offset + 0x33],
                record[offset + 0x34],
                record[offset + 0x35],
                record[offset + 0x36],
                record[offset + 0x37],
            ]);

            let run_offset =
                u16::from_le_bytes([record[offset + 0x20], record[offset + 0x21]]) as usize;

            let runs_start = offset + run_offset;
            if runs_start >= offset + attr_len {
                return None;
            }

            let run_data = &record[runs_start..offset + attr_len];
            let extents = decode_data_runs(run_data, info)?;

            if extents.is_empty() {
                return None;
            }

            let data_start = extents[0].0;

            return Some(super::FsHint {
                data_start,
                data_size,
                extents,
            });
        }

        offset += attr_len;
    }
}

fn decode_data_runs(data: &[u8], info: &NtfsInfo) -> Option<Vec<(u64, u64)>> {
    let mut extents = Vec::new();
    let mut pos = 0usize;
    let mut prev_lcn: i64 = 0;

    while pos < data.len() {
        let header = data[pos];
        if header == 0 {
            break;
        }

        let len_size = (header & 0x0F) as usize;
        let off_size = ((header >> 4) & 0x0F) as usize;

        if len_size == 0 || pos + 1 + len_size + off_size > data.len() {
            break;
        }

        let mut run_len: u64 = 0;
        for i in 0..len_size {
            run_len |= (data[pos + 1 + i] as u64) << (i * 8);
        }

        if off_size == 0 {
            pos += 1 + len_size;
            continue;
        }

        let mut run_offset: i64 = 0;
        for i in 0..off_size {
            run_offset |= (data[pos + 1 + len_size + i] as i64) << (i * 8);
        }

        if off_size < 8 && (data[pos + len_size + off_size] & 0x80) != 0 {
            run_offset |= !0i64 << (off_size * 8);
        }

        let lcn = prev_lcn + run_offset;
        if lcn < 0 {
            return None;
        }
        prev_lcn = lcn;

        let abs_offset = info.cluster_to_offset(lcn as u64);
        let length = run_len * info.cluster_size as u64;

        extents.push((abs_offset, length));

        pos += 1 + len_size + off_size;
    }

    Some(extents)
}
