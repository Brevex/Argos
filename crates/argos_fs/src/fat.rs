//! FAT32 and exFAT deleted-file recovery.
//!
//! On-disk layouts implemented (sources: Microsoft's FAT32 File System
//! Specification 1.03, and the exFAT file system specification): the FAT32
//! BPB and 32-byte directory entries (deleted entries start `0xE5`, with
//! long-name entries preceding them), and the exFAT boot sector plus entry
//! sets (file, stream extension, file name).
//!
//! The FAT chain of a deleted FAT32 file is cleared, so only the start
//! cluster and size survive. Per the spec, extents are reconstructed by
//! assuming contiguity — valid for most camera and SD-card writes — and the
//! result is only tier `FsMetadata` once a validator confirms it; until then
//! it carries the reassembly tier so nothing is overstated.

use std::io::{Read, Seek};
use std::time::{Duration, SystemTime};

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};

use crate::{DeletedFile, FsError, FsKind, Timestamps};
use crate::{read_at, u16_le, u32_le, u64_le, utf16le_name};

/// Boot-sector signature at offset 510, shared by FAT and exFAT.
const BOOT_SIG: u16 = 0xAA55;

/// exFAT boot-sector file-system name at offset 3. Source: exFAT spec §3.1.2.
const EXFAT_NAME: &[u8; 8] = b"EXFAT   ";

/// Marker byte for a deleted FAT directory entry. Source: FAT32 spec §6.
const FAT_DELETED: u8 = 0xE5;

/// FAT attribute bits used here. Source: FAT32 spec §6.
const ATTR_LONG_NAME: u8 = 0x0F;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_VOLUME_ID: u8 = 0x08;

/// exFAT entry types. Source: exFAT spec §7.
const EXFAT_ENTRY_FILE: u8 = 0x85;
const EXFAT_ENTRY_STREAM: u8 = 0xC0;
const EXFAT_ENTRY_NAME: u8 = 0xC1;
/// The in-use bit of an entry type byte; cleared marks a deleted entry.
const EXFAT_IN_USE: u8 = 0x80;

/// Maximum long-name fragments honoured per entry set: FAT32 allows 20
/// (13 chars each, 255-char names); exFAT allows 17.
const MAX_NAME_FRAGMENTS: usize = 20;

/// Maximum characters kept from a recovered name.
const MAX_NAME_CHARS: usize = 255;

/// Largest cluster either format allows, in bytes. Source: exFAT spec §3.1.3
/// (`SectorsPerClusterShift` bounded so a cluster stays at or below 32 MiB);
/// FAT32 never exceeds 64 KiB. Without this bound a one-byte edit to the boot
/// sector turns the root-directory read into a terabyte allocation
/// (A-BOUNDED-ALLOC).
const MAX_CLUSTER_BYTES: u64 = 32 * 1024 * 1024;

/// Bytes per allocation-table entry in both families. Source: FAT spec §4
/// (FAT32) and exFAT spec §7.1.
const FAT32_ENTRY_BYTES: u64 = 4;

/// Smallest allocation-table value that ends a chain rather than continuing
/// it. Source: FAT spec §4 — `0x0FFFFFF8` and above end a FAT32 chain, and
/// `0x0FFFFFF7` marks a bad cluster; exFAT ends at `0xFFFFFFFF`.
const FAT_END_OF_CHAIN: u64 = 0x0FFF_FFF7;

/// Directories one volume walk will read.
///
/// A person's photographs sit in a folder, and folders nest, so the walk has
/// to leave the root — reading only the root is why a whole library could be
/// invisible on a FAT volume. These bound what a crafted or corrupt directory
/// tree can cost (`A-BOUNDED-ALLOC`); a volume with more than this many
/// directories yields the ones nearest its root.
const MAX_DIRECTORIES: usize = 4096;

/// How deep below the root the walk goes.
const MAX_DIRECTORY_DEPTH: usize = 16;

/// Bytes of one directory the walk will read before it stops following the
/// chain. A directory larger than this is not one.
const MAX_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;

/// Geometry of a FAT32 or exFAT volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fat {
    /// Which of the two families this volume is.
    pub kind: FsKind,
    /// Absolute byte offset of the volume start.
    pub volume_offset: ByteOffset,
    /// Bytes per cluster.
    pub cluster_bytes: u64,
    /// Absolute byte offset of the first data cluster (cluster 2).
    pub data_offset: ByteOffset,
    /// Absolute byte offset of the root directory.
    pub root_offset: ByteOffset,
    /// Root directory length in bytes, when fixed by the format.
    pub root_bytes: u64,
    /// First cluster of the root directory.
    pub root_cluster: u64,
    /// Absolute byte offset of the first file allocation table.
    pub fat_offset: ByteOffset,
    /// Length of one allocation table in bytes.
    pub fat_bytes: u64,
    /// Volume length in bytes, from the boot sector's total-sector count.
    pub volume_bytes: u64,
}

impl Fat {
    /// Parses the boot sector at `volume_offset`.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn open<R: Read + Seek>(
        src: &mut R,
        volume_offset: ByteOffset,
    ) -> Result<Option<Self>, FsError> {
        let mut buf = Vec::new();
        if !read_at(src, volume_offset.get(), 512, &mut buf)? {
            return Ok(None);
        }
        Ok(Self::from_boot_sector(&buf, volume_offset))
    }

    /// Interprets `sector` as a FAT32 or exFAT boot sector (also the
    /// residue-sweep anchor validator).
    #[must_use]
    pub fn from_boot_sector(sector: &[u8], volume_offset: ByteOffset) -> Option<Self> {
        if u16_le(sector, 510)? != BOOT_SIG {
            return None;
        }
        if sector.get(3..11)? == EXFAT_NAME {
            return Self::from_exfat(sector, volume_offset);
        }
        Self::from_fat32(sector, volume_offset)
    }

    fn from_fat32(sector: &[u8], volume_offset: ByteOffset) -> Option<Self> {
        let bytes_per_sector = u64::from(u16_le(sector, 11)?);
        if !(512..=4096).contains(&bytes_per_sector) || !bytes_per_sector.is_power_of_two() {
            return None;
        }
        let sectors_per_cluster = u64::from(*sector.get(13)?);
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
            return None;
        }
        let reserved = u64::from(u16_le(sector, 14)?);
        let fats = u64::from(*sector.get(16)?);
        // FAT32 stores zero here and uses the 32-bit field at 36.
        if u16_le(sector, 17)? != 0 || fats == 0 || fats > 4 || reserved == 0 {
            return None;
        }
        let fat_sectors = u64::from(u32_le(sector, 36)?);
        if fat_sectors == 0 {
            return None;
        }
        let cluster_bytes = bytes_per_sector.checked_mul(sectors_per_cluster)?;
        if cluster_bytes > MAX_CLUSTER_BYTES {
            return None;
        }
        let data_start_sector = reserved.checked_add(fats.checked_mul(fat_sectors)?)?;
        let data_offset =
            volume_offset.checked_add(data_start_sector.checked_mul(bytes_per_sector)?)?;
        let root_cluster = u64::from(u32_le(sector, 44)?);
        if root_cluster < 2 {
            return None;
        }
        let root_offset =
            data_offset.checked_add((root_cluster - 2).checked_mul(cluster_bytes)?)?;
        let total_sectors = u64::from(u32_le(sector, 32)?);
        Some(Self {
            kind: FsKind::Fat32,
            volume_offset,
            cluster_bytes,
            data_offset,
            root_offset,
            root_bytes: cluster_bytes,
            root_cluster,
            fat_offset: volume_offset.checked_add(reserved.checked_mul(bytes_per_sector)?)?,
            fat_bytes: fat_sectors.checked_mul(bytes_per_sector)?,
            volume_bytes: total_sectors.checked_mul(bytes_per_sector)?,
        })
    }

    fn from_exfat(sector: &[u8], volume_offset: ByteOffset) -> Option<Self> {
        let bytes_per_sector = 1_u64.checked_shl(u32::from(*sector.get(108)?))?;
        let sectors_per_cluster = 1_u64.checked_shl(u32::from(*sector.get(109)?))?;
        if !(512..=4096).contains(&bytes_per_sector) {
            return None;
        }
        let cluster_bytes = bytes_per_sector.checked_mul(sectors_per_cluster)?;
        if cluster_bytes > MAX_CLUSTER_BYTES {
            return None;
        }
        let cluster_heap = u64::from(u32_le(sector, 88)?);
        let root_cluster = u64::from(u32_le(sector, 96)?);
        if root_cluster < 2 {
            return None;
        }
        let data_offset = volume_offset.checked_add(cluster_heap.checked_mul(bytes_per_sector)?)?;
        let root_offset =
            data_offset.checked_add((root_cluster - 2).checked_mul(cluster_bytes)?)?;
        let total_sectors = u64_le(sector, 72)?;
        let fat_sector = u64::from(u32_le(sector, 80)?);
        let fat_sectors = u64::from(u32_le(sector, 84)?);
        Some(Self {
            kind: FsKind::ExFat,
            volume_offset,
            cluster_bytes,
            data_offset,
            root_offset,
            root_bytes: cluster_bytes,
            root_cluster,
            fat_offset: volume_offset.checked_add(fat_sector.checked_mul(bytes_per_sector)?)?,
            fat_bytes: fat_sectors.checked_mul(bytes_per_sector)?,
            volume_bytes: total_sectors.checked_mul(bytes_per_sector)?,
        })
    }

    /// Absolute byte offset of data cluster `cluster` (clusters start at 2).
    fn cluster_at(&self, cluster: u64) -> Option<ByteOffset> {
        if cluster < 2 {
            return None;
        }
        self.data_offset
            .checked_add((cluster - 2).checked_mul(self.cluster_bytes)?)
    }

    /// Recovers deleted entries from the root directory region.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; malformed entries are skipped.
    pub fn recover_deleted<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let mut out = Vec::new();
        let mut seen: Vec<u64> = Vec::new();
        // Breadth first from the root, so a volume whose depth ceiling is
        // reached still yields the directories nearest it — where a person's
        // own folders are, rather than the deepest branch of one of them.
        let mut queue: Vec<(u64, usize)> = vec![(self.root_cluster, 0)];
        let mut at = 0_usize;
        let mut directories = 0_usize;

        while at < queue.len() && directories < MAX_DIRECTORIES {
            let (cluster, depth) = queue[at];
            at += 1;
            if seen.contains(&cluster) {
                continue;
            }
            seen.push(cluster);
            directories += 1;

            let bytes = self.read_chain(src, cluster)?;
            if bytes.is_empty() {
                continue;
            }
            out.extend(self.deleted_in_directory(&bytes));
            if depth < MAX_DIRECTORY_DEPTH {
                queue.extend(
                    self.subdirectories(&bytes)
                        .into_iter()
                        .map(|child| (child, depth + 1)),
                );
            }
        }
        Ok(out)
    }

    /// Reads a directory by following its cluster chain.
    ///
    /// The chain is intact for a directory that still exists, which is what
    /// makes this different from recovering a deleted *file*: the entries
    /// naming deleted files live in directories the filesystem still tracks.
    /// A chain that runs past [`MAX_DIRECTORY_BYTES`], repeats a cluster or
    /// points outside the volume stops there rather than being followed
    /// (`A-UNTRUSTED-ONDISK`).
    fn read_chain<R: Read + Seek>(&self, src: &mut R, first: u64) -> Result<Vec<u8>, FsError> {
        let mut out = Vec::new();
        let mut piece = Vec::new();
        let mut cluster = first;
        let mut visited = 0_u64;
        let step = usize::try_from(self.cluster_bytes).unwrap_or(0);
        let ceiling = MAX_DIRECTORY_BYTES / self.cluster_bytes.max(1);

        while visited < ceiling && cluster >= 2 {
            let Some(offset) = self.cluster_at(cluster) else {
                break;
            };
            if offset.get() >= self.volume_offset.get().saturating_add(self.volume_bytes) {
                break;
            }
            if step == 0 || !read_at(src, offset.get(), step, &mut piece)? {
                break;
            }
            out.extend_from_slice(&piece);
            visited += 1;
            match self.next_cluster(src, cluster)? {
                // A chain that loops back on itself is corrupt, and following
                // it would read the same directory for as long as the ceiling
                // allowed.
                Some(next) if next != cluster => cluster = next,
                _ => break,
            }
        }
        Ok(out)
    }

    /// The cluster following `cluster` in the allocation table, if any.
    fn next_cluster<R: Read + Seek>(
        &self,
        src: &mut R,
        cluster: u64,
    ) -> Result<Option<u64>, FsError> {
        let Some(at) = cluster
            .checked_mul(FAT32_ENTRY_BYTES)
            .filter(|at| at.saturating_add(FAT32_ENTRY_BYTES) <= self.fat_bytes)
            .and_then(|at| self.fat_offset.checked_add(at))
        else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        if !read_at(src, at.get(), 4, &mut buf)? {
            return Ok(None);
        }
        // Both families use 32-bit entries here; FAT32 reserves the top four
        // bits. Source: exFAT spec §7.1, FAT spec §4.
        let raw = u64::from(u32_le(&buf, 0).unwrap_or(0));
        let next = if self.kind == FsKind::ExFat {
            raw
        } else {
            raw & 0x0FFF_FFFF
        };
        // End-of-chain and bad-cluster marks are both "no next".
        Ok((2..FAT_END_OF_CHAIN).contains(&next).then_some(next))
    }

    /// First clusters of the live subdirectories `dir` names.
    ///
    /// Only directories that still exist: a deleted one has lost its chain,
    /// so following it would be reading whatever now occupies those clusters
    /// and reporting the names found there as if they were its.
    fn subdirectories(&self, dir: &[u8]) -> Vec<u64> {
        match self.kind {
            FsKind::ExFat => Self::exfat_subdirectories(dir),
            _ => Self::fat32_subdirectories(dir),
        }
    }

    fn fat32_subdirectories(dir: &[u8]) -> Vec<u64> {
        let mut out = Vec::new();
        for entry in dir.chunks_exact(32) {
            let first = entry[0];
            if first == 0 {
                break;
            }
            let attrs = entry[11];
            // `.` and `..` point back up; following them would loop.
            if first == FAT_DELETED
                || first == b'.'
                || attrs & ATTR_LONG_NAME == ATTR_LONG_NAME
                || attrs & ATTR_DIRECTORY == 0
                || attrs & ATTR_VOLUME_ID != 0
            {
                continue;
            }
            let cluster = u64::from(u16_le(entry, 20).unwrap_or(0)) << 16
                | u64::from(u16_le(entry, 26).unwrap_or(0));
            if cluster >= 2 {
                out.push(cluster);
            }
        }
        out
    }

    fn exfat_subdirectories(dir: &[u8]) -> Vec<u64> {
        let mut out = Vec::new();
        let mut index = 0_usize;
        while (index + 1) * 32 <= dir.len() {
            let entry = &dir[index * 32..(index + 1) * 32];
            let kind = entry[0];
            index += 1;
            if kind != EXFAT_ENTRY_FILE {
                continue;
            }
            // Bit 4 of the file attributes is the directory flag.
            // Source: exFAT spec §7.4.4.
            let directory = u16_le(entry, 4).unwrap_or(0) & 0x0010 != 0;
            let secondary = usize::from(entry[1]).min(MAX_NAME_FRAGMENTS + 1);
            for _ in 0..secondary {
                if (index + 1) * 32 > dir.len() {
                    break;
                }
                let sub = &dir[index * 32..(index + 1) * 32];
                index += 1;
                if directory && sub[0] == EXFAT_ENTRY_STREAM {
                    let cluster = u64::from(u32_le(sub, 20).unwrap_or(0));
                    if cluster >= 2 {
                        out.push(cluster);
                    }
                }
            }
        }
        out
    }

    /// Parses deleted entries out of one directory region's bytes.
    #[must_use]
    pub fn deleted_in_directory(&self, dir: &[u8]) -> Vec<DeletedFile> {
        match self.kind {
            FsKind::ExFat => self.exfat_deleted(dir),
            _ => self.fat32_deleted(dir),
        }
    }

    fn fat32_deleted(&self, dir: &[u8]) -> Vec<DeletedFile> {
        let mut out = Vec::new();
        let mut fragments: Vec<(u8, String)> = Vec::new();
        for entry in dir.chunks_exact(32) {
            let first = entry[0];
            if first == 0 {
                break; // End of directory.
            }
            let attrs = entry[11];
            if attrs & ATTR_LONG_NAME == ATTR_LONG_NAME {
                if fragments.len() < MAX_NAME_FRAGMENTS
                    && let Some(part) = long_name_fragment(entry)
                {
                    fragments.push((entry[0] & 0x1F, part));
                }
                continue;
            }
            if first != FAT_DELETED || attrs & (ATTR_DIRECTORY | ATTR_VOLUME_ID) != 0 {
                fragments.clear();
                continue;
            }
            let name = assemble_long_name(&mut fragments).or_else(|| short_name(entry, true));
            let size = u64::from(u32_le(entry, 28).unwrap_or(0));
            let cluster = u64::from(u16_le(entry, 20).unwrap_or(0)) << 16
                | u64::from(u16_le(entry, 26).unwrap_or(0));
            let extents = self.contiguous_extents(cluster, size);
            out.push(DeletedFile {
                name,
                timestamps: Timestamps {
                    created: dos_time(u16_le(entry, 16), u16_le(entry, 14)),
                    modified: dos_time(u16_le(entry, 24), u16_le(entry, 22)),
                },
                size,
                extents,
                fs: FsKind::Fat32,
                // The FAT chain is gone: contiguity is an assumption, so the
                // tier stays at the reassembly level until a validator says
                // otherwise (A-CONFIDENCE-HONEST).
                confidence: Confidence::Reassembled,
                source_object: Some(cluster),
            });
        }
        out
    }

    fn exfat_deleted(&self, dir: &[u8]) -> Vec<DeletedFile> {
        let mut out = Vec::new();
        let mut index = 0_usize;
        while (index + 1) * 32 <= dir.len() {
            let entry = &dir[index * 32..(index + 1) * 32];
            let kind = entry[0];
            index += 1;
            // A deleted file entry has the in-use bit cleared.
            if kind != EXFAT_ENTRY_FILE & !EXFAT_IN_USE {
                continue;
            }
            let secondary = usize::from(entry[1]).min(MAX_NAME_FRAGMENTS + 1);
            let mut name_units: Vec<u8> = Vec::new();
            let mut size = 0_u64;
            let mut cluster = 0_u64;
            let mut no_fat_chain = false;
            for _ in 0..secondary {
                if (index + 1) * 32 > dir.len() {
                    break;
                }
                let sub = &dir[index * 32..(index + 1) * 32];
                index += 1;
                match sub[0] | EXFAT_IN_USE {
                    EXFAT_ENTRY_STREAM => {
                        // Bit 1 of the general secondary flags: NoFatChain.
                        no_fat_chain = sub[1] & 0x02 != 0;
                        size = u64_le(sub, 24).unwrap_or(0);
                        cluster = u64::from(u32_le(sub, 20).unwrap_or(0));
                    }
                    EXFAT_ENTRY_NAME => name_units.extend_from_slice(&sub[2..32]),
                    _ => {}
                }
            }
            let extents = self.contiguous_extents(cluster, size);
            out.push(DeletedFile {
                name: utf16le_name(&name_units, MAX_NAME_CHARS),
                timestamps: Timestamps {
                    created: dos_time(u16_le(entry, 10), u16_le(entry, 8)),
                    modified: dos_time(u16_le(entry, 14), u16_le(entry, 12)),
                },
                size,
                extents,
                fs: FsKind::ExFat,
                // A NoFatChain stream stores exact extents; otherwise the
                // chain is lost and contiguity is only an assumption.
                confidence: if no_fat_chain {
                    Confidence::FsMetadata
                } else {
                    Confidence::Reassembled
                },
                source_object: Some(cluster),
            });
        }
        out
    }

    /// Extents under the contiguity assumption: `size` bytes from `cluster`,
    /// capped at the volume so a corrupt size field cannot claim a range the
    /// medium does not contain.
    fn contiguous_extents(&self, cluster: u64, size: u64) -> Vec<ByteRange> {
        if size == 0 {
            return Vec::new();
        }
        let (Some(start), Some(volume_end)) = (
            self.cluster_at(cluster),
            self.volume_offset.checked_add(self.volume_bytes),
        ) else {
            return Vec::new();
        };
        let Some(available) = volume_end.get().checked_sub(start.get()) else {
            return Vec::new();
        };
        let bytes = size.min(available);
        if bytes == 0 {
            return Vec::new();
        }
        vec![ByteRange::new(start, bytes)]
    }
}

/// One long-file-name fragment's 13 UTF-16 units, in entry order.
fn long_name_fragment(entry: &[u8]) -> Option<String> {
    let mut raw = Vec::with_capacity(26);
    raw.extend_from_slice(entry.get(1..11)?);
    raw.extend_from_slice(entry.get(14..26)?);
    raw.extend_from_slice(entry.get(28..32)?);
    // Fragments are not NUL-terminated mid-name; stop at padding.
    let units: Vec<u16> = raw
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0 && unit != 0xFFFF)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}

/// Joins collected fragments (stored highest-sequence-first on disk).
fn assemble_long_name(fragments: &mut Vec<(u8, String)>) -> Option<String> {
    if fragments.is_empty() {
        return None;
    }
    fragments.sort_by_key(|&(sequence, _)| sequence);
    let name: String = fragments.iter().map(|(_, part)| part.as_str()).collect();
    fragments.clear();
    if name.is_empty() { None } else { Some(name) }
}

/// The 8.3 short name; the first character of a deleted entry is lost to the
/// `0xE5` marker and is reported as `_`.
fn short_name(entry: &[u8], deleted: bool) -> Option<String> {
    let raw = entry.get(..11)?;
    let base = std::str::from_utf8(raw.get(..8)?).ok()?.trim_end();
    let ext = std::str::from_utf8(raw.get(8..11)?).ok()?.trim_end();
    if base.is_empty() {
        return None;
    }
    let mut name = String::with_capacity(12);
    if deleted {
        name.push('_');
        name.push_str(base.get(1..).unwrap_or(""));
    } else {
        name.push_str(base);
    }
    if !ext.is_empty() {
        name.push('.');
        name.push_str(ext);
    }
    Some(name)
}

/// DOS date/time pair to `SystemTime`. Source: FAT32 spec §7.
fn dos_time(date: Option<u16>, time: Option<u16>) -> Option<SystemTime> {
    let (date, time) = (date?, time?);
    if date == 0 {
        return None;
    }
    let year = 1980 + u64::from(date >> 9);
    let month = u64::from((date >> 5) & 0x0F);
    let day = u64::from(date & 0x1F);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since the Unix epoch via the civil-from-days algorithm, shifted so
    // March starts the year (leap day lands last).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe;
    // 719468 days between 0000-03-01 and 1970-01-01.
    let unix_days = days.checked_sub(719_468)?;
    let secs = unix_days.checked_mul(86_400)?
        + u64::from(time >> 11) * 3600
        + u64::from((time >> 5) & 0x3F) * 60
        + u64::from(time & 0x1F) * 2;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}
