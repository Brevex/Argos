pub mod hdd;
pub mod ssd;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub offset: u64,
    pub length: Option<u64>,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Hdd,
    Ssd,
}
