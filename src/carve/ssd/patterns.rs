use crate::carve::ImageFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    Header(ImageFormat),
    Footer(ImageFormat),
}

pub fn all_patterns() -> &'static [(&'static [u8], PatternKind)] {
    &[
        (&[0xFF, 0xD8], PatternKind::Header(ImageFormat::Jpeg)),
        (&[0xFF, 0xD9], PatternKind::Footer(ImageFormat::Jpeg)),
        (
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            PatternKind::Header(ImageFormat::Png),
        ),
        (&[0x49, 0x45, 0x4E, 0x44], PatternKind::Footer(ImageFormat::Png)),
    ]
}
