use super::superblock::{Ext4Superblock, SUPERBLOCK_OFFSET, SUPERBLOCK_SIZE};
use crate::core::io::BlockDeviceReader;
use crate::fs::{DeletedFileEntry, FileSystemError, FileSystemType};
use std::sync::Arc;

pub struct Ext4Parser<R: BlockDeviceReader> {
    device: Arc<R>,
    superblock: Option<Ext4Superblock>,
}

impl<R: BlockDeviceReader> Ext4Parser<R> {
    pub fn new(device: Arc<R>) -> Result<Self, FileSystemError> {
        let mut parser = Self {
            device,
            superblock: None,
        };
        parser.read_superblock()?;
        Ok(parser)
    }

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

    pub fn detect_type(&self) -> Result<FileSystemType, FileSystemError> {
        if self.superblock.is_some() {
            Ok(FileSystemType::Ext4)
        } else {
            Err(FileSystemError::NoFileSystem)
        }
    }

    pub fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError> {
        let _sb = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        Ok(Vec::new())
    }

    pub fn read_deleted_data(&self, entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError> {
        let sb = self.superblock.as_ref().ok_or_else(|| {
            FileSystemError::InvalidSuperblock("Superblock not loaded".to_string())
        })?;

        let mut data = Vec::new();
        for &block_addr in &entry.data_blocks {
            let offset = block_addr * sb.block_size as u64;
            let block_data = self
                .device
                .read_at(offset, sb.block_size as usize)
                .map_err(|e| FileSystemError::ReadError(e.to_string()))?;
            data.extend(block_data);
        }

        if let Some(size) = entry.size {
            data.truncate(size as usize);
        }
        Ok(data)
    }
}
