use crate::fs::FileSystemError;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

const EXT4_SUPER_MAGIC: u16 = 0xEF53;
pub const SUPERBLOCK_SIZE: usize = 1024;
pub const SUPERBLOCK_OFFSET: u64 = 1024;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Ext4Superblock {
    pub inode_count: u32,
    pub block_count: u64,
    pub block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub magic: u16,
    pub inode_size: u16,
    pub first_inode: u32,
}

impl Ext4Superblock {
    pub fn parse(data: &[u8]) -> Result<Self, FileSystemError> {
        if data.len() < SUPERBLOCK_SIZE {
            return Err(FileSystemError::InvalidSuperblock(
                "Superblock too small".to_string(),
            ));
        }

        let mut cursor = Cursor::new(data);
        let inode_count = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let block_count_lo = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(24);
        let log_block_size = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let block_size = 1024u32 << log_block_size;

        cursor.set_position(32);
        let blocks_per_group = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(40);
        let inodes_per_group = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(56);
        let magic = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(84);
        let first_inode = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(88);
        let inode_size = cursor
            .read_u16::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        let block_count = block_count_lo as u64;

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

    pub fn is_valid(&self) -> bool {
        self.magic == EXT4_SUPER_MAGIC
            && self.block_size >= 1024
            && self.block_size <= 65536
            && self.inode_size >= 128
    }
}
