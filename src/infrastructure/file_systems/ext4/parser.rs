//! ext4 filesystem parser implementation
//!
//! Parses ext4 superblock and inode tables to locate deleted files.

use crate::domain::repositories::{
    BlockDeviceReader, DeletedFileEntry, FileSystemError, FileSystemParser, FileSystemType,
};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::sync::Arc;

/// ext4 superblock magic number
const EXT4_SUPER_MAGIC: u16 = 0xEF53;

/// Superblock offset from partition start
const SUPERBLOCK_OFFSET: u64 = 1024;

/// Superblock size
const SUPERBLOCK_SIZE: usize = 1024;

/// ext4 superblock structure (partial - only fields we need)
/// Some fields are reserved for future inode table parsing implementation
#[derive(Debug)]
#[allow(dead_code)]
struct Ext4Superblock {
    /// Total inode count
    inode_count: u32,
    /// Total block count
    block_count: u64,
    /// Block size (1024 << log_block_size)
    block_size: u32,
    /// Blocks per group
    blocks_per_group: u32,
    /// Inodes per group
    inodes_per_group: u32,
    /// Magic signature
    magic: u16,
    /// Inode size
    inode_size: u16,
    /// First inode number
    first_inode: u32,
}

impl Ext4Superblock {
    /// Parses superblock from raw bytes
    fn parse(data: &[u8]) -> Result<Self, FileSystemError> {
        if data.len() < SUPERBLOCK_SIZE {
            return Err(FileSystemError::InvalidSuperblock(
                "Superblock too small".to_string(),
            ));
        }

        let mut cursor = Cursor::new(data);

        // Read fields at their offsets
        let inode_count = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        let block_count_lo = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip to offset 24 (log_block_size is at offset 24)
        cursor.set_position(24);
        let log_block_size = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let block_size = 1024u32 << log_block_size;

        // Skip to blocks_per_group at offset 32
        cursor.set_position(32);
        let blocks_per_group = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Skip to inodes_per_group at offset 40
        cursor.set_position(40);
        let inodes_per_group = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Magic is at offset 56
        cursor.set_position(56);
        let magic = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Inode size at offset 88
        cursor.set_position(88);
        let inode_size = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // First inode at offset 84
        cursor.set_position(84);
        let first_inode = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        // Block count high bits at offset 336 (for 64-bit block count)
        let block_count = block_count_lo as u64;
        // Note: Full implementation would read high bits for >16TB filesystems

        Ok(Self {
            inode_count,
            block_count,
            block_size,
            blocks_per_group,
            inodes_per_group,
            magic,
            inode_size,
            first_inode,
        })
    }

    /// Validates the superblock
    fn is_valid(&self) -> bool {
        self.magic == EXT4_SUPER_MAGIC
            && self.block_size >= 1024
            && self.block_size <= 65536
            && self.inode_size >= 128
    }
}

/// ext4 filesystem parser
pub struct Ext4Parser<R: BlockDeviceReader> {
    device: Arc<R>,
    superblock: Option<Ext4Superblock>,
}

impl<R: BlockDeviceReader> Ext4Parser<R> {
    /// Creates a new ext4 parser for the given device
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

        let superblock = Ext4Superblock::parse(&data)?;

        if !superblock.is_valid() {
            return Err(FileSystemError::InvalidSuperblock(
                "Invalid ext4 magic number".to_string(),
            ));
        }

        self.superblock = Some(superblock);
        Ok(())
    }

    /// Returns a reference to the superblock (crate-internal use only)
    #[allow(dead_code)]
    pub(crate) fn superblock(&self) -> Option<&Ext4Superblock> {
        self.superblock.as_ref()
    }
}

impl<R: BlockDeviceReader> FileSystemParser for Ext4Parser<R> {
    fn detect_type(&self) -> Result<FileSystemType, FileSystemError> {
        if self.superblock.is_some() {
            Ok(FileSystemType::Ext4)
        } else {
            Err(FileSystemError::NoFileSystem)
        }
    }

    fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError> {
        let superblock = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        // This is a simplified implementation
        // Full implementation would:
        // 1. Read group descriptors
        // 2. For each group, read the inode table
        // 3. Look for inodes with dtime > 0 (deleted) and data blocks still set

        let deleted_entries = Vec::new();

        // For now, we return an empty list and rely on file carving
        // A complete implementation would iterate through all block groups
        // and analyze inode tables for deleted entries

        log::info!(
            "ext4 parser: Found filesystem with {} inodes, {} blocks",
            superblock.inode_count,
            superblock.block_count
        );

        Ok(deleted_entries)
    }

    fn read_deleted_data(&self, entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError> {
        let superblock = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        let mut data = Vec::new();

        for &block_addr in &entry.data_blocks {
            let offset = block_addr * superblock.block_size as u64;
            let block_data = self
                .device
                .read_at(offset, superblock.block_size as usize)
                .map_err(|e| FileSystemError::ReadError(e.to_string()))?;
            data.extend(block_data);
        }

        // Truncate to actual file size if known
        if let Some(size) = entry.size {
            data.truncate(size as usize);
        }

        Ok(data)
    }

    fn filesystem_type(&self) -> FileSystemType {
        FileSystemType::Ext4
    }

    fn is_healthy(&self) -> bool {
        self.superblock
            .as_ref()
            .map(|s| s.is_valid())
            .unwrap_or(false)
    }
}
