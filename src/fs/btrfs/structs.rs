use crate::fs::FileSystemError;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};

const BTRFS_MAGIC: [u8; 8] = [0x5f, 0x42, 0x48, 0x52, 0x66, 0x53, 0x5f, 0x4d];
pub const SUPERBLOCK_OFFSET: u64 = 65536;
pub const SUPERBLOCK_SIZE: usize = 4096;

#[derive(Debug, Clone)]
pub struct BtrfsSuperblock {
    pub csum: [u8; 32],
    pub fsid: [u8; 16],
    pub bytenr: u64,
    pub flags: u64,
    pub magic: [u8; 8],
    pub generation: u64,
    pub root: u64,
    pub chunk_root: u64,
    pub log_root: u64,
    pub total_bytes: u64,
    pub bytes_used: u64,
    pub root_dir_objectid: u64,
    pub num_devices: u64,
    pub sectorsize: u32,
    pub nodesize: u32,
    pub leafsize: u32,
    pub stripesize: u32,
    pub label: [u8; 256],
}

impl BtrfsSuperblock {
    pub fn parse(data: &[u8]) -> Result<Self, FileSystemError> {
        if data.len() < SUPERBLOCK_SIZE {
            return Err(FileSystemError::InvalidSuperblock(
                "Btrfs superblock too small".to_string(),
            ));
        }

        let mut cursor = Cursor::new(data);
        let mut csum = [0u8; 32];
        cursor.read_exact(&mut csum).unwrap();

        let mut fsid = [0u8; 16];
        cursor.read_exact(&mut fsid).unwrap();

        let bytenr = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let flags = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        let mut magic = [0u8; 8];
        cursor.read_exact(&mut magic).unwrap();

        let generation = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let chunk_root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let log_root = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(112);
        let total_bytes = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(120);
        let bytes_used = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let root_dir_objectid = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let num_devices = cursor
            .read_u64::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let sectorsize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let nodesize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let leafsize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;
        let stripesize = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| FileSystemError::InvalidSuperblock(e.to_string()))?;

        cursor.set_position(299);
        let mut label = [0u8; 256];
        let bytes_available = data.len() as u64 - cursor.position();
        let bytes_to_read = bytes_available.min(256) as usize;
        cursor
            .read_exact(&mut label[0..bytes_to_read])
            .unwrap_or(());

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

    pub fn is_valid(&self) -> bool {
        self.magic == BTRFS_MAGIC && self.sectorsize >= 512 && self.nodesize >= 4096
    }

    pub fn label_str(&self) -> String {
        String::from_utf8_lossy(&self.label)
            .trim_end_matches('\0')
            .to_string()
    }
}
