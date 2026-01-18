#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Jpeg,
    Png,
    Unknown,
}

impl FileType {
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Unknown => "bin",
        }
    }

    #[must_use]
    pub const fn footer_size(&self) -> u64 {
        match self {
            Self::Jpeg => 2,
            Self::Png => 8,
            Self::Unknown => 0,
        }
    }

    #[must_use]
    pub const fn header_bytes(&self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD8, 0xFF],
            Self::Png => &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            Self::Unknown => &[],
        }
    }

    #[must_use]
    pub const fn footer_bytes(&self) -> &'static [u8] {
        match self {
            Self::Jpeg => &[0xFF, 0xD9],
            Self::Png => &[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82],
            Self::Unknown => &[],
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extension() {
        assert_eq!(FileType::Jpeg.extension(), "jpg");
        assert_eq!(FileType::Png.extension(), "png");
        assert_eq!(FileType::Unknown.extension(), "bin");
    }

    #[test]
    fn test_footer_size() {
        assert_eq!(FileType::Jpeg.footer_size(), 2);
        assert_eq!(FileType::Png.footer_size(), 8);
        assert_eq!(FileType::Unknown.footer_size(), 0);
    }

    #[test]
    fn test_header_bytes() {
        assert_eq!(FileType::Jpeg.header_bytes(), &[0xFF, 0xD8, 0xFF]);
        assert_eq!(FileType::Png.header_bytes().len(), 8);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", FileType::Jpeg), "JPEG");
        assert_eq!(format!("{}", FileType::Png), "PNG");
    }
}
