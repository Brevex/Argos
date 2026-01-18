pub mod carving;
mod error;
pub mod jpeg;
pub mod png;
pub mod scanners;
pub mod statistics;
mod traits;
mod types;

pub use error::{CoreError, Result};
pub use scanners::{JpegScanner, PngScanner};
pub use traits::{BlockSource, FileScanner};
pub use types::FileType;

pub fn get_image_dimensions(data: &[u8]) -> Option<(usize, usize)> {
    imagesize::blob_size(data)
        .ok()
        .map(|size| (size.width, size.height))
}
