use crate::formats::jpeg::validate_jpeg_structure;
use crate::io::{AlignedBuffer, DiskReader};
use crate::types::{Fragment, FragmentMap, ImageFormat, RecoveredFile, RecoveryMethod};

const MIN_JPEG_BYTES: u64 = 2048;
const MIN_PNG_BYTES: u64 = 4096;
const MAX_JPEG_BYTES: u64 = 50 * 1024 * 1024;
const MAX_PNG_BYTES: u64 = 100 * 1024 * 1024;
const JPEG_EOI_SIZE: u64 = 2;
const PNG_IEND_SIZE: u64 = 12;
const CONTIGUOUS_SEARCH_LIMIT: u64 = 10 * 1024 * 1024;
const BIFRAGMENT_MAX_GAP: u64 = 50 * 1024 * 1024;
const CLUSTER_SIZES: [u64; 3] = [4096, 8192, 32768];
const MAX_BIFRAGMENT_CANDIDATES: usize = 8;

#[allow(clippy::single_range_in_vec_init)]
pub fn linear_carve(map: &FragmentMap) -> Vec<RecoveredFile> {
    let mut recovered = Vec::new();

    let jpeg_headers: Vec<_> = map.viable_jpeg_headers().collect();
    let jpeg_footers: Vec<_> = map.jpeg_footers().collect();

    for header in &jpeg_headers {
        if let Some(footer) =
            find_nearest_footer(header, &jpeg_footers, MIN_JPEG_BYTES, MAX_JPEG_BYTES)
        {
            recovered.push(RecoveredFile::new(
                vec![header.offset..footer.offset + JPEG_EOI_SIZE],
                RecoveryMethod::Linear,
                ImageFormat::Jpeg,
                header.entropy,
            ));
        }
    }

    let png_headers: Vec<_> = map.viable_png_headers().collect();
    let png_footers: Vec<_> = map.png_footers().collect();

    for header in &png_headers {
        if let Some(footer) =
            find_nearest_footer(header, &png_footers, MIN_PNG_BYTES, MAX_PNG_BYTES)
        {
            recovered.push(RecoveredFile::new(
                vec![header.offset..footer.offset + PNG_IEND_SIZE],
                RecoveryMethod::Linear,
                ImageFormat::Png,
                header.entropy,
            ));
        }
    }

    recovered
}

fn find_nearest_footer<'a>(
    header: &Fragment,
    footers: &[&'a Fragment],
    min_size: u64,
    max_size: u64,
) -> Option<&'a Fragment> {
    footers
        .iter()
        .filter(|f| {
            f.offset > header.offset && (min_size..max_size).contains(&(f.offset - header.offset))
        })
        .min_by_key(|f| f.offset)
        .copied()
}

pub fn bifragment_carve(map: &FragmentMap, reader: &mut DiskReader) -> Vec<RecoveredFile> {
    let mut recovered = Vec::new();

    let jpeg_headers: Vec<_> = map.viable_jpeg_headers().collect();
    let jpeg_footers: Vec<_> = map.jpeg_footers().collect();

    for header in &jpeg_headers {
        let has_contiguous = jpeg_footers.iter().any(|f| {
            f.offset > header.offset && f.offset - header.offset < CONTIGUOUS_SEARCH_LIMIT
        });

        if has_contiguous {
            continue;
        }

        for footer in &jpeg_footers {
            if footer.offset <= header.offset {
                continue;
            }

            let gap = footer.offset - header.offset;

            if !(MIN_JPEG_BYTES..=BIFRAGMENT_MAX_GAP).contains(&gap) {
                continue;
            }

            if let Some(file) = try_bifragment_points(header, footer, reader) {
                recovered.push(file);
                break;
            }
        }
    }

    recovered
}

fn try_bifragment_points(
    header: &Fragment,
    footer: &Fragment,
    reader: &mut DiskReader,
) -> Option<RecoveredFile> {
    let gap = footer.offset - header.offset;

    for &cluster_size in &CLUSTER_SIZES {
        let mut candidates_tested = 0;
        let mut frag_point = header.offset + cluster_size;

        while frag_point < footer.offset && candidates_tested < MAX_BIFRAGMENT_CANDIDATES {
            let first_frag_size = frag_point - header.offset;
            let second_frag_size = footer.offset + JPEG_EOI_SIZE - frag_point;

            if first_frag_size < MIN_JPEG_BYTES || second_frag_size < MIN_JPEG_BYTES {
                frag_point += cluster_size;
                candidates_tested += 1;
                continue;
            }

            let total_size = first_frag_size + second_frag_size;
            if total_size > MAX_JPEG_BYTES {
                break;
            }

            if let Some(data) = read_bifragment_candidate(
                reader,
                header.offset,
                first_frag_size,
                frag_point,
                second_frag_size,
            ) {
                if validate_jpeg_structure(&data) {
                    return Some(RecoveredFile::new(
                        vec![
                            header.offset..frag_point,
                            frag_point..footer.offset + JPEG_EOI_SIZE,
                        ],
                        RecoveryMethod::Bifragment,
                        ImageFormat::Jpeg,
                        header.entropy,
                    ));
                }
            }

            frag_point += cluster_size;
            candidates_tested += 1;
        }

        if gap > cluster_size * MAX_BIFRAGMENT_CANDIDATES as u64 {
            let mid_point = header.offset + (gap / 2);
            let aligned_mid = mid_point & !(cluster_size - 1);
            let first_frag_size = aligned_mid - header.offset;
            let second_frag_size = footer.offset + JPEG_EOI_SIZE - aligned_mid;

            if first_frag_size >= MIN_JPEG_BYTES && second_frag_size >= MIN_JPEG_BYTES {
                if let Some(data) = read_bifragment_candidate(
                    reader,
                    header.offset,
                    first_frag_size,
                    aligned_mid,
                    second_frag_size,
                ) {
                    if validate_jpeg_structure(&data) {
                        return Some(RecoveredFile::new(
                            vec![
                                header.offset..aligned_mid,
                                aligned_mid..footer.offset + JPEG_EOI_SIZE,
                            ],
                            RecoveryMethod::Bifragment,
                            ImageFormat::Jpeg,
                            header.entropy,
                        ));
                    }
                }
            }
        }
    }

    None
}

fn read_bifragment_candidate(
    reader: &mut DiskReader,
    first_offset: u64,
    first_size: u64,
    second_offset: u64,
    second_size: u64,
) -> Option<Vec<u8>> {
    let total = (first_size + second_size) as usize;
    let mut result = Vec::with_capacity(total);
    let mut buffer = AlignedBuffer::new();

    if !read_range_into(reader, first_offset, first_size, &mut result, &mut buffer) {
        return None;
    }

    if !read_range_into(reader, second_offset, second_size, &mut result, &mut buffer) {
        return None;
    }

    Some(result)
}

fn read_range_into(
    reader: &mut DiskReader,
    start: u64,
    size: u64,
    dest: &mut Vec<u8>,
    buffer: &mut AlignedBuffer,
) -> bool {
    let mut offset = start;
    let end = start + size;

    while offset < end {
        let aligned_offset = offset & !4095;
        let skip = (offset - aligned_offset) as usize;

        let n = match reader.read_at(aligned_offset, buffer) {
            Ok(n) if n > 0 => n,
            _ => return false,
        };

        let available = n.saturating_sub(skip);
        let remaining = (end - offset) as usize;
        let to_copy = available.min(remaining);

        dest.extend_from_slice(&buffer.as_slice()[skip..skip + to_copy]);
        offset += to_copy as u64;
    }

    true
}

#[derive(Debug, Default)]
pub struct RecoveryStats {
    pub jpeg_linear: usize,
    pub jpeg_bifragment: usize,
    pub png_linear: usize,
    pub png_bifragment: usize,
}

impl RecoveryStats {
    pub fn from_recovered(files: &[RecoveredFile]) -> Self {
        let mut stats = Self::default();

        for file in files {
            match (file.format, file.method) {
                (ImageFormat::Jpeg, RecoveryMethod::Linear) => stats.jpeg_linear += 1,
                (ImageFormat::Jpeg, RecoveryMethod::Bifragment) => stats.jpeg_bifragment += 1,
                (ImageFormat::Png, RecoveryMethod::Linear) => stats.png_linear += 1,
                (ImageFormat::Png, RecoveryMethod::Bifragment) => stats.png_bifragment += 1,
            }
        }
        stats
    }

    pub fn total_files(&self) -> usize {
        self.jpeg_linear + self.jpeg_bifragment + self.png_linear + self.png_bifragment
    }
}
