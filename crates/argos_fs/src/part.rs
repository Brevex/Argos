//! Partition tables: MBR and GPT, including the backup GPT.
//!
//! On-disk layouts implemented: the classic MBR (bootstrap + four 16-byte
//! entries + `55 AA`), and UEFI GPT (header at LBA 1, entry array, backup
//! header at the last LBA). GPT headers are validated by signature and CRC32
//! before any field is trusted; when the primary header is damaged the backup
//! is used — that asymmetry is exactly what recovers a wiped LBA 1.

use std::io::{Read, Seek};

use argos_core::geometry::{ByteOffset, ByteRange};

use crate::bytes::{read_at, u16_le, u32_le, u64_le};
use crate::{FsError, FsKind};

/// MBR boot signature at offset 510. Source: classic PC/AT layout.
const MBR_BOOT_SIG: u16 = 0xAA55;

/// Offset of the four MBR partition entries. Source: PC/AT MBR layout.
const MBR_ENTRIES_AT: usize = 446;

/// MBR partition type meaning "protective GPT". Source: UEFI spec §5.2.3.
const MBR_TYPE_PROTECTIVE_GPT: u8 = 0xEE;

/// GPT header signature. Source: UEFI spec §5.3.2.
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";

/// Sector size GPT structures are addressed in for this parser. GPT is
/// LBA-addressed; 512 is the addressing unit of every raw image this tool
/// ingests (4Kn devices arrive via the device HAL with real geometry later).
const SECTOR: u64 = 512;

/// Cap on GPT partition entries walked. The spec minimum reserve is 128
/// entries; a corrupt count above 1024 is rejected rather than walked.
const MAX_GPT_ENTRIES: u32 = 1024;

/// Cap on the size of one GPT partition entry. The spec fixes 128 bytes and
/// allows growth in future revisions; anything above one sector is a corrupt
/// field, and without this bound `entry_count * entry_size` is medium-sized
/// (A-BOUNDED-ALLOC).
const MAX_GPT_ENTRY_BYTES: u32 = 512;

/// A partition slot from MBR or GPT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// Byte range of the partition on the medium.
    pub range: ByteRange,
    /// Filesystem family hinted by the entry type, when recognizable.
    pub kind_hint: Option<FsKind>,
}

/// Outcome of reading the partition tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tables {
    /// Partitions from the table that validated (GPT preferred over MBR).
    pub partitions: Vec<Partition>,
    /// Whether the *backup* GPT supplied the data because the primary failed
    /// validation — itself evidence of damage worth reporting.
    pub used_backup_gpt: bool,
}

/// Reads MBR and GPT (primary, then backup) from `src` of `len` bytes.
///
/// # Errors
///
/// Fails only on I/O faults; corrupt or absent tables yield an empty result.
pub fn scan<R: Read + Seek>(src: &mut R, len: u64) -> Result<Tables, FsError> {
    let mut buf = Vec::new();

    // MBR at LBA 0.
    if !read_at(src, 0, 512, &mut buf)? {
        return Ok(Tables::default());
    }
    let Some(mbr) = parse_mbr(&buf) else {
        return Ok(Tables::default());
    };

    let gpt_protective = mbr
        .iter()
        .any(|entry| entry.type_byte == MBR_TYPE_PROTECTIVE_GPT);

    if gpt_protective {
        // Primary GPT header at LBA 1.
        if read_at(src, SECTOR, 512, &mut buf)?
            && let Some(partitions) = parse_gpt(src, &buf.clone(), len)?
        {
            return Ok(Tables {
                partitions,
                used_backup_gpt: false,
            });
        }
        // Backup GPT header at the last LBA.
        let backup_at = len.saturating_sub(SECTOR);
        if read_at(src, backup_at, 512, &mut buf)?
            && let Some(partitions) = parse_gpt(src, &buf.clone(), len)?
        {
            return Ok(Tables {
                partitions,
                used_backup_gpt: true,
            });
        }
        return Ok(Tables::default());
    }

    // Plain MBR partitions.
    let partitions = mbr
        .iter()
        .filter(|entry| entry.type_byte != 0 && entry.sectors > 0)
        .filter_map(|entry| {
            let start = entry.start_lba.checked_mul(SECTOR)?;
            let bytes = entry.sectors.checked_mul(SECTOR)?;
            Some(Partition {
                range: ByteRange::new(ByteOffset::new(start), bytes),
                kind_hint: mbr_kind_hint(entry.type_byte),
            })
        })
        .collect();
    Ok(Tables {
        partitions,
        used_backup_gpt: false,
    })
}

struct MbrEntry {
    type_byte: u8,
    start_lba: u64,
    sectors: u64,
}

fn parse_mbr(sector: &[u8]) -> Option<Vec<MbrEntry>> {
    if u16_le(sector, 510)? != MBR_BOOT_SIG {
        return None;
    }
    let mut entries = Vec::with_capacity(4);
    for slot in 0..4 {
        let base = MBR_ENTRIES_AT + slot * 16;
        entries.push(MbrEntry {
            type_byte: *sector.get(base + 4)?,
            start_lba: u64::from(u32_le(sector, base + 8)?),
            sectors: u64::from(u32_le(sector, base + 12)?),
        });
    }
    Some(entries)
}

/// Filesystem hint from an MBR type byte; only types Argos recovers.
fn mbr_kind_hint(type_byte: u8) -> Option<FsKind> {
    match type_byte {
        // FAT32 (CHS and LBA variants).
        0x0B | 0x0C => Some(FsKind::Fat32),
        // NTFS/exFAT share 0x07; the volume anchor disambiguates later.
        0x07 => Some(FsKind::Ntfs),
        // Linux native.
        0x83 => Some(FsKind::Ext4),
        _ => None,
    }
}

/// Validates a GPT header and reads its entry array; `None` when anything
/// fails validation (the caller then tries the backup header).
fn parse_gpt<R: Read + Seek>(
    src: &mut R,
    header: &[u8],
    len: u64,
) -> Result<Option<Vec<Partition>>, FsError> {
    if header.get(..8).is_none_or(|sig| sig != GPT_SIGNATURE) {
        return Ok(None);
    }
    let Some(fields) = GptHeader::parse(header) else {
        return Ok(None);
    };
    if !fields.header_crc_ok(header) {
        return Ok(None);
    }

    let Some(array_bytes) = fields
        .entry_count
        .checked_mul(fields.entry_size)
        .map(u64::from)
    else {
        return Ok(None);
    };
    let Some(array_at) = fields.entries_lba.checked_mul(SECTOR) else {
        return Ok(None);
    };
    if array_at.saturating_add(array_bytes) > len {
        return Ok(None);
    }
    let Ok(array_len) = usize::try_from(array_bytes) else {
        return Ok(None);
    };

    let mut array = Vec::new();
    if !read_at(src, array_at, array_len, &mut array)? {
        return Ok(None);
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(&array);
    if crc.finalize() != fields.array_crc {
        return Ok(None);
    }

    let entry_size = fields.entry_size as usize;
    let mut partitions = Vec::new();
    for entry in array.chunks_exact(entry_size) {
        // A zero type GUID marks an unused slot. Source: UEFI spec §5.3.3.
        if entry
            .get(..16)
            .is_none_or(|guid| guid.iter().all(|&b| b == 0))
        {
            continue;
        }
        let (Some(first), Some(last)) = (u64_le(entry, 32), u64_le(entry, 40)) else {
            continue;
        };
        if last < first {
            continue;
        }
        let (Some(start), Some(bytes)) = (
            first.checked_mul(SECTOR),
            (last - first)
                .checked_add(1)
                .and_then(|s| s.checked_mul(SECTOR)),
        ) else {
            continue;
        };
        partitions.push(Partition {
            range: ByteRange::new(ByteOffset::new(start), bytes),
            kind_hint: None,
        });
    }
    Ok(Some(partitions))
}

struct GptHeader {
    entries_lba: u64,
    entry_count: u32,
    entry_size: u32,
    array_crc: u32,
    header_size: u32,
    header_crc: u32,
}

impl GptHeader {
    fn parse(header: &[u8]) -> Option<Self> {
        let header_size = u32_le(header, 12)?;
        // The spec fixes the header between 92 bytes and one sector.
        if !(92..=512).contains(&header_size) {
            return None;
        }
        let entry_count = u32_le(header, 80)?;
        let entry_size = u32_le(header, 84)?;
        // Entry size is 128 per spec; tolerate larger powers of two only.
        if entry_count > MAX_GPT_ENTRIES
            || !entry_size.is_power_of_two()
            || !(128..=MAX_GPT_ENTRY_BYTES).contains(&entry_size)
        {
            return None;
        }
        Some(Self {
            entries_lba: u64_le(header, 72)?,
            entry_count,
            entry_size,
            array_crc: u32_le(header, 88)?,
            header_size,
            header_crc: u32_le(header, 16)?,
        })
    }

    /// CRC32 over the header with its own CRC field zeroed (UEFI §5.3.2).
    fn header_crc_ok(&self, header: &[u8]) -> bool {
        let Ok(size) = usize::try_from(self.header_size) else {
            return false;
        };
        let Some(bytes) = header.get(..size) else {
            return false;
        };
        let mut zeroed = bytes.to_vec();
        zeroed[16..20].fill(0);
        let mut crc = crc32fast::Hasher::new();
        crc.update(&zeroed);
        crc.finalize() == self.header_crc
    }
}
