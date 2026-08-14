//! Synthetic filesystem-image builders (`test-util` only).
//!
//! Each builder produces a structurally valid volume with a known deleted
//! file, plus helpers for the corrupt variants every parser must survive:
//! truncation, overflowed length/count fields, cycled cross-references and
//! zero-fill. Content is synthetic — never real files.
#![expect(
    clippy::cast_possible_truncation,
    reason = "fixture layouts are fixed compile-time constants that fit their on-disk fields, \
              and byte patterns intentionally wrap"
)]

use crate::apfs::fletcher64;

/// Sector size every builder addresses in. 512 bytes is the addressing unit
/// of raw images this tool ingests.
pub const SECTOR: usize = 512;

/// A byte medium under construction.
#[derive(Clone, Debug)]
pub struct Image {
    data: Vec<u8>,
}

impl Image {
    /// A zero-filled medium of `len` bytes.
    #[must_use]
    pub fn new(len: usize) -> Self {
        Self {
            data: vec![0_u8; len],
        }
    }

    /// Writes `bytes` at `offset`.
    ///
    /// # Panics
    ///
    /// Panics if the write runs past the end, with the values.
    #[must_use]
    pub fn with(mut self, offset: usize, bytes: &[u8]) -> Self {
        assert!(
            offset + bytes.len() <= self.data.len(),
            "write at {offset}+{} runs past the {}-byte image",
            bytes.len(),
            self.data.len()
        );
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        self
    }

    /// Length of the medium in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the medium has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The composed medium.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// A synthetic file placed in a fixture volume, contiguous or fragmented.
#[derive(Clone, Debug)]
pub struct FilePlan {
    /// File name the metadata records.
    pub name: String,
    /// File content, in file order.
    pub content: Vec<u8>,
    /// Where the content lives in the image: (absolute offset, length) in
    /// file order. One part for a contiguous file, several for a fragmented
    /// one — the case filesystem metadata exists to describe.
    pub parts: Vec<(usize, usize)>,
}

impl FilePlan {
    /// A contiguous file of `len` patterned bytes named `name`, at `offset`.
    #[must_use]
    pub fn new(name: &str, offset: usize, len: usize) -> Self {
        Self {
            name: name.to_owned(),
            content: patterned(len),
            parts: vec![(offset, len)],
        }
    }

    /// A file whose content is split across `parts`, given as (absolute
    /// offset, length) pairs in file order.
    ///
    /// # Panics
    ///
    /// Panics if `parts` is empty.
    #[must_use]
    pub fn fragmented(name: &str, parts: &[(usize, usize)]) -> Self {
        assert!(!parts.is_empty(), "a file plan needs at least one part");
        let len: usize = parts.iter().map(|&(_, len)| len).sum();
        Self {
            name: name.to_owned(),
            content: patterned(len),
            parts: parts.to_vec(),
        }
    }

    /// Replaces the generated pattern with specific bytes — a real image, for
    /// tests that follow the content all the way out of the pipeline.
    ///
    /// # Panics
    ///
    /// Panics unless `content` is exactly as long as the plan's parts.
    #[must_use]
    pub fn with_content(mut self, content: Vec<u8>) -> Self {
        let planned: usize = self.parts.iter().map(|&(_, len)| len).sum();
        assert_eq!(
            content.len(),
            planned,
            "content of {} bytes does not fill the plan's {planned} bytes",
            content.len()
        );
        self.content = content;
        self
    }

    /// Absolute offset of the file's first byte.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.parts
            .first()
            .unwrap_or_else(|| unreachable!("a file plan always has a part"))
            .0
    }

    /// Writes every part's bytes into `image`.
    #[must_use]
    pub fn place(&self, mut image: Image) -> Image {
        let mut at = 0_usize;
        for &(offset, len) in &self.parts {
            image = image.with(offset, &self.content[at..at + len]);
            at += len;
        }
        image
    }
}

/// The synthetic byte pattern fixture content uses.
fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

// --- MBR / GPT ------------------------------------------------------------

/// An MBR with one partition of `sectors` sectors starting at `start_lba`.
#[must_use]
pub fn mbr(start_lba: u32, sectors: u32, type_byte: u8) -> Vec<u8> {
    let mut sector = vec![0_u8; SECTOR];
    let base = 446;
    sector[base + 4] = type_byte;
    sector[base + 8..base + 12].copy_from_slice(&start_lba.to_le_bytes());
    sector[base + 12..base + 16].copy_from_slice(&sectors.to_le_bytes());
    sector[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    sector
}

/// A GPT-partitioned image of `len` bytes covering the given inclusive LBA
/// ranges, written with primary and backup headers.
///
/// # Panics
///
/// Panics if `len` is not a multiple of the sector size.
#[must_use]
pub fn gpt_image(len: usize, partitions: &[std::ops::RangeInclusive<u64>]) -> Vec<u8> {
    assert!(
        len.is_multiple_of(SECTOR),
        "gpt image length {len} must be a multiple of {SECTOR}"
    );
    let sectors = (len / SECTOR) as u64;
    let entry_count = 128_u32;
    let entry_size = 128_u32;
    let array_bytes = (entry_count * entry_size) as usize;

    let mut array = vec![0_u8; array_bytes];
    for (index, lbas) in partitions.iter().enumerate() {
        let (first, last) = (*lbas.start(), *lbas.end());
        let at = index * entry_size as usize;
        // A non-zero type GUID marks the slot used.
        array[at..at + 16].copy_from_slice(&[0x0F; 16]);
        array[at + 16..at + 32].copy_from_slice(&[0xA0 + index as u8; 16]);
        array[at + 32..at + 40].copy_from_slice(&first.to_le_bytes());
        array[at + 40..at + 48].copy_from_slice(&last.to_le_bytes());
    }
    let mut array_crc = crc32fast::Hasher::new();
    array_crc.update(&array);
    let array_crc = array_crc.finalize();

    let entries_lba = 2_u64;
    let header = gpt_header(entries_lba, entry_count, entry_size, array_crc);
    let backup_entries_lba = sectors - 1 - (array_bytes / SECTOR) as u64;
    let backup = gpt_header(backup_entries_lba, entry_count, entry_size, array_crc);

    Image::new(len)
        .with(
            0,
            &mbr(1, u32::try_from(sectors - 1).unwrap_or(u32::MAX), 0xEE),
        )
        .with(SECTOR, &header)
        .with(entries_lba as usize * SECTOR, &array)
        .with(backup_entries_lba as usize * SECTOR, &array)
        .with(len - SECTOR, &backup)
        .into_bytes()
}

/// One GPT header sector with a correct CRC32.
#[must_use]
pub fn gpt_header(entries_lba: u64, entry_count: u32, entry_size: u32, array_crc: u32) -> Vec<u8> {
    let mut header = vec![0_u8; SECTOR];
    header[..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000_u32.to_le_bytes()); // revision 1.0
    header[12..16].copy_from_slice(&92_u32.to_le_bytes()); // header size
    header[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    header[80..84].copy_from_slice(&entry_count.to_le_bytes());
    header[84..88].copy_from_slice(&entry_size.to_le_bytes());
    header[88..92].copy_from_slice(&array_crc.to_le_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(&header[..92]);
    let value = crc.finalize();
    header[16..20].copy_from_slice(&value.to_le_bytes());
    header
}

// --- NTFS -----------------------------------------------------------------

/// Bytes per cluster in NTFS fixtures.
pub const NTFS_CLUSTER: usize = 4096;

/// Bytes per MFT record in NTFS fixtures.
pub const NTFS_RECORD: usize = 1024;

/// An NTFS volume of `len` bytes whose `$MFT` holds one deleted file.
///
/// The `$MFT` sits at `mft_offset` (volume-relative); the file's content is
/// placed at `file.offset` (image-absolute) and described by a non-resident
/// `$DATA` run.
///
/// # Panics
///
/// Panics if the layout does not fit the requested length.
#[must_use]
pub fn ntfs_volume(len: usize, mft_offset: usize, file: &FilePlan) -> Vec<u8> {
    assert!(
        mft_offset + 4 * NTFS_RECORD <= len,
        "the $MFT at {mft_offset} does not fit a {len}-byte volume"
    );
    let boot = ntfs_boot_sector(len, mft_offset);

    // Record 0: $MFT itself, one run covering four records.
    let mft_run = run_list(&[(mft_offset as u64 / NTFS_CLUSTER as u64, 1)]);
    let mft_record = ntfs_record(true, None, 0, Some(&mft_run), NTFS_CLUSTER as u64);

    // Record 1: the deleted file, one run per content part.
    let runs: Vec<(u64, u64)> = file
        .parts
        .iter()
        .map(|&(offset, len)| {
            (
                offset as u64 / NTFS_CLUSTER as u64,
                (len as u64).div_ceil(NTFS_CLUSTER as u64),
            )
        })
        .collect();
    let file_run = run_list(&runs);
    let deleted = ntfs_record(
        false,
        Some(&file.name),
        0,
        Some(&file_run),
        file.content.len() as u64,
    );

    file.place(
        Image::new(len)
            .with(0, &boot)
            // The copy NTFS keeps in the volume's last sector. A fixture
            // without it is not an NTFS volume, and a sweep that tests every
            // sector meets both — which is the case that has to work.
            .with(len - SECTOR, &boot)
            .with(mft_offset, &mft_record)
            .with(mft_offset + NTFS_RECORD, &deleted),
    )
    .into_bytes()
}

/// An NTFS volume whose deleted file keeps half its runs in an extension
/// record, named by an `$ATTRIBUTE_LIST`.
///
/// The shape NTFS gives a file whose run list outgrew one MFT record, which
/// happens to the most heavily fragmented files on a volume. A recovery that
/// reads only the base record reports the file truncated at whatever fitted.
///
/// # Panics
///
/// Panics if the volume cannot hold the layout, or if the plan has fewer than
/// two parts — there would be nothing to split across records.
#[must_use]
pub fn ntfs_volume_with_attribute_list(len: usize, mft_offset: usize, file: &FilePlan) -> Vec<u8> {
    assert!(
        mft_offset + 4 * NTFS_RECORD <= len,
        "the $MFT at {mft_offset} does not fit a {len}-byte volume"
    );
    assert!(
        file.parts.len() >= 2,
        "an attribute-list fixture needs at least two content runs"
    );
    let boot = ntfs_boot_sector(len, mft_offset);
    let mft_run = run_list(&[(mft_offset as u64 / NTFS_CLUSTER as u64, 1)]);
    let mft_record = ntfs_record(true, None, 0, Some(&mft_run), NTFS_CLUSTER as u64);

    let runs: Vec<(u64, u64)> = file
        .parts
        .iter()
        .map(|&(offset, len)| {
            (
                offset as u64 / NTFS_CLUSTER as u64,
                (len as u64).div_ceil(NTFS_CLUSTER as u64),
            )
        })
        .collect();
    let (first, rest) = runs.split_at(1);

    // Record 1 is the base: the first run, plus a list naming record 2.
    let base = ntfs_record_with_attribute_list(
        &file.name,
        &run_list(first),
        file.content.len() as u64,
        &[2],
    );
    // Record 2 carries the rest of the run list and nothing else.
    let extension = ntfs_record(false, None, 0, Some(&run_list(rest)), 0);

    file.place(
        Image::new(len)
            .with(0, &boot)
            .with(mft_offset, &mft_record)
            .with(mft_offset + NTFS_RECORD, &base)
            .with(mft_offset + 2 * NTFS_RECORD, &extension),
    )
    .into_bytes()
}

/// A deleted-file record whose `$ATTRIBUTE_LIST` names `extensions` as holding
/// the rest of its unnamed `$DATA`.
#[must_use]
pub fn ntfs_record_with_attribute_list(
    name: &str,
    runs: &[u8],
    size: u64,
    extensions: &[u64],
) -> Vec<u8> {
    let mut record = ntfs_record(false, Some(name), 0, Some(runs), size);
    // Rebuild with the list spliced in: the attribute walk stops at the end
    // marker, so the list has to sit before it.
    let usa_at = 48_usize;
    let usa_count = NTFS_RECORD / SECTOR + 1;
    unapply_fixups(&mut record, usa_at, usa_count);
    let end = u32::from_le_bytes([record[24], record[25], record[26], record[27]]) as usize - 8;

    let mut list = Vec::new();
    // One entry per extension: type $DATA, starting VCN 1, base reference the
    // extension record. Source: NTFS on-disk $ATTRIBUTE_LIST layout.
    for &record_number in extensions {
        let mut entry = vec![0_u8; 32];
        entry[0..4].copy_from_slice(&0x80_u32.to_le_bytes());
        entry[4..6].copy_from_slice(&32_u16.to_le_bytes());
        entry[6] = 0; // no attribute name
        entry[7] = 26; // name offset
        entry[8..16].copy_from_slice(&1_u64.to_le_bytes()); // starting VCN
        entry[16..24].copy_from_slice(&record_number.to_le_bytes());
        list.extend_from_slice(&entry);
    }
    let at = push_resident_attr(&mut record, end, 0x20, &list);
    record[at..at + 4].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    record[24..28].copy_from_slice(&((at + 8) as u32).to_le_bytes());
    apply_fixups(&mut record, usa_at, usa_count);
    record
}

/// An NTFS boot sector for a volume of `len` bytes with its `$MFT` at
/// `mft_offset` (a multiple of the cluster size).
#[must_use]
pub fn ntfs_boot_sector(len: usize, mft_offset: usize) -> Vec<u8> {
    let mut sector = vec![0_u8; SECTOR];
    sector[0..3].copy_from_slice(&[0xEB, 0x52, 0x90]);
    sector[3..11].copy_from_slice(b"NTFS    ");
    sector[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    sector[13] = (NTFS_CLUSTER / SECTOR) as u8;
    sector[40..48].copy_from_slice(&((len / SECTOR) as u64).to_le_bytes());
    sector[48..56].copy_from_slice(&((mft_offset / NTFS_CLUSTER) as u64).to_le_bytes());
    // Negative clusters-per-record: 2^10 = 1024 bytes.
    sector[64] = (-10_i8).cast_unsigned();
    sector[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    sector
}

/// One `FILE` record with update-sequence fixups applied.
///
/// `in_use` false marks the record deleted. When `runs` is given the `$DATA`
/// attribute is non-resident with `size` as its real size; otherwise the
/// record has no content stream.
#[must_use]
pub fn ntfs_record(
    in_use: bool,
    name: Option<&str>,
    _parent: u64,
    runs: Option<&[u8]>,
    size: u64,
) -> Vec<u8> {
    let mut record = vec![0_u8; NTFS_RECORD];
    record[0..4].copy_from_slice(b"FILE");
    let usa_at = 48_u16;
    let usa_count = (NTFS_RECORD / SECTOR + 1) as u16;
    record[4..6].copy_from_slice(&usa_at.to_le_bytes());
    record[6..8].copy_from_slice(&usa_count.to_le_bytes());
    record[20..22].copy_from_slice(&64_u16.to_le_bytes()); // attributes offset
    record[22..24].copy_from_slice(&u16::from(in_use).to_le_bytes());

    let mut at = 64_usize;

    // $STANDARD_INFORMATION with fixed FILETIMEs.
    let mut si = vec![0_u8; 48];
    si[0..8].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
    si[8..16].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
    at = push_resident_attr(&mut record, at, 0x10, &si);

    if let Some(name) = name {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut fname = vec![0_u8; 66 + units.len() * 2];
        fname[8..16].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
        fname[16..24].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
        fname[64] = u8::try_from(units.len()).unwrap_or(u8::MAX);
        fname[65] = 1; // Win32 namespace
        for (index, unit) in units.iter().enumerate() {
            fname[66 + index * 2..68 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        at = push_resident_attr(&mut record, at, 0x30, &fname);
    }

    if let Some(runs) = runs {
        at = push_nonresident_data(&mut record, at, runs, size);
    }

    record[at..at + 4].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    let used = (at + 8) as u32;
    record[24..28].copy_from_slice(&used.to_le_bytes());
    record[28..32].copy_from_slice(&(NTFS_RECORD as u32).to_le_bytes());

    apply_fixups(&mut record, usize::from(usa_at), usize::from(usa_count));
    record
}

/// A `FILE` record for `$UsnJrnl`, whose change journal is a named `$DATA`
/// stream rather than the file's own content.
///
/// The shape a reader has to handle to reach a change journal at all: the
/// unnamed `$DATA` is empty, and everything is in the alternate stream.
#[must_use]
pub fn ntfs_usn_record(stream: &str, runs: &[u8], size: u64) -> Vec<u8> {
    let mut record = vec![0_u8; NTFS_RECORD];
    record[0..4].copy_from_slice(b"FILE");
    let usa_at = 48_u16;
    let usa_count = (NTFS_RECORD / SECTOR + 1) as u16;
    record[4..6].copy_from_slice(&usa_at.to_le_bytes());
    record[6..8].copy_from_slice(&usa_count.to_le_bytes());
    record[20..22].copy_from_slice(&64_u16.to_le_bytes());
    record[22..24].copy_from_slice(&1_u16.to_le_bytes()); // in use

    let mut at = 64_usize;
    let mut si = vec![0_u8; 48];
    si[0..8].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
    si[8..16].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
    at = push_resident_attr(&mut record, at, 0x10, &si);

    let units: Vec<u16> = "$UsnJrnl".encode_utf16().collect();
    let mut fname = vec![0_u8; 66 + units.len() * 2];
    fname[8..16].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
    fname[16..24].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
    fname[64] = u8::try_from(units.len()).unwrap_or(u8::MAX);
    fname[65] = 1;
    for (index, unit) in units.iter().enumerate() {
        fname[66 + index * 2..68 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    at = push_resident_attr(&mut record, at, 0x30, &fname);

    at = push_named_nonresident_data(&mut record, at, stream, runs, size);

    record[at..at + 4].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    let used = (at + 8) as u32;
    record[24..28].copy_from_slice(&used.to_le_bytes());
    record[28..32].copy_from_slice(&(NTFS_RECORD as u32).to_le_bytes());

    apply_fixups(&mut record, usize::from(usa_at), usize::from(usa_count));
    record
}

/// Appends a non-resident `$DATA` attribute carrying a stream name.
fn push_named_nonresident_data(
    record: &mut [u8],
    at: usize,
    stream: &str,
    runs: &[u8],
    size: u64,
) -> usize {
    let units: Vec<u16> = stream.encode_utf16().collect();
    let name_at = 64_usize;
    let runs_at = name_at + units.len() * 2;
    let len = (runs_at + runs.len()).next_multiple_of(8);
    record[at..at + 4].copy_from_slice(&0x80_u32.to_le_bytes());
    record[at + 4..at + 8].copy_from_slice(&(len as u32).to_le_bytes());
    record[at + 8] = 1; // non-resident
    record[at + 9] = u8::try_from(units.len()).unwrap_or(u8::MAX);
    record[at + 10..at + 12].copy_from_slice(&(name_at as u16).to_le_bytes());
    record[at + 32..at + 34].copy_from_slice(&(runs_at as u16).to_le_bytes());
    record[at + 48..at + 56].copy_from_slice(&size.to_le_bytes());
    for (index, unit) in units.iter().enumerate() {
        let to = at + name_at + index * 2;
        record[to..to + 2].copy_from_slice(&unit.to_le_bytes());
    }
    record[at + runs_at..at + runs_at + runs.len()].copy_from_slice(runs);
    at + len
}

fn push_resident_attr(record: &mut [u8], at: usize, kind: u32, value: &[u8]) -> usize {
    let len = 24 + value.len().next_multiple_of(8);
    record[at..at + 4].copy_from_slice(&kind.to_le_bytes());
    record[at + 4..at + 8].copy_from_slice(&(len as u32).to_le_bytes());
    record[at + 8] = 0; // resident
    record[at + 16..at + 20].copy_from_slice(&(value.len() as u32).to_le_bytes());
    record[at + 20..at + 22].copy_from_slice(&24_u16.to_le_bytes());
    record[at + 24..at + 24 + value.len()].copy_from_slice(value);
    at + len
}

fn push_nonresident_data(record: &mut [u8], at: usize, runs: &[u8], size: u64) -> usize {
    let runs_at = 64_usize;
    let len = (runs_at + runs.len()).next_multiple_of(8);
    record[at..at + 4].copy_from_slice(&0x80_u32.to_le_bytes());
    record[at + 4..at + 8].copy_from_slice(&(len as u32).to_le_bytes());
    record[at + 8] = 1; // non-resident
    record[at + 32..at + 34].copy_from_slice(&(runs_at as u16).to_le_bytes());
    record[at + 48..at + 56].copy_from_slice(&size.to_le_bytes()); // real size
    record[at + runs_at..at + runs_at + runs.len()].copy_from_slice(runs);
    at + len
}

/// A deleted `FILE` record whose `$DATA` is resident — the common shape for
/// files under a few hundred bytes, whose content lives inside the record.
#[must_use]
pub fn ntfs_record_resident(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut record = vec![0_u8; NTFS_RECORD];
    record[0..4].copy_from_slice(b"FILE");
    let usa_at = 48_u16;
    let usa_count = (NTFS_RECORD / SECTOR + 1) as u16;
    record[4..6].copy_from_slice(&usa_at.to_le_bytes());
    record[6..8].copy_from_slice(&usa_count.to_le_bytes());
    record[20..22].copy_from_slice(&64_u16.to_le_bytes());
    record[22..24].copy_from_slice(&0_u16.to_le_bytes()); // deleted

    let mut at = 64_usize;
    let mut si = vec![0_u8; 48];
    si[0..8].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
    si[8..16].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
    at = push_resident_attr(&mut record, at, 0x10, &si);

    let units: Vec<u16> = name.encode_utf16().collect();
    let mut fname = vec![0_u8; 66 + units.len() * 2];
    fname[8..16].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
    fname[16..24].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
    fname[64] = u8::try_from(units.len()).unwrap_or(u8::MAX);
    fname[65] = 1;
    for (index, unit) in units.iter().enumerate() {
        fname[66 + index * 2..68 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    at = push_resident_attr(&mut record, at, 0x30, &fname);

    at = push_resident_attr(&mut record, at, 0x80, payload);

    record[at..at + 4].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    record[24..28].copy_from_slice(&((at + 8) as u32).to_le_bytes());
    record[28..32].copy_from_slice(&(NTFS_RECORD as u32).to_le_bytes());
    apply_fixups(&mut record, usize::from(usa_at), usize::from(usa_count));
    record
}

/// The byte offset of a resident `$DATA` payload within a record built by
/// [`ntfs_record_resident`], so a test can state where the bytes really are.
///
/// # Panics
///
/// Panics if `payload` is not present in `record`.
#[must_use]
pub fn resident_payload_offset(record: &[u8], payload: &[u8]) -> usize {
    record
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap_or_else(|| panic!("the resident payload must be present in the record"))
}

/// Encodes a run list of (starting LCN, cluster count) pairs.
#[must_use]
pub fn run_list(runs: &[(u64, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = 0_i64;
    for &(lcn, clusters) in runs {
        let delta = lcn.cast_signed() - previous;
        previous = lcn.cast_signed();
        let len_bytes = min_le_bytes(clusters);
        let off_bytes = min_signed_le_bytes(delta);
        out.push((off_bytes.len() as u8) << 4 | len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(&off_bytes);
    }
    out.push(0);
    out
}

fn min_le_bytes(value: u64) -> Vec<u8> {
    let mut bytes = value.to_le_bytes().to_vec();
    while bytes.len() > 1 && bytes[bytes.len() - 1] == 0 {
        bytes.pop();
    }
    // A high bit set would read as negative in the signed offset field; the
    // length field is unsigned, so only trailing zeros are trimmed here.
    bytes
}

fn min_signed_le_bytes(value: i64) -> Vec<u8> {
    let mut bytes = value.to_le_bytes().to_vec();
    let fill = if value < 0 { 0xFF } else { 0x00 };
    while bytes.len() > 1 {
        let last = bytes[bytes.len() - 1];
        let prev_sign = bytes[bytes.len() - 2] & 0x80;
        if last == fill && ((fill == 0 && prev_sign == 0) || (fill == 0xFF && prev_sign != 0)) {
            bytes.pop();
        } else {
            break;
        }
    }
    bytes
}

/// Writes the update-sequence array and stamps each sector's last two bytes.
fn apply_fixups(record: &mut [u8], usa_at: usize, usa_count: usize) {
    let usn = 0x0001_u16;
    record[usa_at..usa_at + 2].copy_from_slice(&usn.to_le_bytes());
    for index in 1..usa_count {
        let tail = index * SECTOR - 2;
        let saved = [record[tail], record[tail + 1]];
        record[usa_at + index * 2..usa_at + index * 2 + 2].copy_from_slice(&saved);
        record[tail..tail + 2].copy_from_slice(&usn.to_le_bytes());
    }
}

/// Puts back the bytes an update sequence array replaced, so a finished record
/// can be edited and fixed up again.
fn unapply_fixups(record: &mut [u8], usa_at: usize, usa_count: usize) {
    for index in 1..usa_count {
        let tail = index * SECTOR - 2;
        let saved = [record[usa_at + index * 2], record[usa_at + index * 2 + 1]];
        record[tail..tail + 2].copy_from_slice(&saved);
    }
}

/// An `INDX` index buffer holding one entry per (name, MFT record) pair.
#[must_use]
pub fn ntfs_indx(entries: &[(&str, u64)]) -> Vec<u8> {
    let mut buf = vec![0_u8; NTFS_RECORD];
    buf[0..4].copy_from_slice(b"INDX");
    let usa_at = 40_u16;
    let usa_count = (NTFS_RECORD / SECTOR + 1) as u16;
    buf[4..6].copy_from_slice(&usa_at.to_le_bytes());
    buf[6..8].copy_from_slice(&usa_count.to_le_bytes());
    // Index header at 0x18; entries start at 0x40, clear of the update
    // sequence array at 0x28.
    buf[0x18..0x1C].copy_from_slice(&0x28_u32.to_le_bytes());

    let mut at = 0x40_usize;
    for &(name, record) in entries {
        let units: Vec<u16> = name.encode_utf16().collect();
        let entry_len = (16 + 66 + units.len() * 2).next_multiple_of(8);
        buf[at..at + 8].copy_from_slice(&record.to_le_bytes());
        buf[at + 8..at + 10].copy_from_slice(&(entry_len as u16).to_le_bytes());
        let fname = at + 16;
        buf[fname + 8..fname + 16].copy_from_slice(&132_000_000_000_000_000_u64.to_le_bytes());
        buf[fname + 16..fname + 24].copy_from_slice(&132_100_000_000_000_000_u64.to_le_bytes());
        buf[fname + 64] = u8::try_from(units.len()).unwrap_or(u8::MAX);
        buf[fname + 65] = 1;
        for (index, unit) in units.iter().enumerate() {
            buf[fname + 66 + index * 2..fname + 68 + index * 2]
                .copy_from_slice(&unit.to_le_bytes());
        }
        at += entry_len;
    }
    apply_fixups(&mut buf, usize::from(usa_at), usize::from(usa_count));
    buf
}

/// A `$UsnJrnl:$J` byte run with one delete record per (name, record) pair.
#[must_use]
pub fn usn_journal(entries: &[(&str, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(name, record) in entries {
        let units: Vec<u16> = name.encode_utf16().collect();
        let name_bytes = units.len() * 2;
        let len = (60 + name_bytes).next_multiple_of(8);
        let mut buf = vec![0_u8; len];
        buf[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        buf[4..6].copy_from_slice(&2_u16.to_le_bytes()); // major version 2
        buf[8..16].copy_from_slice(&record.to_le_bytes());
        buf[32..40].copy_from_slice(&132_200_000_000_000_000_u64.to_le_bytes());
        buf[40..44].copy_from_slice(&0x0000_0200_u32.to_le_bytes()); // FILE_DELETE
        buf[56..58].copy_from_slice(&(name_bytes as u16).to_le_bytes());
        buf[58..60].copy_from_slice(&60_u16.to_le_bytes());
        for (index, unit) in units.iter().enumerate() {
            buf[60 + index * 2..62 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&buf);
    }
    out
}

// --- ext4 -----------------------------------------------------------------

/// Bytes per block in ext4 fixtures.
pub const EXT4_BLOCK: usize = 1024;

/// Inodes per group in ext4 fixtures.
pub const EXT4_INODES_PER_GROUP: u32 = 16;

/// Bytes per inode in ext4 fixtures.
pub const EXT4_INODE_BYTES: u16 = 128;

/// An ext4 volume of `len` bytes whose journal holds a stale inode-table
/// block describing `file` as deleted.
///
/// # Panics
///
/// Panics if the fixed fixture layout does not fit `len`.
#[must_use]
pub fn ext4_volume(len: usize, file: &FilePlan) -> Vec<u8> {
    // Layout (blocks of 1 KiB): 0 boot, 1 superblock, 2 group descriptors,
    // 3 inode table, 4 journal inode table copy, 5.. journal blocks,
    // then file content.
    const INODE_TABLE_BLOCK: u64 = 3;
    const JOURNAL_DESC_BLOCK: u64 = 8;
    const JOURNAL_DATA_BLOCK: u64 = 9;
    assert!(
        len >= 16 * EXT4_BLOCK,
        "ext4 fixture needs at least {} bytes, got {len}",
        16 * EXT4_BLOCK
    );

    let sb = ext4_superblock(len / EXT4_BLOCK);
    let mut gdt = vec![0_u8; EXT4_BLOCK];
    gdt[8..12].copy_from_slice(&u32::try_from(INODE_TABLE_BLOCK).unwrap_or(0).to_le_bytes());

    // The journal inode (8) lives in the live inode table; its extent tree
    // points at the journal blocks.
    let mut inode_table = vec![0_u8; EXT4_BLOCK];
    let journal_inode = ext4_inode_with_extents(
        0x8000,
        1,
        0,
        u64::try_from(EXT4_BLOCK * 4).unwrap_or(0),
        &[(JOURNAL_DESC_BLOCK, 4)],
    );
    let slot = (8 - 1) as usize * usize::from(EXT4_INODE_BYTES);
    inode_table[slot..slot + journal_inode.len()].copy_from_slice(&journal_inode);

    // The journal's descriptor block maps the following data block onto the
    // live inode table; that data block is a *stale copy* holding the deleted
    // file's inode with its extent tree intact.
    let descriptor = jbd2_descriptor(&[INODE_TABLE_BLOCK]);
    let mut stale = vec![0_u8; EXT4_BLOCK];
    let extents: Vec<(u64, u16)> = file
        .parts
        .iter()
        .map(|&(offset, len)| {
            (
                (offset / EXT4_BLOCK) as u64,
                len.div_ceil(EXT4_BLOCK) as u16,
            )
        })
        .collect();
    let deleted = ext4_inode_with_extents(
        0x8000,
        0,
        1_700_000_000,
        file.content.len() as u64,
        &extents,
    );
    // The journalled copy is one inode-table block; place the deleted inode
    // in a slot inside it (a 1 KiB block holds 8 inodes of 128 bytes).
    let deleted_slot = 5 * usize::from(EXT4_INODE_BYTES);
    stale[deleted_slot..deleted_slot + deleted.len()].copy_from_slice(&deleted);

    let image = Image::new(len)
        .with(EXT4_BLOCK, &sb)
        .with(2 * EXT4_BLOCK, &gdt)
        .with(INODE_TABLE_BLOCK as usize * EXT4_BLOCK, &inode_table)
        .with(JOURNAL_DESC_BLOCK as usize * EXT4_BLOCK, &descriptor)
        .with(JOURNAL_DATA_BLOCK as usize * EXT4_BLOCK, &stale);
    file.place(image).into_bytes()
}

/// An ext4 volume whose deleted file's extent tree is one index level deep.
///
/// The same journal-mined recovery as [`ext4_volume`], but with the inode's
/// extents pushed out into a leaf block — the shape ext4 gives a file with more
/// extents than fit in the inode, which is a heavily fragmented one.
///
/// # Panics
///
/// Panics if the volume is too small to hold the layout.
#[must_use]
pub fn ext4_volume_deep_extents(len: usize, file: &FilePlan) -> Vec<u8> {
    const INODE_TABLE_BLOCK: u64 = 3;
    const JOURNAL_DESC_BLOCK: u64 = 8;
    const JOURNAL_DATA_BLOCK: u64 = 9;
    const LEAF_BLOCK: u64 = 10;
    assert!(
        len >= 16 * EXT4_BLOCK,
        "ext4 fixture needs at least {} bytes, got {len}",
        16 * EXT4_BLOCK
    );

    let sb = ext4_superblock(len / EXT4_BLOCK);
    let mut gdt = vec![0_u8; EXT4_BLOCK];
    gdt[8..12].copy_from_slice(&u32::try_from(INODE_TABLE_BLOCK).unwrap_or(0).to_le_bytes());

    let mut inode_table = vec![0_u8; EXT4_BLOCK];
    let journal_inode = ext4_inode_with_extents(
        0x8000,
        1,
        0,
        u64::try_from(EXT4_BLOCK * 4).unwrap_or(0),
        &[(JOURNAL_DESC_BLOCK, 4)],
    );
    let slot = (8 - 1) as usize * usize::from(EXT4_INODE_BYTES);
    inode_table[slot..slot + journal_inode.len()].copy_from_slice(&journal_inode);

    let descriptor = jbd2_descriptor(&[INODE_TABLE_BLOCK]);
    let extents: Vec<(u64, u16)> = file
        .parts
        .iter()
        .map(|&(offset, len)| {
            (
                (offset / EXT4_BLOCK) as u64,
                len.div_ceil(EXT4_BLOCK) as u16,
            )
        })
        .collect();
    let leaf = ext4_extent_leaf(0, &extents);
    let deleted = ext4_inode_with_index(
        0x8000,
        0,
        1_700_000_000,
        file.content.len() as u64,
        &[LEAF_BLOCK],
    );
    let mut stale = vec![0_u8; EXT4_BLOCK];
    let deleted_slot = 5 * usize::from(EXT4_INODE_BYTES);
    stale[deleted_slot..deleted_slot + deleted.len()].copy_from_slice(&deleted);

    let image = Image::new(len)
        .with(EXT4_BLOCK, &sb)
        .with(2 * EXT4_BLOCK, &gdt)
        .with(INODE_TABLE_BLOCK as usize * EXT4_BLOCK, &inode_table)
        .with(JOURNAL_DESC_BLOCK as usize * EXT4_BLOCK, &descriptor)
        .with(JOURNAL_DATA_BLOCK as usize * EXT4_BLOCK, &stale)
        .with(LEAF_BLOCK as usize * EXT4_BLOCK, &leaf);
    file.place(image).into_bytes()
}

/// An ext4 superblock describing a `blocks`-block filesystem.
#[must_use]
pub fn ext4_superblock(blocks: usize) -> Vec<u8> {
    let mut sb = vec![0_u8; EXT4_BLOCK];
    sb[4..8].copy_from_slice(&u32::try_from(blocks).unwrap_or(u32::MAX).to_le_bytes());
    sb[24..28].copy_from_slice(&0_u32.to_le_bytes()); // log block size: 1 KiB
    sb[32..36].copy_from_slice(&8192_u32.to_le_bytes()); // blocks per group
    sb[40..44].copy_from_slice(&EXT4_INODES_PER_GROUP.to_le_bytes());
    sb[56..58].copy_from_slice(&0xEF53_u16.to_le_bytes());
    sb[88..90].copy_from_slice(&EXT4_INODE_BYTES.to_le_bytes());
    sb[224..228].copy_from_slice(&8_u32.to_le_bytes()); // journal inode
    sb
}

/// An inode with a depth-0 extent tree over (start block, block count) pairs.
#[must_use]
pub fn ext4_inode_with_extents(
    mode: u16,
    links: u16,
    dtime: u32,
    size: u64,
    extents: &[(u64, u16)],
) -> Vec<u8> {
    let mut inode = vec![0_u8; usize::from(EXT4_INODE_BYTES)];
    inode[0..2].copy_from_slice(&mode.to_le_bytes());
    inode[4..8].copy_from_slice(&u32::try_from(size & 0xFFFF_FFFF).unwrap_or(0).to_le_bytes());
    inode[12..16].copy_from_slice(&1_600_000_000_u32.to_le_bytes()); // ctime
    inode[16..20].copy_from_slice(&1_650_000_000_u32.to_le_bytes()); // mtime
    inode[20..24].copy_from_slice(&dtime.to_le_bytes());
    inode[26..28].copy_from_slice(&links.to_le_bytes());

    // i_block at 40: extent header then entries.
    inode[40..42].copy_from_slice(&0xF30A_u16.to_le_bytes());
    inode[42..44].copy_from_slice(&u16::try_from(extents.len()).unwrap_or(0).to_le_bytes());
    inode[44..46].copy_from_slice(&4_u16.to_le_bytes()); // max entries
    inode[46..48].copy_from_slice(&0_u16.to_le_bytes()); // depth
    for (index, &(block, count)) in extents.iter().enumerate() {
        let at = 40 + 12 + index * 12;
        inode[at..at + 4].copy_from_slice(&u32::try_from(index).unwrap_or(0).to_le_bytes());
        inode[at + 4..at + 6].copy_from_slice(&count.to_le_bytes());
        inode[at + 6..at + 8]
            .copy_from_slice(&u16::try_from(block >> 32).unwrap_or(0).to_le_bytes());
        inode[at + 8..at + 12].copy_from_slice(
            &u32::try_from(block & 0xFFFF_FFFF)
                .unwrap_or(0)
                .to_le_bytes(),
        );
    }
    inode
}

/// An inode whose extent tree is one index level deep.
///
/// The shape ext4 gives a file with more extents than fit in its inode — a
/// heavily fragmented one, which is exactly what a recovery is looking for.
/// The index entries point at `leaf_blocks`, and [`ext4_extent_leaf`] builds
/// what has to be written at each.
#[must_use]
pub fn ext4_inode_with_index(
    mode: u16,
    links: u16,
    dtime: u32,
    size: u64,
    leaf_blocks: &[u64],
) -> Vec<u8> {
    let mut inode = vec![0_u8; usize::from(EXT4_INODE_BYTES)];
    inode[0..2].copy_from_slice(&mode.to_le_bytes());
    inode[4..8].copy_from_slice(&u32::try_from(size & 0xFFFF_FFFF).unwrap_or(0).to_le_bytes());
    inode[12..16].copy_from_slice(&1_600_000_000_u32.to_le_bytes());
    inode[16..20].copy_from_slice(&1_650_000_000_u32.to_le_bytes());
    inode[20..24].copy_from_slice(&dtime.to_le_bytes());
    inode[26..28].copy_from_slice(&links.to_le_bytes());

    inode[40..42].copy_from_slice(&0xF30A_u16.to_le_bytes());
    inode[42..44].copy_from_slice(&u16::try_from(leaf_blocks.len()).unwrap_or(0).to_le_bytes());
    inode[44..46].copy_from_slice(&4_u16.to_le_bytes());
    inode[46..48].copy_from_slice(&1_u16.to_le_bytes()); // one index level
    for (index, &block) in leaf_blocks.iter().enumerate() {
        let at = 40 + 12 + index * 12;
        inode[at..at + 4].copy_from_slice(&u32::try_from(index).unwrap_or(0).to_le_bytes());
        inode[at + 4..at + 8].copy_from_slice(
            &u32::try_from(block & 0xFFFF_FFFF)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        inode[at + 8..at + 10]
            .copy_from_slice(&u16::try_from(block >> 32).unwrap_or(0).to_le_bytes());
    }
    inode
}

/// A depth-0 extent node as it appears in its own block.
#[must_use]
pub fn ext4_extent_leaf(first_file_block: u32, extents: &[(u64, u16)]) -> Vec<u8> {
    let mut block = vec![0_u8; EXT4_BLOCK];
    block[0..2].copy_from_slice(&0xF30A_u16.to_le_bytes());
    block[2..4].copy_from_slice(&u16::try_from(extents.len()).unwrap_or(0).to_le_bytes());
    block[4..6].copy_from_slice(&340_u16.to_le_bytes()); // max entries in a block
    block[6..8].copy_from_slice(&0_u16.to_le_bytes()); // depth
    let mut file_block = first_file_block;
    for (index, &(start, count)) in extents.iter().enumerate() {
        let at = 12 + index * 12;
        block[at..at + 4].copy_from_slice(&file_block.to_le_bytes());
        block[at + 4..at + 6].copy_from_slice(&count.to_le_bytes());
        block[at + 6..at + 8]
            .copy_from_slice(&u16::try_from(start >> 32).unwrap_or(0).to_le_bytes());
        block[at + 8..at + 12].copy_from_slice(
            &u32::try_from(start & 0xFFFF_FFFF)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        file_block += u32::from(count);
    }
    block
}

/// A jbd2 descriptor block tagging `targets` in order.
#[must_use]
pub fn jbd2_descriptor(targets: &[u64]) -> Vec<u8> {
    let mut block = vec![0_u8; EXT4_BLOCK];
    block[0..4].copy_from_slice(&0xC03B_3998_u32.to_be_bytes());
    block[4..8].copy_from_slice(&1_u32.to_be_bytes()); // descriptor block
    let mut at = 12_usize;
    for (index, &target) in targets.iter().enumerate() {
        let last = index + 1 == targets.len();
        block[at..at + 4].copy_from_slice(&u32::try_from(target).unwrap_or(0).to_be_bytes());
        // Same-uuid so no UUID follows; last-tag closes the list.
        let flags = 2_u32 | if last { 8 } else { 0 };
        block[at + 4..at + 8].copy_from_slice(&flags.to_be_bytes());
        at += 8;
    }
    block
}

/// A directory block holding `ext4_dir_entry_2` records.
#[must_use]
pub fn ext4_dir_block(entries: &[(&str, u32)]) -> Vec<u8> {
    let mut block = vec![0_u8; EXT4_BLOCK];
    let mut at = 0_usize;
    for (index, &(name, inode)) in entries.iter().enumerate() {
        let last = index + 1 == entries.len();
        let needed = (8 + name.len()).next_multiple_of(4);
        let rec_len = if last { EXT4_BLOCK - at } else { needed };
        block[at..at + 4].copy_from_slice(&inode.to_le_bytes());
        block[at + 4..at + 6].copy_from_slice(&u16::try_from(rec_len).unwrap_or(0).to_le_bytes());
        block[at + 6] = u8::try_from(name.len()).unwrap_or(0);
        block[at + 7] = 1; // regular file
        block[at + 8..at + 8 + name.len()].copy_from_slice(name.as_bytes());
        at += rec_len;
    }
    block
}

// --- FAT32 / exFAT --------------------------------------------------------

/// Bytes per cluster in FAT fixtures.
pub const FAT_CLUSTER: usize = 4096;

/// A FAT32 volume of `len` bytes with `file` deleted in the root directory.
///
/// # Panics
///
/// Panics if the fixed fixture layout does not fit `len`.
#[must_use]
pub fn fat32_volume(len: usize, file: &FilePlan) -> Vec<u8> {
    const RESERVED_SECTORS: usize = 32;
    const FAT_SECTORS: usize = 64;
    let data_start = (RESERVED_SECTORS + FAT_SECTORS) * SECTOR;
    assert!(
        data_start + 2 * FAT_CLUSTER <= len,
        "fat32 fixture needs more than {len} bytes"
    );
    let boot = fat32_boot_sector(len);
    let cluster = ((file.offset() - data_start) / FAT_CLUSTER + 2) as u32;
    let root = fat_dir_deleted(&file.name, cluster, file.content.len() as u32);

    file.place(Image::new(len).with(0, &boot).with(data_start, &root))
        .into_bytes()
}

/// A FAT32 volume whose deleted file sits in a subdirectory of the root.
///
/// The shape a person's own files actually have: the root holds folders, and
/// the photographs are inside one of them. A recovery that reads only the root
/// finds nothing here.
///
/// # Panics
///
/// Panics if the volume is too small to hold the layout.
#[must_use]
pub fn fat32_volume_with_subdirectory(len: usize, folder: &str, file: &FilePlan) -> Vec<u8> {
    const RESERVED_SECTORS: usize = 32;
    const FAT_SECTORS: usize = 64;
    let data_start = (RESERVED_SECTORS + FAT_SECTORS) * SECTOR;
    assert!(
        data_start + 4 * FAT_CLUSTER <= len,
        "fat32 fixture needs more than {len} bytes"
    );
    let boot = fat32_boot_sector(len);
    // Cluster 2 is the root, cluster 3 the subdirectory; the file's content
    // sits wherever the plan put it.
    let child_cluster = 3_u32;
    let child_at = data_start + (child_cluster as usize - 2) * FAT_CLUSTER;
    let root = fat_dir_subdirectory(folder, child_cluster);
    let cluster = ((file.offset() - data_start) / FAT_CLUSTER + 2) as u32;
    let child = fat_dir_deleted(&file.name, cluster, file.content.len() as u32);

    // The root's chain ends at cluster 2; the subdirectory's at cluster 3.
    let fat_at = RESERVED_SECTORS * SECTOR;
    let mut fat = vec![0_u8; 4 * 8];
    fat[8..12].copy_from_slice(&0x0FFF_FFFF_u32.to_le_bytes());
    fat[12..16].copy_from_slice(&0x0FFF_FFFF_u32.to_le_bytes());

    file.place(
        Image::new(len)
            .with(0, &boot)
            .with(fat_at, &fat)
            .with(data_start, &root)
            .with(child_at, &child),
    )
    .into_bytes()
}

/// A root-directory region naming one live subdirectory.
#[must_use]
pub fn fat_dir_subdirectory(name: &str, cluster: u32) -> Vec<u8> {
    let mut dir = vec![0_u8; FAT_CLUSTER];
    let short = name.to_ascii_uppercase();
    for (index, byte) in short.bytes().take(8).enumerate() {
        dir[index] = byte;
    }
    for slot in &mut dir[short.len().min(8)..11] {
        *slot = b' ';
    }
    dir[11] = 0x10; // directory
    dir[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    dir[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    dir
}

/// A FAT32 boot sector for a volume of `len` bytes.
#[must_use]
pub fn fat32_boot_sector(len: usize) -> Vec<u8> {
    let mut sector = vec![0_u8; SECTOR];
    sector[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    sector[3..11].copy_from_slice(b"MSDOS5.0");
    sector[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    sector[13] = (FAT_CLUSTER / SECTOR) as u8;
    sector[14..16].copy_from_slice(&32_u16.to_le_bytes()); // reserved sectors
    sector[16] = 1; // one FAT
    sector[17..19].copy_from_slice(&0_u16.to_le_bytes()); // FAT32 marker
    sector[32..36].copy_from_slice(&u32::try_from(len / SECTOR).unwrap_or(0).to_le_bytes());
    sector[36..40].copy_from_slice(&64_u32.to_le_bytes()); // sectors per FAT
    sector[44..48].copy_from_slice(&2_u32.to_le_bytes()); // root cluster
    sector[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    sector
}

/// A root-directory region with one deleted entry (long name + 8.3 alias).
#[must_use]
pub fn fat_dir_deleted(name: &str, cluster: u32, size: u32) -> Vec<u8> {
    let mut dir = vec![0_u8; FAT_CLUSTER];
    let units: Vec<u16> = name.encode_utf16().collect();
    let fragments = units.len().div_ceil(13).max(1);

    // Long-name entries come first, highest sequence first.
    for fragment in (0..fragments).rev() {
        let at = (fragments - 1 - fragment) * 32;
        let sequence = u8::try_from(fragment + 1).unwrap_or(1);
        dir[at] = if fragment + 1 == fragments {
            sequence | 0x40 // last logical fragment
        } else {
            sequence
        };
        dir[at + 11] = 0x0F; // long-name attribute
        let mut slots = [0xFFFF_u16; 13];
        for (index, slot) in slots.iter_mut().enumerate() {
            match units.get(fragment * 13 + index) {
                Some(&unit) => *slot = unit,
                None if fragment * 13 + index == units.len() => *slot = 0,
                None => {}
            }
        }
        for (index, unit) in slots.iter().enumerate() {
            let bytes = unit.to_le_bytes();
            let target = match index {
                0..=4 => at + 1 + index * 2,
                5..=10 => at + 14 + (index - 5) * 2,
                _ => at + 28 + (index - 11) * 2,
            };
            dir[target..target + 2].copy_from_slice(&bytes);
        }
    }

    // The 8.3 entry, marked deleted.
    let at = fragments * 32;
    dir[at] = 0xE5;
    dir[at + 1..at + 11].copy_from_slice(b"HOTO   JPG");
    dir[at + 11] = 0x20; // archive
    dir[at + 14..at + 16].copy_from_slice(&0x6000_u16.to_le_bytes()); // create time
    dir[at + 16..at + 18].copy_from_slice(&0x5621_u16.to_le_bytes()); // create date
    dir[at + 20..at + 22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    dir[at + 22..at + 24].copy_from_slice(&0x6100_u16.to_le_bytes()); // write time
    dir[at + 24..at + 26].copy_from_slice(&0x5622_u16.to_le_bytes()); // write date
    dir[at + 26..at + 28].copy_from_slice(&((cluster & 0xFFFF) as u16).to_le_bytes());
    dir[at + 28..at + 32].copy_from_slice(&size.to_le_bytes());
    dir
}

/// An exFAT volume of `len` bytes with `file` deleted in the root directory.
///
/// # Panics
///
/// Panics if the fixed fixture layout does not fit `len`.
#[must_use]
pub fn exfat_volume(len: usize, file: &FilePlan) -> Vec<u8> {
    const HEAP_SECTOR: usize = 64;
    let data_start = HEAP_SECTOR * SECTOR;
    assert!(
        data_start + 2 * FAT_CLUSTER <= len,
        "exfat fixture needs more than {len} bytes"
    );
    let boot = exfat_boot_sector(len, HEAP_SECTOR as u32);
    let cluster = ((file.offset() - data_start) / FAT_CLUSTER + 2) as u32;
    let root = exfat_dir_deleted(&file.name, cluster, file.content.len() as u64);

    file.place(Image::new(len).with(0, &boot).with(data_start, &root))
        .into_bytes()
}

/// An exFAT boot sector whose cluster heap starts at `heap_sector`.
#[must_use]
pub fn exfat_boot_sector(len: usize, heap_sector: u32) -> Vec<u8> {
    let mut sector = vec![0_u8; SECTOR];
    sector[3..11].copy_from_slice(b"EXFAT   ");
    sector[72..80].copy_from_slice(&(len as u64 / SECTOR as u64).to_le_bytes());
    sector[88..92].copy_from_slice(&heap_sector.to_le_bytes());
    sector[96..100].copy_from_slice(&2_u32.to_le_bytes()); // root cluster
    sector[108] = 9; // 512-byte sectors
    sector[109] = 3; // 8 sectors per cluster
    sector[510..512].copy_from_slice(&0xAA55_u16.to_le_bytes());
    sector
}

/// An exFAT root directory with one deleted entry set.
#[must_use]
pub fn exfat_dir_deleted(name: &str, cluster: u32, size: u64) -> Vec<u8> {
    let mut dir = vec![0_u8; FAT_CLUSTER];
    let units: Vec<u16> = name.encode_utf16().collect();
    let name_entries = units.len().div_ceil(15).max(1);

    // File entry with the in-use bit cleared.
    dir[0] = 0x85 & !0x80;
    dir[1] = u8::try_from(1 + name_entries).unwrap_or(1);
    dir[8..12].copy_from_slice(&0x5621_6000_u32.to_le_bytes()); // create timestamp
    dir[12..16].copy_from_slice(&0x5622_6100_u32.to_le_bytes()); // modify timestamp

    // Stream extension: NoFatChain set, so extents are exact.
    dir[32] = 0xC0 & !0x80;
    dir[33] = 0x02;
    dir[32 + 3] = u8::try_from(units.len()).unwrap_or(0);
    dir[32 + 20..32 + 24].copy_from_slice(&cluster.to_le_bytes());
    dir[32 + 24..32 + 32].copy_from_slice(&size.to_le_bytes());

    for entry in 0..name_entries {
        let at = 64 + entry * 32;
        dir[at] = 0xC1 & !0x80;
        for index in 0..15 {
            if let Some(&unit) = units.get(entry * 15 + index) {
                dir[at + 2 + index * 2..at + 4 + index * 2].copy_from_slice(&unit.to_le_bytes());
            }
        }
    }
    dir
}

// --- APFS -----------------------------------------------------------------

/// Bytes per block in APFS fixtures.
pub const APFS_BLOCK: usize = 4096;

/// An APFS container of `len` bytes whose previous checkpoint still describes
/// `file`, deleted in the newest checkpoint.
///
/// # Panics
///
/// Panics if the fixed fixture layout does not fit `len`.
#[must_use]
pub fn apfs_container(len: usize, file: &FilePlan) -> Vec<u8> {
    // Block layout: 0 newest NXSB, 1 descriptor NXSB (older xid),
    // 2 omap(new) 3 omap-root(new) 4 volume(new) 5 fs-tree(new),
    // 6 omap(old) 7 omap-root(old) 8 volume(old) 9 fs-tree(old).
    assert!(
        len >= 16 * APFS_BLOCK,
        "apfs fixture needs at least {} bytes, got {len}",
        16 * APFS_BLOCK
    );
    let blocks = (len / APFS_BLOCK) as u64;
    let content_block = (file.offset() / APFS_BLOCK) as u64;

    let newest = apfs_nxsb(blocks, 2, 2, 100, 1, 1);
    let older = apfs_nxsb(blocks, 6, 20, 99, 1, 1);

    let inode = 100_u64;
    let live_tree = apfs_fs_tree(&[]);
    let past_tree = apfs_fs_tree(&[FsRecord {
        inode,
        name: file.name.clone(),
        size: file.content.len() as u64,
        extent_block: content_block,
        extent_len: file.content.len() as u64,
    }]);

    let image = Image::new(len)
        .with(0, &newest)
        .with(APFS_BLOCK, &older)
        .with(2 * APFS_BLOCK, &apfs_omap(3))
        .with(3 * APFS_BLOCK, &apfs_omap_root(2, 4))
        .with(4 * APFS_BLOCK, &apfs_volume(5))
        .with(5 * APFS_BLOCK, &live_tree)
        .with(6 * APFS_BLOCK, &apfs_omap(7))
        .with(7 * APFS_BLOCK, &apfs_omap_root(20, 8))
        .with(8 * APFS_BLOCK, &apfs_volume(9))
        .with(9 * APFS_BLOCK, &past_tree);
    file.place(image).into_bytes()
}

/// A container superblock; `desc_base`/`desc_blocks` locate the checkpoint
/// ring, `volume_oid` the volume this checkpoint's omap resolves.
#[must_use]
pub fn apfs_nxsb(
    blocks: u64,
    omap_oid: u64,
    volume_oid: u64,
    xid: u64,
    desc_base: u64,
    desc_blocks: u32,
) -> Vec<u8> {
    let mut block = vec![0_u8; APFS_BLOCK];
    block[16..24].copy_from_slice(&xid.to_le_bytes());
    block[24..28].copy_from_slice(&0x0000_0001_u32.to_le_bytes()); // object type
    block[32..36].copy_from_slice(&0x4253_584E_u32.to_le_bytes()); // NXSB
    block[36..40].copy_from_slice(&(APFS_BLOCK as u32).to_le_bytes());
    block[40..48].copy_from_slice(&blocks.to_le_bytes());
    block[104..108].copy_from_slice(&desc_blocks.to_le_bytes());
    block[120..128].copy_from_slice(&desc_base.to_le_bytes());
    block[160..168].copy_from_slice(&omap_oid.to_le_bytes());
    block[184..192].copy_from_slice(&volume_oid.to_le_bytes());
    seal(block)
}

/// An object-map object pointing at its root node block.
#[must_use]
pub fn apfs_omap(root_block: u64) -> Vec<u8> {
    let mut block = vec![0_u8; APFS_BLOCK];
    block[24..28].copy_from_slice(&0x0000_000B_u32.to_le_bytes()); // omap type
    block[48..56].copy_from_slice(&root_block.to_le_bytes());
    seal(block)
}

/// An omap leaf node mapping `oid` to `paddr`.
#[must_use]
pub fn apfs_omap_root(oid: u64, paddr: u64) -> Vec<u8> {
    let mut key = vec![0_u8; 16];
    key[0..8].copy_from_slice(&oid.to_le_bytes());
    let mut value = vec![0_u8; 16];
    value[8..16].copy_from_slice(&paddr.to_le_bytes());
    btree_leaf(&[(key, value)])
}

/// A volume superblock pointing at its filesystem tree block.
#[must_use]
pub fn apfs_volume(tree_block: u64) -> Vec<u8> {
    let mut block = vec![0_u8; APFS_BLOCK];
    block[24..28].copy_from_slice(&0x0000_000D_u32.to_le_bytes()); // volume type
    block[32..36].copy_from_slice(&0x4253_5041_u32.to_le_bytes()); // APSB
    block[0x30..0x38].copy_from_slice(&tree_block.to_le_bytes());
    seal(block)
}

/// One file's records in a filesystem tree.
#[derive(Clone, Debug)]
pub struct FsRecord {
    /// Inode number.
    pub inode: u64,
    /// File name.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Physical start block of the file's single extent.
    pub extent_block: u64,
    /// Extent length in bytes.
    pub extent_len: u64,
}

/// A filesystem-tree leaf node holding inode, extent and name records.
#[must_use]
pub fn apfs_fs_tree(records: &[FsRecord]) -> Vec<u8> {
    let mut entries = Vec::new();
    for record in records {
        // Inode record: key oid tagged with the record type in the top nibble.
        let mut key = vec![0_u8; 8];
        key.copy_from_slice(&(record.inode | (3_u64 << 60)).to_le_bytes());
        let mut value = vec![0_u8; 56];
        value[8..16].copy_from_slice(&1_700_000_000_000_000_000_u64.to_le_bytes());
        value[16..24].copy_from_slice(&1_700_000_500_000_000_000_u64.to_le_bytes());
        value[48..56].copy_from_slice(&record.size.to_le_bytes());
        entries.push((key, value));

        // Extent record.
        let mut key = vec![0_u8; 16];
        key[0..8].copy_from_slice(&(record.inode | (8_u64 << 60)).to_le_bytes());
        let mut value = vec![0_u8; 16];
        value[0..8].copy_from_slice(&record.extent_len.to_le_bytes());
        value[8..16].copy_from_slice(&record.extent_block.to_le_bytes());
        entries.push((key, value));

        // Directory record: name in the key after a 10-byte header.
        let mut key = vec![0_u8; 10];
        key[0..8].copy_from_slice(&(record.inode | (9_u64 << 60)).to_le_bytes());
        key.extend_from_slice(record.name.as_bytes());
        key.push(0);
        let mut value = vec![0_u8; 8];
        value.copy_from_slice(&record.inode.to_le_bytes());
        entries.push((key, value));
    }
    btree_leaf(&entries)
}

/// An APFS filesystem tree whose index node points back at its own block —
/// the crafted cycle a bounded walker must terminate on.
#[must_use]
pub fn apfs_cyclic_tree(self_block: u64) -> Vec<u8> {
    let key = vec![0_u8; 8];
    let mut value = vec![0_u8; 8];
    value.copy_from_slice(&self_block.to_le_bytes());
    let mut block = btree_node(&[(key, value)], false);
    // Re-seal after clearing the leaf flag so the checksum still validates.
    block[0..8].fill(0);
    let checksum = fletcher64(&block[8..]);
    block[0..8].copy_from_slice(&checksum.to_le_bytes());
    block
}

/// A B-tree leaf node holding `entries` with a fixed 8-byte TOC per entry.
fn btree_leaf(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    btree_node(entries, true)
}

/// A B-tree node; `leaf` decides whether values are records or child blocks.
fn btree_node(entries: &[(Vec<u8>, Vec<u8>)], leaf: bool) -> Vec<u8> {
    const HEADER_BYTES: usize = 56;
    let mut block = vec![0_u8; APFS_BLOCK];
    block[24..28].copy_from_slice(&0x0000_0003_u32.to_le_bytes()); // btree node
    let flags: u16 = if leaf { 0x0002 } else { 0x0000 };
    block[32..34].copy_from_slice(&flags.to_le_bytes());
    block[36..40].copy_from_slice(&u32::try_from(entries.len()).unwrap_or(0).to_le_bytes());
    block[40..42].copy_from_slice(&0_u16.to_le_bytes()); // toc offset
    block[42..44].copy_from_slice(&0_u16.to_le_bytes()); // toc length

    let toc_at = HEADER_BYTES;
    let mut key_at = toc_at + entries.len() * 8;
    let mut value_end = APFS_BLOCK;
    for (index, (key, value)) in entries.iter().enumerate() {
        let entry = toc_at + index * 8;
        block[entry..entry + 2]
            .copy_from_slice(&u16::try_from(key_at - toc_at).unwrap_or(0).to_le_bytes());
        block[entry + 2..entry + 4]
            .copy_from_slice(&u16::try_from(key.len()).unwrap_or(0).to_le_bytes());
        block[key_at..key_at + key.len()].copy_from_slice(key);
        key_at += key.len();

        value_end -= value.len();
        block[entry + 4..entry + 6].copy_from_slice(
            &u16::try_from(APFS_BLOCK - value_end)
                .unwrap_or(0)
                .to_le_bytes(),
        );
        block[entry + 6..entry + 8]
            .copy_from_slice(&u16::try_from(value.len()).unwrap_or(0).to_le_bytes());
        block[value_end..value_end + value.len()].copy_from_slice(value);
    }
    seal(block)
}

/// Stamps an object's Fletcher-64 checksum into its first eight bytes.
fn seal(mut block: Vec<u8>) -> Vec<u8> {
    let checksum = fletcher64(&block[8..]);
    block[0..8].copy_from_slice(&checksum.to_le_bytes());
    block
}

// --- corrupt variants -----------------------------------------------------

/// `bytes` truncated after `keep` bytes.
///
/// # Panics
///
/// Panics if `keep` exceeds the input length, with both values.
#[must_use]
pub fn truncated(bytes: &[u8], keep: usize) -> Vec<u8> {
    assert!(
        keep <= bytes.len(),
        "cannot keep {keep} bytes of a {}-byte fixture",
        bytes.len()
    );
    bytes[..keep].to_vec()
}

/// `bytes` with a little-endian `u16` written at `at`.
///
/// # Panics
///
/// Panics if the write runs past the end, with the values.
#[must_use]
pub fn with_u16_le(bytes: &[u8], at: usize, value: u16) -> Vec<u8> {
    assert!(
        at + 2 <= bytes.len(),
        "cannot write u16 at {at} of a {}-byte fixture",
        bytes.len()
    );
    let mut out = bytes.to_vec();
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
    out
}

/// `bytes` with a little-endian `u32` written at `at`.
///
/// # Panics
///
/// Panics if the write runs past the end, with the values.
#[must_use]
pub fn with_u32_le(bytes: &[u8], at: usize, value: u32) -> Vec<u8> {
    assert!(
        at + 4 <= bytes.len(),
        "cannot write u32 at {at} of a {}-byte fixture",
        bytes.len()
    );
    let mut out = bytes.to_vec();
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    out
}

/// A zero-filled buffer of `len` bytes.
#[must_use]
pub fn zero_filled(len: usize) -> Vec<u8> {
    vec![0_u8; len]
}
