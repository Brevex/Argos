#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct JpegInfo {
    pub width: u16,
    pub height: u16,
    pub components: u8,
}

#[allow(dead_code)]
pub const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
#[allow(dead_code)]
pub const JPEG_EOI: [u8; 2] = [0xFF, 0xD9];

#[inline]
pub fn validate_jpeg_header(data: &[u8]) -> Option<JpegInfo> {
    if data.len() < 10 {
        return None;
    }

    if data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }

    if data.len() > 2 && data[2] != 0xFF {
        return None;
    }

    let mut pos = 2;
    while pos + 9 < data.len() {
        if data[pos] == 0xFF {
            let marker = data[pos + 1];

            if matches!(marker, 0xC0..=0xC3) {
                let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
                let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
                let components = data[pos + 9];

                if width > 0 && height > 0 {
                    return Some(JpegInfo {
                        width,
                        height,
                        components,
                    });
                }
            }

            if pos + 3 < data.len() && marker != 0x00 && marker != 0xFF && marker != 0xD9 {
                if marker >= 0xD0 && marker <= 0xD7 {
                    pos += 2;
                    continue;
                }
                let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
                pos += 2 + len;
                continue;
            }
        }
        pos += 1;
    }

    Some(JpegInfo {
        width: 0,
        height: 0,
        components: 0,
    })
}

#[inline]
#[allow(dead_code)]
pub fn find_jpeg_footer(data: &[u8]) -> Option<usize> {
    if data.len() < 2 {
        return None;
    }

    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            return Some(i);
        }
    }

    None
}

#[inline]
#[allow(dead_code)]
pub fn is_valid_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xC0..=0xCF |
        0xD0..=0xD9 |
        0xDA |
        0xDB |
        0xDC..=0xDF |
        0xE0..=0xEF |
        0xFE
    )
}

#[allow(dead_code)]
pub fn validate_jpeg_structure(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    if data[0] != 0xFF || data[1] != 0xD8 {
        return false;
    }

    let len = data.len();

    if data[len - 2] != 0xFF || data[len - 1] != 0xD9 {
        return false;
    }

    let mut pos = 2;
    let mut has_sof = false;
    let mut has_sos = false;

    while pos + 1 < len - 2 {
        if data[pos] != 0xFF {
            if has_sos {
                pos += 1;
                continue;
            }
            return false;
        }

        let marker = data[pos + 1];

        if marker == 0x00 {
            pos += 2;
            continue;
        }

        if marker == 0xFF {
            pos += 1;
            continue;
        }

        if !is_valid_marker(marker) {
            return false;
        }

        if matches!(marker, 0xC0..=0xC3) {
            has_sof = true;
        }

        if marker == 0xDA {
            has_sos = true;
        }

        if matches!(marker, 0xD0..=0xD7 | 0xD8 | 0xD9) {
            pos += 2;
            continue;
        }

        if pos + 3 >= len {
            break;
        }

        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 2 + seg_len;
    }

    has_sof && has_sos
}
