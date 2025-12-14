//! NTFS filesystem parser implementation
//!
//! Parses NTFS boot sector and MFT (Master File Table) to locate deleted files.
//! NTFS stores file metadata in MFT entries, which may still contain
//! information about deleted files.

use crate::domain::repositories::{
    BlockDeviceReader, DeletedFileEntry, FileSystemError, FileSystemParser, FileSystemType,
};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::sync::Arc;

/// NTFS boot sector signature "NTFS    "
const NTFS_OEM_ID: [u8; 8] = [0x4E, 0x54, 0x46, 0x53, 0x20, 0x20, 0x20, 0x20];

/// Boot sector offset
const BOOT_SECTOR_OFFSET: u64 = 0;

/// Boot sector size
const BOOT_SECTOR_SIZE: usize = 512;

/// MFT entry signature "FILE"
const MFT_ENTRY_SIGNATURE: [u8; 4] = [0x46, 0x49, 0x4C, 0x45];

/// NTFS boot sector structure (BIOS Parameter Block)
/// Contains essential filesystem parameters
#[derive(Debug)]
#[allow(dead_code)]
struct NtfsBootSector {
    /// Jump instruction (3 bytes)
    jump: [u8; 3],
    /// OEM ID "NTFS    "
    oem_id: [u8; 8],
    /// Bytes per sector
    bytes_per_sector: u16,
    /// Sectors per cluster
    sectors_per_cluster: u8,
    /// Reserved sectors (always 0 for NTFS)
    reserved_sectors: u16,
    /// Always 0 for NTFS
    always_zero1: [u8; 3],
    /// Not used by NTFS
    not_used1: u16,
    /// Media descriptor
    media_descriptor: u8,
    /// Always 0 for NTFS
    always_zero2: u16,
    /// Sectors per track
    sectors_per_track: u16,
    /// Number of heads
    number_of_heads: u16,
    /// Hidden sectors
    hidden_sectors: u32,
    /// Not used by NTFS
    not_used2: u32,
    /// Not used by NTFS (0x80008000)
    not_used3: u32,
    /// Total sectors in volume
    total_sectors: u64,
    /// LCN of MFT
    mft_lcn: u64,
    /// LCN of MFT mirror
    mft_mirror_lcn: u64,
    /// Clusters per MFT record (can be negative for bytes)
    clusters_per_mft_record: i8,
    /// Clusters per index record
    clusters_per_index_record: i8,
    /// Volume serial number
    volume_serial: u64,
}

impl NtfsBootSector {
    /// Parses boot sector from raw bytes
    fn parse(data: &[u8]) -> Result<Self, FileSystemError> {
        if data.len() < BOOT_SECTOR_SIZE {
            return Err(FileSystemError::InvalidSuperblock(
                "NTFS boot sector too small".to_string(),
            ));
        }

        let mut cursor = Cursor::new(data);

        // Jump instruction (3 bytes)
        let mut jump = [0u8; 3];
        for byte in &mut jump {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // OEM ID (8 bytes)
        let mut oem_id = [0u8; 8];
        for byte in &mut oem_id {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // Bytes per sector at offset 11
        let bytes_per_sector = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Sectors per cluster at offset 13
        let sectors_per_cluster = cursor
            .read_u8()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Reserved sectors at offset 14
        let reserved_sectors = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Always zero (3 bytes) at offset 16
        let mut always_zero1 = [0u8; 3];
        for byte in &mut always_zero1 {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // Not used at offset 19
        let not_used1 = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Media descriptor at offset 21
        let media_descriptor = cursor
            .read_u8()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Always zero at offset 22
        let always_zero2 = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Sectors per track at offset 24
        let sectors_per_track = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Number of heads at offset 26
        let number_of_heads = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Hidden sectors at offset 28
        let hidden_sectors = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Not used at offset 32
        let not_used2 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Not used at offset 36
        let not_used3 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Total sectors at offset 40
        let total_sectors = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // MFT LCN at offset 48
        let mft_lcn = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // MFT mirror LCN at offset 56
        let mft_mirror_lcn = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Clusters per MFT record at offset 64
        let clusters_per_mft_record = cursor
            .read_i8()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip 3 unused bytes
        cursor.set_position(68);

        // Clusters per index record at offset 68
        let clusters_per_index_record = cursor
            .read_i8()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip to volume serial at offset 72
        cursor.set_position(72);
        let volume_serial = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        Ok(Self {
            jump,
            oem_id,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            always_zero1,
            not_used1,
            media_descriptor,
            always_zero2,
            sectors_per_track,
            number_of_heads,
            hidden_sectors,
            not_used2,
            not_used3,
            total_sectors,
            mft_lcn,
            mft_mirror_lcn,
            clusters_per_mft_record,
            clusters_per_index_record,
            volume_serial,
        })
    }

    /// Validates the boot sector
    fn is_valid(&self) -> bool {
        self.oem_id == NTFS_OEM_ID
            && self.bytes_per_sector >= 512
            && self.sectors_per_cluster > 0
            && self.total_sectors > 0
    }

    /// Returns the cluster size in bytes
    fn cluster_size(&self) -> u64 {
        self.bytes_per_sector as u64 * self.sectors_per_cluster as u64
    }

    /// Returns the MFT record size in bytes
    fn mft_record_size(&self) -> u64 {
        if self.clusters_per_mft_record > 0 {
            self.cluster_size() * self.clusters_per_mft_record as u64
        } else {
            // Negative value means 2^|value| bytes
            1u64 << (-self.clusters_per_mft_record as u64)
        }
    }

    /// Returns the MFT offset in bytes
    fn mft_offset(&self) -> u64 {
        self.mft_lcn * self.cluster_size()
    }

    /// Returns total volume size in bytes
    fn volume_size(&self) -> u64 {
        self.total_sectors * self.bytes_per_sector as u64
    }
}

/// NTFS filesystem parser
pub struct NtfsParser<R: BlockDeviceReader> {
    device: Arc<R>,
    boot_sector: Option<NtfsBootSector>,
}

impl<R: BlockDeviceReader> NtfsParser<R> {
    /// Creates a new NTFS parser for the given device
    pub fn new(device: Arc<R>) -> Result<Self, FileSystemError> {
        let mut parser = Self {
            device,
            boot_sector: None,
        };

        // Try to read and validate boot sector
        parser.read_boot_sector()?;

        Ok(parser)
    }

    /// Reads and parses the boot sector
    fn read_boot_sector(&mut self) -> Result<(), FileSystemError> {
        let data = self
            .device
            .read_at(BOOT_SECTOR_OFFSET, BOOT_SECTOR_SIZE)
            .map_err(|e| FileSystemError::ReadError(e.to_string()))?;

        let boot_sector = NtfsBootSector::parse(&data)?;

        if !boot_sector.is_valid() {
            return Err(FileSystemError::InvalidSuperblock(
                "Invalid NTFS OEM ID".to_string(),
            ));
        }

        self.boot_sector = Some(boot_sector);
        Ok(())
    }

    /// Returns a reference to the boot sector (crate-internal use only)
    #[allow(dead_code)]
    pub(crate) fn boot_sector(&self) -> Option<&NtfsBootSector> {
        self.boot_sector.as_ref()
    }

    /// Checks if a buffer contains a valid MFT entry signature
    #[allow(dead_code)]
    fn is_mft_entry(data: &[u8]) -> bool {
        data.len() >= 4 && data[0..4] == MFT_ENTRY_SIGNATURE
    }
}

impl<R: BlockDeviceReader> FileSystemParser for NtfsParser<R> {
    fn detect_type(&self) -> Result<FileSystemType, FileSystemError> {
        if self.boot_sector.is_some() {
            Ok(FileSystemType::Ntfs)
        } else {
            Err(FileSystemError::NoFileSystem)
        }
    }

    fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError> {
        let boot_sector = self.boot_sector.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Boot sector not loaded".to_string())
        })?;

        // NTFS recovery approach:
        // 1. Read MFT from mft_lcn
        // 2. Parse MFT entries (FILE records)
        // 3. Look for entries with FLAG_IN_USE = 0 (deleted)
        // 4. Extract filename from $FILE_NAME attribute
        // 5. Get data runs from $DATA attribute

        // This is a simplified implementation
        // Full implementation would parse MFT entries

        let deleted_entries = Vec::new();

        log::info!(
            "NTFS parser: Found filesystem with {} bytes, MFT at offset {}, record size {}",
            boot_sector.volume_size(),
            boot_sector.mft_offset(),
            boot_sector.mft_record_size()
        );

        Ok(deleted_entries)
    }

    fn read_deleted_data(&self, entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError> {
        let boot_sector = self.boot_sector.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Boot sector not loaded".to_string())
        })?;

        let mut data = Vec::new();
        let cluster_size = boot_sector.cluster_size() as usize;

        // NTFS stores data in cluster runs
        // Each block address is a cluster number
        for &cluster in &entry.data_blocks {
            let offset = cluster * boot_sector.cluster_size();
            let cluster_data = self
                .device
                .read_at(offset, cluster_size)
                .map_err(|e| FileSystemError::ReadError(e.to_string()))?;
            data.extend(cluster_data);
        }

        if let Some(size) = entry.size {
            data.truncate(size as usize);
        }

        Ok(data)
    }

    fn filesystem_type(&self) -> FileSystemType {
        FileSystemType::Ntfs
    }

    fn is_healthy(&self) -> bool {
        self.boot_sector
            .as_ref()
            .map(|s| s.is_valid())
            .unwrap_or(false)
    }
}
