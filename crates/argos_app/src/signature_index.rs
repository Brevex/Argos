use argos_core::FileType;

#[derive(Debug, Default)]
pub struct SignatureIndex {
    jpeg_headers: Vec<u64>,
    png_headers: Vec<u64>,

    jpeg_footers: Vec<u64>,
    png_footers: Vec<u64>,

    finalized: bool,
}

impl SignatureIndex {
    pub fn with_capacity(device_size: u64) -> Self {
        let estimated_images = (device_size / (10 * 1024 * 1024)).max(1000) as usize;
        Self {
            jpeg_headers: Vec::with_capacity(estimated_images),
            png_headers: Vec::with_capacity(estimated_images / 4),
            jpeg_footers: Vec::with_capacity(estimated_images),
            png_footers: Vec::with_capacity(estimated_images / 4),
            finalized: false,
        }
    }

    #[inline]
    pub fn add_header(&mut self, offset: u64, file_type: FileType) {
        debug_assert!(!self.finalized, "Cannot add to finalized index");
        match file_type {
            FileType::Jpeg => self.jpeg_headers.push(offset),
            FileType::Png => self.png_headers.push(offset),
            FileType::Unknown => {}
        }
    }

    #[inline]
    pub fn add_footer(&mut self, offset: u64, file_type: FileType) {
        debug_assert!(!self.finalized, "Cannot add to finalized index");
        match file_type {
            FileType::Jpeg => self.jpeg_footers.push(offset),
            FileType::Png => self.png_footers.push(offset),
            FileType::Unknown => {}
        }
    }

    pub fn finalize(&mut self) {
        self.jpeg_headers.sort_unstable();
        self.png_headers.sort_unstable();
        self.jpeg_footers.sort_unstable();
        self.png_footers.sort_unstable();
        self.finalized = true;
    }

    pub fn headers(&self, file_type: FileType) -> &[u64] {
        debug_assert!(self.finalized, "Index must be finalized before querying");
        match file_type {
            FileType::Jpeg => &self.jpeg_headers,
            FileType::Png => &self.png_headers,
            FileType::Unknown => &[],
        }
    }

    pub fn footers(&self, file_type: FileType) -> &[u64] {
        debug_assert!(self.finalized, "Index must be finalized before querying");
        match file_type {
            FileType::Jpeg => &self.jpeg_footers,
            FileType::Png => &self.png_footers,
            FileType::Unknown => &[],
        }
    }

    pub fn find_next_footer(&self, header_offset: u64, file_type: FileType) -> Option<u64> {
        let footers = self.footers(file_type);

        match footers.binary_search(&header_offset) {
            Ok(idx) => footers.get(idx + 1).copied(),
            Err(idx) => footers.get(idx).copied(),
        }
    }

    pub fn find_closest_footer(
        &self,
        header_offset: u64,
        file_type: FileType,
        max_distance: u64,
    ) -> Option<u64> {
        let footer = self.find_next_footer(header_offset, file_type)?;
        if footer <= header_offset + max_distance {
            Some(footer)
        } else {
            None
        }
    }

    pub fn stats(&self) -> IndexStats {
        IndexStats {
            jpeg_headers: self.jpeg_headers.len(),
            jpeg_footers: self.jpeg_footers.len(),
            png_headers: self.png_headers.len(),
            png_footers: self.png_footers.len(),
        }
    }

    pub fn jpeg_candidates(&self, max_file_size: u64) -> impl Iterator<Item = FileCandidate> + '_ {
        self.jpeg_headers.iter().filter_map(move |&header| {
            self.find_closest_footer(header, FileType::Jpeg, max_file_size)
                .map(|footer| FileCandidate {
                    header_offset: header,
                    footer_offset: footer,
                    file_type: FileType::Jpeg,
                })
        })
    }

    /// Iterate over all PNG headers with their potential matching footers
    pub fn png_candidates(&self, max_file_size: u64) -> impl Iterator<Item = FileCandidate> + '_ {
        self.png_headers.iter().filter_map(move |&header| {
            self.find_closest_footer(header, FileType::Png, max_file_size)
                .map(|footer| FileCandidate {
                    header_offset: header,
                    footer_offset: footer,
                    file_type: FileType::Png,
                })
        })
    }

    pub fn orphan_headers(&self, file_type: FileType, max_file_size: u64) -> Vec<u64> {
        self.headers(file_type)
            .iter()
            .filter(|&&h| {
                self.find_closest_footer(h, file_type, max_file_size)
                    .is_none()
            })
            .copied()
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FileCandidate {
    pub header_offset: u64,
    pub footer_offset: u64,
    pub file_type: FileType,
}

impl FileCandidate {
    #[inline]
    pub fn estimated_size(&self) -> u64 {
        self.footer_offset
            .saturating_sub(self.header_offset)
            .saturating_add(self.file_type.footer_size())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexStats {
    pub jpeg_headers: usize,
    pub jpeg_footers: usize,
    pub png_headers: usize,
    pub png_footers: usize,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "JPEG: {} headers / {} footers, PNG: {} headers / {} footers",
            self.jpeg_headers, self.jpeg_footers, self.png_headers, self.png_footers
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_index_basic() {
        let mut index = SignatureIndex::default();

        index.add_header(100, FileType::Jpeg);
        index.add_header(5000, FileType::Jpeg);
        index.add_footer(1000, FileType::Jpeg);
        index.add_footer(6000, FileType::Jpeg);

        index.finalize();

        assert_eq!(index.find_next_footer(100, FileType::Jpeg), Some(1000));
        assert_eq!(index.find_next_footer(5000, FileType::Jpeg), Some(6000));
        assert_eq!(index.find_next_footer(6000, FileType::Jpeg), None);
    }

    #[test]
    fn test_find_closest_footer_with_max_distance() {
        let mut index = SignatureIndex::default();

        index.add_header(100, FileType::Jpeg);
        index.add_footer(200, FileType::Jpeg);
        index.add_footer(50000, FileType::Jpeg);

        index.finalize();

        assert_eq!(
            index.find_closest_footer(100, FileType::Jpeg, 1000),
            Some(200)
        );

        assert_eq!(index.find_closest_footer(100, FileType::Jpeg, 50), None);
    }

    #[test]
    fn test_candidates_iterator() {
        let mut index = SignatureIndex::default();

        index.add_header(100, FileType::Jpeg);
        index.add_header(2000, FileType::Jpeg);
        index.add_footer(500, FileType::Jpeg);
        index.add_footer(3000, FileType::Jpeg);

        index.finalize();

        let candidates: Vec<_> = index.jpeg_candidates(10000).collect();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].header_offset, 100);
        assert_eq!(candidates[0].footer_offset, 500);
    }

    #[test]
    fn test_orphan_headers() {
        let mut index = SignatureIndex::default();

        index.add_header(100, FileType::Jpeg);
        index.add_header(50000, FileType::Jpeg);
        index.add_footer(500, FileType::Jpeg);

        index.finalize();

        let orphans = index.orphan_headers(FileType::Jpeg, 1000);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0], 50000);
    }
}
