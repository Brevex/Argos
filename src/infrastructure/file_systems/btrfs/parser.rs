//! Btrfs filesystem parser implementation
//!
//! Parses Btrfs superblock and tree structures to locate deleted files.
//! Btrfs is a copy-on-write filesystem with B-tree based metadata.

use crate::domain::repositories::{
    BlockDeviceReader, DeletedFileEntry, FileSystemError, FileSystemParser, FileSystemType,
};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::sync::Arc;

/// Btrfs superblock magic number "_BHRfS_M" in little-endian
const BTRFS_MAGIC: [u8; 8] = [0x5f, 0x42, 0x48, 0x52, 0x66, 0x53, 0x5f, 0x4d];

/// Primary superblock offset (64 KiB)
const SUPERBLOCK_OFFSET: u64 = 65536;

/// Superblock size
const SUPERBLOCK_SIZE: usize = 4096;

/// Btrfs superblock structure (partial - essential fields only)
/// Reserved fields for future implementation of tree walking
#[derive(Debug)]
#[allow(dead_code)]
struct BtrfsSuperblock {
    /// Checksum of the superblock
    csum: [u8; 32],
    /// Filesystem UUID
    fsid: [u8; 16],
    /// Physical address of this block
    bytenr: u64,
    /// Flags
    flags: u64,
    /// Magic number "_BHRfS_M"
    magic: [u8; 8],
    /// Generation number
    generation: u64,
    /// Logical address of the root tree root
    root: u64,
    /// Logical address of the chunk tree root
    chunk_root: u64,
    /// Logical address of the log tree root
    log_root: u64,
    /// Total bytes in filesystem
    total_bytes: u64,
    /// Bytes used
    bytes_used: u64,
    /// Root directory objectid
    root_dir_objectid: u64,
    /// Number of devices
    num_devices: u64,
    /// Sector size
    sectorsize: u32,
    /// Node size
    nodesize: u32,
    /// Leaf size (deprecated, same as nodesize)
    leafsize: u32,
    /// Stripe size
    stripesize: u32,
    /// Label
    label: [u8; 256],
}

impl BtrfsSuperblock {
    /// Parses superblock from raw bytes
    fn parse(data: &[u8]) -> Result<Self, FileSystemError> {
        if data.len() < SUPERBLOCK_SIZE {
            return Err(FileSystemError::InvalidSuperblock(
                "Btrfs superblock too small".to_string(),
            ));
        }

        let mut cursor = Cursor::new(data);

        // Read csum (32 bytes)
        let mut csum = [0u8; 32];
        for byte in &mut csum {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // Read fsid (16 bytes)
        let mut fsid = [0u8; 16];
        for byte in &mut fsid {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // bytenr at offset 48
        let bytenr = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // flags at offset 56
        let flags = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // magic at offset 64
        let mut magic = [0u8; 8];
        for byte in &mut magic {
            *byte = cursor
                .read_u8()
                .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        }

        // generation at offset 72
        let generation = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // root at offset 80
        let root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // chunk_root at offset 88
        let chunk_root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // log_root at offset 96
        let log_root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip to total_bytes at offset 104
        // (log_root_transid at 104, but we read total_bytes)
        cursor.set_position(112);
        let total_bytes = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // bytes_used at offset 120
        let bytes_used = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // root_dir_objectid at offset 128
        let root_dir_objectid = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // num_devices at offset 136
        let num_devices = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // sectorsize at offset 144
        let sectorsize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // nodesize at offset 148
        let nodesize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // leafsize at offset 152 (deprecated)
        let leafsize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // stripesize at offset 156
        let stripesize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip to label at offset 299 (approximate)
        cursor.set_position(299);
        let mut label = [0u8; 256];
        for byte in &mut label {
            if cursor.position() >= data.len() as u64 {
                break;
            }
            *byte = cursor.read_u8().unwrap_or(0);
        }

        Ok(Self {
            csum,
            fsid,
            bytenr,
            flags,
            magic,
            generation,
            root,
            chunk_root,
            log_root,
            total_bytes,
            bytes_used,
            root_dir_objectid,
            num_devices,
            sectorsize,
            nodesize,
            leafsize,
            stripesize,
            label,
        })
    }

    /// Validates the superblock by checking magic number
    fn is_valid(&self) -> bool {
        self.magic == BTRFS_MAGIC && self.sectorsize >= 512 && self.nodesize >= 4096
    }

    /// Returns the filesystem label as a string
    fn label_str(&self) -> String {
        String::from_utf8_lossy(&self.label)
            .trim_end_matches('\0')
            .to_string()
    }
}

/// Btrfs filesystem parser
pub struct BtrfsParser<R: BlockDeviceReader> {
    device: Arc<R>,
    superblock: Option<BtrfsSuperblock>,
}

impl<R: BlockDeviceReader> BtrfsParser<R> {
    /// Creates a new Btrfs parser for the given device
    pub fn new(device: Arc<R>) -> Result<Self, FileSystemError> {
        let mut parser = Self {
            device,
            superblock: None,
        };

        // Try to read and validate superblock
        parser.read_superblock()?;

        Ok(parser)
    }

    /// Reads and parses the superblock
    fn read_superblock(&mut self) -> Result<(), FileSystemError> {
        let data = self
            .device
            .read_at(SUPERBLOCK_OFFSET, SUPERBLOCK_SIZE)
            .map_err(|e| FileSystemError::ReadError(e.to_string()))?;

        let superblock = BtrfsSuperblock::parse(&data)?;

        if !superblock.is_valid() {
            return Err(FileSystemError::InvalidSuperblock(
                "Invalid Btrfs magic number".to_string(),
            ));
        }

        self.superblock = Some(superblock);
        Ok(())
    }

    /// Returns a reference to the superblock (crate-internal use only)
    #[allow(dead_code)]
    pub(crate) fn superblock(&self) -> Option<&BtrfsSuperblock> {
        self.superblock.as_ref()
    }
}

impl<R: BlockDeviceReader> FileSystemParser for BtrfsParser<R> {
    fn detect_type(&self) -> Result<FileSystemType, FileSystemError> {
        if self.superblock.is_some() {
            Ok(FileSystemType::Btrfs)
        } else {
            Err(FileSystemError::NoFileSystem)
        }
    }

    fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError> {
        let superblock = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        // Btrfs uses copy-on-write, which means:
        // 1. Old data is preserved until overwritten
        // 2. Deleted files may exist in older tree generations
        // 3. Recovery requires walking the B-tree and finding orphaned inodes

        // This is a simplified implementation
        // Full implementation would walk the subvolume trees

        let deleted_entries = Vec::new();

        log::info!(
            "Btrfs parser: Found filesystem '{}' with {} bytes used of {} total",
            superblock.label_str(),
            superblock.bytes_used,
            superblock.total_bytes
        );

        Ok(deleted_entries)
    }

    fn read_deleted_data(&self, entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError> {
        let superblock = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        let mut data = Vec::new();

        // Btrfs stores extents, which may be compressed or sparse
        // For now, we just read raw blocks
        for &block_addr in &entry.data_blocks {
            let offset = block_addr;
            let block_data = self
                .device
                .read_at(offset, superblock.nodesize as usize)
                .map_err(|e| FileSystemError::ReadError(e.to_string()))?;
            data.extend(block_data);
        }

        if let Some(size) = entry.size {
            data.truncate(size as usize);
        }

        Ok(data)
    }

    fn filesystem_type(&self) -> FileSystemType {
        FileSystemType::Btrfs
    }

    fn is_healthy(&self) -> bool {
        self.superblock
            .as_ref()
            .map(|s| s.is_valid())
            .unwrap_or(false)
    }
}
