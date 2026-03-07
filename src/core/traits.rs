use super::types::{BreakPoint, ImageFormat};

/// Extensibility contract for image format support.
///
/// Each format (JPEG, PNG, future WebP, etc.) implements this trait to define
/// its parsing, validation, and fragmentation detection capabilities.
///
/// Uses associated functions (not `&self`) for zero-cost static dispatch.
/// Adding a new format requires:
/// 1. Implement this trait on a unit struct (e.g., `struct WebPFormat;`)
/// 2. Add corresponding detection logic in `scan/mod.rs`
pub trait FormatStrategy {
    const FORMAT: ImageFormat;
    const MAX_FILE_SIZE: u64;
    const MIN_FILE_SIZE: u64;
    const FOOTER_SIZE: u64;
    const VALIDATE_FOOTER_CONTEXT: bool;

    fn candidate_score(data: &[u8]) -> Option<u8>;

    fn detect_break(data: &[u8]) -> Option<BreakPoint>;

    fn matches_continuation(cluster_data: &[u8]) -> bool;
}
