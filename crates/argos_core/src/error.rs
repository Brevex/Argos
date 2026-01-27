use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArgosError {
    #[error("I/O error at offset {offset:#x}: {source}")]
    Io {
        offset: u64,
        #[source]
        source: std::io::Error,
    },

    #[error("I/O error reading {path}: {source}")]
    IoPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Bad sector at offset {offset:#x}, skipped {bytes} bytes")]
    BadSector { offset: u64, bytes: u64 },

    #[error("Buffer alignment error: buffer at {ptr:#x} is not aligned to {required} bytes")]
    BufferAlignment { ptr: usize, required: usize },

    #[error("Read beyond end of source: offset {offset:#x} + {size} > {source_size}")]
    ReadBeyondEnd {
        offset: u64,
        size: usize,
        source_size: u64,
    },

    #[error("Invalid {format} structure: {reason}")]
    InvalidStructure { format: FileFormat, reason: String },

    #[error("Missing required {format} marker: {marker}")]
    MissingMarker { format: FileFormat, marker: String },

    #[error("Corrupted {format} data at offset {offset:#x}: {reason}")]
    CorruptedData {
        format: FileFormat,
        offset: u64,
        reason: String,
    },

    #[error("Truncated {format} file: expected at least {expected} bytes, got {actual}")]
    TruncatedFile {
        format: FileFormat,
        expected: usize,
        actual: usize,
    },

    #[error("Insufficient confidence for recovery: {confidence:.1}% (minimum: {threshold:.1}%)")]
    LowConfidence { confidence: f64, threshold: f64 },

    #[error("Fragment reconstruction failed: {reason}")]
    ReconstructionFailed { reason: String },

    #[error("No valid fragments found in search range {start:#x} to {end:#x}")]
    NoFragmentsFound { start: u64, end: u64 },

    #[error("Fragment gap too large: {gap} bytes exceeds maximum {max_gap} bytes")]
    FragmentGapTooLarge { gap: u64, max_gap: u64 },

    #[error("Conflicting fragment assignments: {reason}")]
    FragmentConflict { reason: String },

    #[error("Validation failed: {stage} - {reason}")]
    ValidationFailed { stage: String, reason: String },

    #[error("Entropy out of range for {format}: {entropy:.2} (expected {min:.2} - {max:.2})")]
    EntropyOutOfRange {
        format: FileFormat,
        entropy: f64,
        min: f64,
        max: f64,
    },

    #[error(
        "CRC mismatch in {format} at offset {offset:#x}: expected {expected:#x}, got {actual:#x}"
    )]
    CrcMismatch {
        format: FileFormat,
        offset: u64,
        expected: u32,
        actual: u32,
    },

    #[error("Image decode failed: {reason}")]
    DecodeFailed { reason: String },

    #[error("Invalid configuration: {field} = {value} - {reason}")]
    InvalidConfig {
        field: String,
        value: String,
        reason: String,
    },

    #[error("Permission denied: {path} - {reason}")]
    PermissionDenied { path: PathBuf, reason: String },

    #[error("Device not found: {path}")]
    DeviceNotFound { path: PathBuf },

    #[error("Device busy: {path}")]
    DeviceBusy { path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Jpeg,
    Png,
    Unknown,
}

impl std::fmt::Display for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

pub type ArgosResult<T> = std::result::Result<T, ArgosError>;

pub trait ErrorContext<T> {
    fn with_offset(self, offset: u64) -> ArgosResult<T>;
    fn with_path(self, path: impl Into<PathBuf>) -> ArgosResult<T>;
}

impl<T> ErrorContext<T> for std::result::Result<T, std::io::Error> {
    fn with_offset(self, offset: u64) -> ArgosResult<T> {
        self.map_err(|source| ArgosError::Io { offset, source })
    }

    fn with_path(self, path: impl Into<PathBuf>) -> ArgosResult<T> {
        self.map_err(|source| ArgosError::IoPath {
            path: path.into(),
            source,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Recoverable,
    Retryable,
    FileLevel,
    Fatal,
}

impl ArgosError {
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::BadSector { .. } => ErrorSeverity::Recoverable,
            Self::LowConfidence { .. } => ErrorSeverity::Recoverable,
            Self::NoFragmentsFound { .. } => ErrorSeverity::Recoverable,
            Self::EntropyOutOfRange { .. } => ErrorSeverity::Recoverable,
            Self::InvalidStructure { .. } => ErrorSeverity::FileLevel,
            Self::MissingMarker { .. } => ErrorSeverity::FileLevel,
            Self::CorruptedData { .. } => ErrorSeverity::FileLevel,
            Self::TruncatedFile { .. } => ErrorSeverity::FileLevel,
            Self::ReconstructionFailed { .. } => ErrorSeverity::FileLevel,
            Self::ValidationFailed { .. } => ErrorSeverity::FileLevel,
            Self::CrcMismatch { .. } => ErrorSeverity::FileLevel,
            Self::DecodeFailed { .. } => ErrorSeverity::FileLevel,
            Self::FragmentGapTooLarge { .. } => ErrorSeverity::FileLevel,
            Self::FragmentConflict { .. } => ErrorSeverity::FileLevel,
            Self::BufferAlignment { .. } => ErrorSeverity::Retryable,
            Self::InvalidConfig { .. } => ErrorSeverity::Retryable,
            Self::Io { .. } => ErrorSeverity::Fatal,
            Self::IoPath { .. } => ErrorSeverity::Fatal,
            Self::ReadBeyondEnd { .. } => ErrorSeverity::Fatal,
            Self::PermissionDenied { .. } => ErrorSeverity::Fatal,
            Self::DeviceNotFound { .. } => ErrorSeverity::Fatal,
            Self::DeviceBusy { .. } => ErrorSeverity::Fatal,
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(
            self.severity(),
            ErrorSeverity::Recoverable | ErrorSeverity::FileLevel
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ArgosError::Io {
            offset: 0x1000,
            source: std::io::Error::other("test"),
        };
        assert!(err.to_string().contains("0x1000"));

        let err = ArgosError::LowConfidence {
            confidence: 45.5,
            threshold: 70.0,
        };
        assert!(err.to_string().contains("45.5%"));
    }

    #[test]
    fn test_error_severity() {
        let err = ArgosError::BadSector {
            offset: 0,
            bytes: 512,
        };
        assert_eq!(err.severity(), ErrorSeverity::Recoverable);
        assert!(err.is_recoverable());

        let err = ArgosError::DeviceNotFound {
            path: PathBuf::from("/dev/sda"),
        };
        assert_eq!(err.severity(), ErrorSeverity::Fatal);
        assert!(!err.is_recoverable());
    }

    #[test]
    fn test_error_context() {
        let io_err: std::result::Result<(), std::io::Error> = Err(std::io::Error::other("test"));

        let argos_err = io_err.with_offset(0x2000);
        assert!(matches!(
            argos_err,
            Err(ArgosError::Io { offset: 0x2000, .. })
        ));
    }
}
