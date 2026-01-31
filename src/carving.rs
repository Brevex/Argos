use crate::types::{Fragment, FragmentKind, ImageFormat, RecoveredFile, RecoveryMethod};

pub fn linear_carve(fragments: &[Fragment]) -> Vec<RecoveredFile> {
    let mut recovered = Vec::new();

    let jpeg_headers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::JpegHeader)
        .collect();

    let jpeg_footers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::JpegFooter)
        .collect();

    for header in &jpeg_headers {
        if let Some(footer) = find_best_footer(header, &jpeg_headers, &jpeg_footers) {
            recovered.push(RecoveredFile::new(
                vec![header.offset..footer.offset + 2],
                RecoveryMethod::Linear,
                ImageFormat::Jpeg,
            ));
        }
    }

    let png_headers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::PngHeader)
        .collect();

    let png_footers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::PngIend)
        .collect();

    for header in &png_headers {
        if let Some(footer) = find_best_png_footer(header, &png_headers, &png_footers) {
            recovered.push(RecoveredFile::new(
                vec![header.offset..footer.offset + 12],
                RecoveryMethod::Linear,
                ImageFormat::Png,
            ));
        }
    }

    recovered
}

fn find_best_footer<'a>(
    header: &Fragment,
    all_headers: &[&Fragment],
    footers: &[&'a Fragment],
) -> Option<&'a Fragment> {
    const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

    footers
        .iter()
        .filter(|f| f.offset > header.offset && f.offset - header.offset < MAX_FILE_SIZE)
        .filter(|footer| {
            !all_headers
                .iter()
                .any(|h| h.offset > header.offset && h.offset < footer.offset)
        })
        .min_by_key(|f| f.offset)
        .copied()
}

fn find_best_png_footer<'a>(
    header: &Fragment,
    all_headers: &[&Fragment],
    footers: &[&'a Fragment],
) -> Option<&'a Fragment> {
    const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

    footers
        .iter()
        .filter(|f| f.offset > header.offset && f.offset - header.offset < MAX_FILE_SIZE)
        .filter(|footer| {
            !all_headers
                .iter()
                .any(|h| h.offset > header.offset && h.offset < footer.offset)
        })
        .min_by_key(|f| f.offset)
        .copied()
}

pub fn bifragment_carve(
    fragments: &[Fragment],
    _reader: &mut crate::io::DiskReader,
) -> Vec<RecoveredFile> {
    let mut recovered = Vec::new();

    let orphan_headers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::JpegHeader)
        .collect();

    let orphan_footers: Vec<_> = fragments
        .iter()
        .filter(|f| f.kind == FragmentKind::JpegFooter)
        .collect();

    for header in &orphan_headers {
        let has_contiguous = orphan_footers.iter().any(|f| {
            f.offset > header.offset
                && f.offset - header.offset < 10 * 1024 * 1024
                && !orphan_headers
                    .iter()
                    .any(|h| h.offset > header.offset && h.offset < f.offset)
        });

        if has_contiguous {
            continue;
        }

        for footer in &orphan_footers {
            if footer.offset <= header.offset {
                continue;
            }

            let gap = footer.offset - header.offset;

            if gap > 100 * 1024 * 1024 {
                continue;
            }

            if validate_bifragment_heuristic(header, footer) {
                let estimated_first_frag_size = (gap as f64 * 0.7) as u64;
                let estimated_second_frag_start = footer.offset - (gap as f64 * 0.3) as u64;

                recovered.push(RecoveredFile::new(
                    vec![
                        header.offset..header.offset + estimated_first_frag_size,
                        estimated_second_frag_start..footer.offset + 2,
                    ],
                    RecoveryMethod::Bifragment,
                    ImageFormat::Jpeg,
                ));
                break;
            }
        }
    }

    recovered
}

fn validate_bifragment_heuristic(header: &Fragment, footer: &Fragment) -> bool {
    if header.entropy < 7.0 {
        return false;
    }
    let gap = footer.offset - header.offset;
    gap >= 1024 && gap <= 50 * 1024 * 1024
}

#[derive(Debug, Default)]
pub struct RecoveryStats {
    pub jpeg_linear: usize,
    pub jpeg_bifragment: usize,
    pub png_linear: usize,
    pub png_bifragment: usize,
    pub total_bytes: u64,
}

impl RecoveryStats {
    pub fn from_recovered(files: &[RecoveredFile]) -> Self {
        let mut stats = Self::default();

        for file in files {
            let bytes: u64 = file.fragments.iter().map(|r| r.end - r.start).sum();
            stats.total_bytes += bytes;

            match (file.format, file.method) {
                (ImageFormat::Jpeg, RecoveryMethod::Linear) => stats.jpeg_linear += 1,
                (ImageFormat::Jpeg, RecoveryMethod::Bifragment) => stats.jpeg_bifragment += 1,
                (ImageFormat::Png, RecoveryMethod::Linear) => stats.png_linear += 1,
                (ImageFormat::Png, RecoveryMethod::Bifragment) => stats.png_bifragment += 1,
                _ => {}
            }
        }
        stats
    }

    pub fn total_files(&self) -> usize {
        self.jpeg_linear + self.jpeg_bifragment + self.png_linear + self.png_bifragment
    }
}
