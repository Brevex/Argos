use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArgosError {
    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("allocation failed: size={size}, align={align}")]
    Allocation { size: usize, align: usize },

    #[error("unsupported platform")]
    Unsupported,

    #[error("pattern build error")]
    PatternBuild(#[from] aho_corasick::BuildError),

    #[error("validation failed: {kind}")]
    Validation { kind: ValidationKind },

    #[error("audit serialization error")]
    AuditSerialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationKind {
    MissingSoi,
    MissingEoi,
    InvalidMarker,
    TruncatedSegment,
    BadHuffmanTable,
    BadHuffmanCode,
    BadEntropyStream,
    BadDctCoefficient,
    MissingIhdr,
    MissingIend,
    BadCrc32,
    TruncatedChunk,
}

impl std::fmt::Display for ValidationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<rustix::io::Errno> for ArgosError {
    fn from(e: rustix::io::Errno) -> Self {
        ArgosError::Io(e.into())
    }
}
