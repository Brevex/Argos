use thiserror::Error;

pub mod btrfs;
pub mod ext4;

#[derive(Error, Debug)]
pub enum FileSystemError {
    #[error("Unsupported file system: {0}")]
    Unsupported(String),
    #[error("Invalid superblock: {0}")]
    InvalidSuperblock(String),
    #[error("Corrupted metadata: {0}")]
    CorruptedMetadata(String),
    #[error("Read error: {0}")]
    ReadError(String),
    #[error("No file system detected")]
    NoFileSystem,
    #[error("File system error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileSystemType {
    Ext4,
    Btrfs,
    Raw,
}

impl FileSystemType {
    pub fn name(&self) -> &'static str {
        match self {
            FileSystemType::Ext4 => "ext4",
            FileSystemType::Btrfs => "Btrfs",
            FileSystemType::Raw => "Raw",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeletedFileEntry {
    pub inode: u64,
    pub filename: Option<String>,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub deleted_at: Option<u64>,
    pub data_blocks: Vec<u64>,
    pub is_recoverable: bool,
}
