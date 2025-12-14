//! File system parser trait
//!
//! Defines the interface for parsing file system metadata to find
//! deleted files. This is separate from raw file carving.

use thiserror::Error;

/// Supported file system types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileSystemType {
    /// Linux ext4 filesystem
    Ext4,
    /// Linux ext3 filesystem
    Ext3,
    /// Linux ext2 filesystem
    Ext2,
    /// Linux Btrfs filesystem
    Btrfs,
    /// Linux XFS filesystem
    Xfs,
    /// Windows NTFS filesystem
    Ntfs,
    /// Windows FAT32 filesystem
    Fat32,
    /// Windows exFAT filesystem
    ExFat,
    /// Raw/unknown filesystem (use file carving only)
    Raw,
}

impl FileSystemType {
    /// Returns a human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            FileSystemType::Ext4 => "ext4",
            FileSystemType::Ext3 => "ext3",
            FileSystemType::Ext2 => "ext2",
            FileSystemType::Btrfs => "Btrfs",
            FileSystemType::Xfs => "XFS",
            FileSystemType::Ntfs => "NTFS",
            FileSystemType::Fat32 => "FAT32",
            FileSystemType::ExFat => "exFAT",
            FileSystemType::Raw => "Raw",
        }
    }

    /// Returns whether this filesystem stores deleted file metadata
    pub fn supports_deleted_entries(&self) -> bool {
        matches!(
            self,
            FileSystemType::Ext4
                | FileSystemType::Ext3
                | FileSystemType::Ext2
                | FileSystemType::Ntfs
        )
    }
}

/// Errors that can occur when parsing a file system
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

/// Represents an entry for a deleted file found in filesystem metadata
#[derive(Debug, Clone)]
pub struct DeletedFileEntry {
    /// Inode number (or equivalent identifier)
    pub inode: u64,
    /// Original filename if available
    pub filename: Option<String>,
    /// Original path if recoverable
    pub path: Option<String>,
    /// Size in bytes if known
    pub size: Option<u64>,
    /// Deletion timestamp if available
    pub deleted_at: Option<u64>,
    /// Block addresses where data may be found
    pub data_blocks: Vec<u64>,
    /// Whether the entry is likely recoverable
    pub is_recoverable: bool,
}

/// Trait for parsing file system metadata
///
/// Implementations of this trait can parse specific file systems
/// to extract information about deleted files, including their
/// original locations on disk.
///
/// # Example
///
/// ```ignore
/// let parser = Ext4Parser::new(block_device)?;
/// let deleted_files = parser.find_deleted_entries()?;
/// for entry in deleted_files {
///     println!("Found deleted file: {:?}", entry.filename);
/// }
/// ```
pub trait FileSystemParser: Send + Sync {
    /// Detects the file system type from the superblock
    fn detect_type(&self) -> Result<FileSystemType, FileSystemError>;

    /// Finds entries for deleted files in the filesystem metadata
    fn find_deleted_entries(&self) -> Result<Vec<DeletedFileEntry>, FileSystemError>;

    /// Reads data blocks for a deleted file entry
    fn read_deleted_data(&self, entry: &DeletedFileEntry) -> Result<Vec<u8>, FileSystemError>;

    /// Returns the filesystem type this parser handles
    fn filesystem_type(&self) -> FileSystemType;

    fn is_healthy(&self) -> bool;
}
