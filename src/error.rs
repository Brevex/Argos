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
}

impl From<rustix::io::Errno> for ArgosError {
    fn from(e: rustix::io::Errno) -> Self {
        ArgosError::Io(e.into())
    }
}
