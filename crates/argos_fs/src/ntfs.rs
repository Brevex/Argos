//! NTFS deleted-file recovery.
//!
//! On-disk layout implemented (source: the NTFS on-disk structure as
//! documented by ntfs-3g/libntfs and Microsoft's file-record layout): the
//! boot sector BPB, `FILE` records with update-sequence fixups, the
//! `$STANDARD_INFORMATION`/`$FILE_NAME`/`$DATA` attributes, non-resident run
//! lists (signed cluster deltas), `INDX` index buffers (`$I30` names) and
//! `USN_RECORD_V2` journal records.
//!
//! Recovery paths, per the NTFS spec in `argos-recovery-algorithms`:
//! the live `$MFT` walk yields records flagged deleted; an independent
//! surface scan finds **orphaned** `FILE` records that survive re-formats;
//! `$I30` buffers yield name ghosts (no extents); USN records yield names
//! and timestamps of deletions.

use std::io::{Read, Seek};
use std::time::{Duration, SystemTime};

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};

use crate::bytes::{read_at, u16_le, u32_le, u64_le, utf16le_name};
use crate::{DeletedFile, FsError, FsKind, Timestamps};

/// OEM id at byte 3 of an NTFS boot sector.
const OEM_NTFS: &[u8; 8] = b"NTFS    ";

/// `FILE` record signature.
const FILE_MAGIC: &[u8; 4] = b"FILE";

/// `INDX` buffer signature.
const INDX_MAGIC: &[u8; 4] = b"INDX";

/// Attribute type ids. Source: NTFS attribute definitions.
const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;

/// Bytes of a fixed `$ATTRIBUTE_LIST` entry header, before its name.
/// Source: NTFS on-disk layout — type, length, name length, name offset,
/// starting VCN, base file reference, attribute id.
const ATTRIBUTE_LIST_ENTRY_BYTES: usize = 26;

/// Extension records one file's `$ATTRIBUTE_LIST` may name.
///
/// A file needs them when its run list outgrows one MFT record, which happens
/// to the most heavily fragmented files on a volume — the ones worth
/// recovering. This bounds what a corrupt list can make the walk read
/// (`A-BOUNDED-ALLOC`); a real file uses a handful.
const MAX_EXTENSION_RECORDS: usize = 256;
const ATTR_END: u32 = 0xFFFF_FFFF;

/// MFT record size used when no boot sector is available (orphan scan).
/// 1024 bytes is the NTFS default on every shipping cluster size.
pub const DEFAULT_RECORD_SIZE: u32 = 1024;

/// Caps on medium-derived counts, so crafted metadata cannot balloon a walk:
/// attributes per record (record is ≤ 4 KiB), runs per run list, name chars.
const MAX_ATTRS: usize = 64;
const MAX_RUNS: usize = 4096;
const MAX_NAME_CHARS: usize = 255;

/// Seconds between 1601-01-01 (FILETIME epoch) and 1970-01-01 (Unix epoch).
const FILETIME_UNIX_DIFF_SECS: u64 = 11_644_473_600;

/// Geometry of an NTFS volume, parsed from its boot sector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ntfs {
    /// Absolute byte offset of the volume start.
    pub volume_offset: ByteOffset,
    /// Bytes per cluster.
    pub cluster_bytes: u64,
    /// Absolute byte offset of the `$MFT`.
    pub mft_offset: ByteOffset,
    /// Bytes per MFT record.
    pub record_size: u32,
    /// Volume length in bytes.
    pub volume_bytes: u64,
}

impl Ntfs {
    /// Parses the boot sector at `volume_offset`; `None` when it is not a
    /// self-consistent NTFS boot sector.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults.
    pub fn open<R: Read + Seek>(
        src: &mut R,
        volume_offset: ByteOffset,
    ) -> Result<Option<Self>, FsError> {
        let mut buf = Vec::new();
        if !read_at(src, volume_offset.get(), 512, &mut buf)? {
            return Ok(None);
        }
        Ok(Self::from_boot_sector(&buf, volume_offset))
    }

    /// Interprets `sector` as an NTFS boot sector (also the residue-sweep
    /// anchor validator: consistency, never position, decides).
    #[must_use]
    pub fn from_boot_sector(sector: &[u8], volume_offset: ByteOffset) -> Option<Self> {
        if sector.get(3..11)? != OEM_NTFS || u16_le(sector, 510)? != 0xAA55 {
            return None;
        }
        let bytes_per_sector = u64::from(u16_le(sector, 11)?);
        if !(256..=4096).contains(&bytes_per_sector) || !bytes_per_sector.is_power_of_two() {
            return None;
        }
        let sectors_per_cluster = u64::from(*sector.get(13)?);
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
            return None;
        }
        let cluster_bytes = bytes_per_sector.checked_mul(sectors_per_cluster)?;
        let total_sectors = u64_le(sector, 40)?;
        let mft_cluster = u64_le(sector, 48)?;
        // Clusters-per-record is signed: negative means 2^|v| bytes.
        let raw = (*sector.get(64)?).cast_signed();
        let record_size = if raw < 0 {
            1_u32.checked_shl(u32::from(raw.unsigned_abs()))?
        } else {
            u32::try_from(u64::from(raw.cast_unsigned()).checked_mul(cluster_bytes)?).ok()?
        };
        if !(256..=4096 * 16).contains(&record_size) {
            return None;
        }
        let volume_bytes = total_sectors.checked_mul(bytes_per_sector)?;
        let mft_offset = volume_offset.checked_add(mft_cluster.checked_mul(cluster_bytes)?)?;
        Some(Self {
            volume_offset,
            cluster_bytes,
            mft_offset,
            record_size,
            volume_bytes,
        })
    }

    /// Absolute offset of MFT record `number`.
    ///
    /// Records are numbered through the `$MFT`'s own extents, so a fragmented
    /// `$MFT` puts record *n* wherever its runs place it — not at a fixed
    /// stride from the start.
    fn record_offset(&self, mft_extents: &[ByteRange], number: u64) -> Option<u64> {
        let mut skip = number.checked_mul(u64::from(self.record_size))?;
        for extent in mft_extents {
            if skip < extent.len {
                let at = extent.start.get().checked_add(skip)?;
                return (at.checked_add(u64::from(self.record_size))?
                    <= extent.end_saturating().get())
                .then_some(at);
            }
            skip -= extent.len;
        }
        None
    }

    /// Walks the `$MFT` and returns every record flagged deleted, with name,
    /// timestamps and content extents.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; corrupt records are skipped.
    pub fn recover_deleted<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<Vec<DeletedFile>, FsError> {
        let record_len = self.record_size as usize;
        let mut buf = Vec::new();

        // Record 0 is $MFT itself; its $DATA runs bound the record walk.
        if !read_at(src, self.mft_offset.get(), record_len, &mut buf)? {
            return Ok(Vec::new());
        }
        let Some(mft_record) = Record::parse(&buf.clone(), self.mft_offset.get()) else {
            return Ok(Vec::new());
        };
        let mft_extents = mft_record
            .data_extents(self)
            .unwrap_or_else(|| vec![ByteRange::new(self.mft_offset, 0)]);

        let mut found = Vec::new();
        let mut extension = Vec::new();
        for extent in &mft_extents {
            let mut at = extent.start.get();
            let end = at.saturating_add(extent.len);
            while at.saturating_add(record_len as u64) <= end {
                if !read_at(src, at, record_len, &mut buf)? {
                    break;
                }
                if let Some(mut record) = Record::parse(&buf, at)
                    && record.deleted
                {
                    // A file whose run list outgrew one record keeps the rest
                    // in extension records. Following them is what makes the
                    // most fragmented files come back whole rather than
                    // truncated at whatever fitted.
                    for number in std::mem::take(&mut record.extensions) {
                        let Some(offset) = self.record_offset(&mft_extents, number) else {
                            continue;
                        };
                        if read_at(src, offset, record_len, &mut extension)? {
                            record.absorb_extension(&extension, offset);
                        }
                    }
                    found.push(record.into_deleted_file(self));
                }
                at += record_len as u64;
            }
        }
        Ok(found)
    }
}

/// Name of the file whose change journal Windows keeps.
///
/// It lives in `$Extend`, and its journal is the alternate data stream
/// [`JOURNAL_STREAM`] rather than the file's own content — which is why a
/// reader looking only at unnamed `$DATA` finds nothing in it.
const JOURNAL_FILE: &str = "$UsnJrnl";

/// Name of the `$DATA` stream holding the change journal's records.
///
/// Written `$UsnJrnl:$J`; the name stored in the attribute is the part after
/// the colon. **Unverified against real NTFS media** — every fixture here
/// writes what this constant says, so the tests prove the reader and the
/// fixture agree and nothing more, exactly as the ioctl request codes in
/// `docs/DEVICE-SMOKE-CHECKLIST.md` do.
const JOURNAL_STREAM: &str = "$J";

/// Most of a change journal one volume will read.
///
/// `$J` is sparse and grows without bound — Windows sizes it in tens of
/// megabytes and trims the front — so a reader that took it whole would size
/// an allocation from a medium (A-BOUNDED-ALLOC). The newest records are at
/// the end, and the end is what dates a deletion, so this reads the tail.
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

/// A deletion the change journal recorded, tied to the volume it happened on.
#[derive(Clone, PartialEq, Eq)]
pub struct JournalDeletion {
    /// What the journal says about the event.
    pub event: UsnDeletion,
    /// Absolute offset of the MFT record the event names, for the geometry it
    /// was read with. This is what ties an event to a recovery.
    pub source_object: u64,
}

impl std::fmt::Debug for JournalDeletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JournalDeletion")
            .field("event", &self.event)
            .field("source_object", &self.source_object)
            .finish()
    }
}

/// Resolves a boot-sector anchor to the volume it actually describes.
///
/// NTFS keeps a copy of its boot sector in the volume's **last** sector, and a
/// residue sweep tests every sector, so it meets both. They are byte-identical,
/// and [`Ntfs::from_boot_sector`] reads whichever it is handed as the volume's
/// start — which for the copy puts the whole geometry, the `$MFT` above all,
/// almost a volume's length past where it really is. Every extent resolved
/// from that anchor then points at the wrong bytes, and every orphaned record
/// the sweep found inside the real volume falls outside the range it reports.
///
/// So the anchor is confirmed rather than trusted, by the one thing that
/// settles it: whichever reading puts a real `FILE` record where it says the
/// `$MFT` is, is the volume. Neither reading placing one means the sector
/// parsed by coincidence — which a sweep of a terabyte produces in quantity —
/// and nothing is reported for it (`A-CONFIDENCE-HONEST`).
///
/// # Errors
///
/// Fails only on I/O faults.
pub fn locate<R: Read + Seek>(src: &mut R, anchor: ByteOffset) -> Result<Option<Ntfs>, FsError> {
    let mut sector = Vec::new();
    if !read_at(src, anchor.get(), SECTOR_BYTES, &mut sector)? {
        return Ok(None);
    }
    let Some(primary) = Ntfs::from_boot_sector(&sector, anchor) else {
        return Ok(None);
    };
    if has_mft(src, primary)? {
        return Ok(Some(primary));
    }

    // Read instead as the copy in the last sector: the volume then begins a
    // volume's length back, ending just past this sector.
    let Some(start) = anchor
        .get()
        .checked_add(SECTOR_BYTES as u64)
        .and_then(|past| past.checked_sub(primary.volume_bytes))
    else {
        return Ok(None);
    };
    let Some(backup) = Ntfs::from_boot_sector(&sector, ByteOffset::new(start)) else {
        return Ok(None);
    };
    if has_mft(src, backup)? {
        return Ok(Some(backup));
    }
    Ok(None)
}

/// Bytes of a boot sector, primary or copy.
const SECTOR_BYTES: usize = 512;

/// Whether `geometry` puts a verifiable `FILE` record where it says the `$MFT`
/// begins.
///
/// Record 0 of any `$MFT` is `$MFT` itself, so a volume that is there has one
/// here. Unreadable is not confirmation: a geometry that cannot be checked is
/// one that must not be acted on.
fn has_mft<R: Read + Seek>(src: &mut R, geometry: Ntfs) -> Result<bool, FsError> {
    let want = usize::try_from(geometry.record_size.max(DEFAULT_RECORD_SIZE)).unwrap_or(usize::MAX);
    let mut buf = Vec::new();
    if !read_at(src, geometry.mft_offset.get(), want, &mut buf)? {
        return Ok(false);
    }
    Ok(is_plausible_record(&buf))
}

/// Extents of the named `$DATA` stream `stream` in a raw `FILE` record.
///
/// The record's own parser takes the *unnamed* `$DATA`, which is a file's
/// content; an alternate stream is a separate attribute of the same kind,
/// distinguished only by carrying a name. `None` when the record does not
/// parse or has no such stream.
fn named_stream_extents(raw: &[u8], stream: &str, geom: &Ntfs) -> Option<Vec<ByteRange>> {
    if raw.get(..4)? != FILE_MAGIC {
        return None;
    }
    let mut record = raw.to_vec();
    apply_fixups(&mut record)?;

    let attrs_at = usize::from(u16_le(&record, 20)?);
    let used = usize::try_from(u32_le(&record, 24)?).ok()?;
    if used > record.len() || attrs_at >= used {
        return None;
    }

    let mut at = attrs_at;
    for _ in 0..MAX_ATTRS {
        let kind = u32_le(&record, at)?;
        if kind == ATTR_END {
            break;
        }
        let len = usize::try_from(u32_le(&record, at + 4)?).ok()?;
        if len < 16 || at.checked_add(len)? > used {
            return None;
        }
        let attr = &record[at..at + len];
        if kind == ATTR_DATA {
            // Attribute name: length in characters at offset 9, offset to it
            // at 10. Both are medium-derived, so both are bounds-checked.
            let name_chars = usize::from(*attr.get(9)?);
            let name_at = usize::from(u16_le(attr, 10)?);
            if name_chars > 0
                && let Some(raw_name) = attr.get(name_at..name_at.checked_add(name_chars * 2)?)
                && utf16le_name(raw_name, MAX_NAME_CHARS).as_deref() == Some(stream)
                && *attr.get(8)? != 0
            {
                // Non-resident: a journal of any size is never resident.
                let real_size = u64_le(attr, 48)?;
                let runs_at = usize::from(u16_le(attr, 32)?);
                let runs = decode_runs(attr.get(runs_at..)?)?;
                return Runs { runs, real_size }.extents(geom);
            }
        }
        at = at.checked_add(len)?;
    }
    None
}

/// A non-resident stream's run list and the bytes it really holds.
struct Runs {
    runs: Vec<Run>,
    real_size: u64,
}

impl Runs {
    /// Absolute extents, sparse runs contributing none.
    fn extents(&self, geom: &Ntfs) -> Option<Vec<ByteRange>> {
        let mut extents = Vec::with_capacity(self.runs.len());
        let mut remaining = self.real_size;
        for &run in &self.runs {
            let bytes = run.clusters.checked_mul(geom.cluster_bytes)?.min(remaining);
            if let Some(lcn) = run.lcn {
                let start = geom
                    .volume_offset
                    .checked_add(lcn.checked_mul(geom.cluster_bytes)?)?;
                extents.push(ByteRange::new(start, bytes));
            }
            remaining = remaining.saturating_sub(bytes);
        }
        Some(extents)
    }
}

impl Ntfs {
    /// Reads this volume's `$UsnJrnl:$J` change journal, when it survived.
    ///
    /// The journal is what dates a deletion. Nothing else on an NTFS volume
    /// records *when* a file was removed — a `FILE` record keeps the times the
    /// file was created and last written, not the moment it stopped existing —
    /// so a batch of files deleted in one action is recognisable here and
    /// nowhere else: hundreds of entries sharing a timestamp.
    ///
    /// It names files and dates events. It never produces an extent, and
    /// nothing here can promote a recovery: an event is evidence that a file
    /// was deleted, not evidence that its bytes are still there
    /// (`A-CONFIDENCE-HONEST`).
    ///
    /// Returns an empty list when the volume has no journal, when it was
    /// trimmed to nothing, or when the `$MFT` no longer describes it.
    ///
    /// # Errors
    ///
    /// Fails only on I/O faults; a record that does not parse is skipped.
    pub fn change_journal<R: Read + Seek>(
        &self,
        src: &mut R,
    ) -> Result<Vec<JournalDeletion>, FsError> {
        let record_len = self.record_size as usize;
        let mut buf = Vec::new();
        if !read_at(src, self.mft_offset.get(), record_len, &mut buf)? {
            return Ok(Vec::new());
        }
        let Some(mft) = Record::parse(&buf.clone(), self.mft_offset.get()) else {
            return Ok(Vec::new());
        };
        let mft_extents = mft
            .data_extents(self)
            .unwrap_or_else(|| vec![ByteRange::new(self.mft_offset, 0)]);

        let Some(journal) = self.find_journal(src, &mft_extents, &mut buf)? else {
            return Ok(Vec::new());
        };
        let raw = Self::read_journal_tail(src, &journal)?;

        Ok(usn_deletions(&raw)
            .into_iter()
            .filter_map(|event| {
                // An event names a record number; a recovery is identified by
                // where its record sat. They meet through this volume's own
                // geometry, and an event whose record number does not resolve
                // names nothing rather than naming by coincidence.
                let at = event
                    .mft_record
                    .checked_mul(u64::from(self.record_size))
                    .and_then(|offset| self.mft_offset.checked_add(offset))?;
                Some(JournalDeletion {
                    event,
                    source_object: at.get(),
                })
            })
            .collect())
    }

    /// Extents of `$UsnJrnl:$J`, found by walking the `$MFT` for the file.
    fn find_journal<R: Read + Seek>(
        &self,
        src: &mut R,
        mft_extents: &[ByteRange],
        buf: &mut Vec<u8>,
    ) -> Result<Option<Vec<ByteRange>>, FsError> {
        let record_len = self.record_size as usize;
        for extent in mft_extents {
            let mut at = extent.start.get();
            let end = at.saturating_add(extent.len);
            while at.saturating_add(record_len as u64) <= end {
                if !read_at(src, at, record_len, buf)? {
                    break;
                }
                if Record::parse(buf, at)
                    .is_some_and(|record| record.name.as_deref() == Some(JOURNAL_FILE))
                    && let Some(extents) = named_stream_extents(buf, JOURNAL_STREAM, self)
                    && !extents.is_empty()
                {
                    return Ok(Some(extents));
                }
                at = at.saturating_add(record_len as u64);
            }
        }
        Ok(None)
    }

    /// Reads at most [`MAX_JOURNAL_BYTES`] from the end of the journal.
    ///
    /// The end is where the newest records are, and a journal Windows has been
    /// trimming has nothing but zeroes at its front.
    fn read_journal_tail<R: Read + Seek>(
        src: &mut R,
        extents: &[ByteRange],
    ) -> Result<Vec<u8>, FsError> {
        let total: u64 = extents
            .iter()
            .fold(0, |sum, extent| sum.saturating_add(extent.len));
        let skip = total.saturating_sub(MAX_JOURNAL_BYTES);
        let mut raw = Vec::new();
        let mut passed = 0_u64;
        let mut chunk = Vec::new();
        for extent in extents {
            let end = passed.saturating_add(extent.len);
            if end <= skip {
                passed = end;
                continue;
            }
            let from = skip.saturating_sub(passed);
            let want = extent.len.saturating_sub(from);
            let want = usize::try_from(want).unwrap_or(usize::MAX);
            if read_at(
                src,
                extent.start.get().saturating_add(from),
                want,
                &mut chunk,
            )? {
                raw.extend_from_slice(&chunk);
            }
            passed = end;
        }
        Ok(raw)
    }
}

/// Whether `raw` is a `FILE` record whose update-sequence array verifies.
///
/// The residue sweep uses this to recognise orphaned records by internal
/// consistency rather than by position — a stray `FILE` in file content
/// fails the fixup check.
#[must_use]
pub fn is_plausible_record(raw: &[u8]) -> bool {
    let Some(head) = raw.get(..DEFAULT_RECORD_SIZE as usize) else {
        return false;
    };
    if head.get(..4) != Some(FILE_MAGIC) {
        return false;
    }
    fixups_verify(head).is_some()
}

/// Scans `range` for orphaned `FILE` records.
///
/// Orphans are records outside any known `$MFT` — the survivors of a
/// re-format. Run lists store volume-relative cluster numbers, so
/// `volume_offset` and `cluster_bytes` must describe the volume the records
/// belong to (the residue anchor's geometry); passing the wrong volume start
/// yields extents pointing at the wrong bytes.
///
/// # Errors
///
/// Fails only on I/O faults; anything that does not validate is skipped.
pub fn orphan_scan<R: Read + Seek>(
    src: &mut R,
    range: ByteRange,
    volume_offset: ByteOffset,
    cluster_bytes: u64,
) -> Result<Vec<DeletedFile>, FsError> {
    let geom = Ntfs {
        volume_offset,
        cluster_bytes,
        mft_offset: ByteOffset::new(0),
        record_size: DEFAULT_RECORD_SIZE,
        volume_bytes: range.end().map_or(u64::MAX, ByteOffset::get),
    };
    let record_len = DEFAULT_RECORD_SIZE as usize;
    let mut found = Vec::new();
    let mut buf = Vec::new();
    let mut at = range.start.get();
    let end = at.saturating_add(range.len);
    while at.saturating_add(record_len as u64) <= end {
        if !read_at(src, at, record_len, &mut buf)? {
            break;
        }
        // The spec is explicit: only records with the in-use flag clear are
        // deleted files. Reporting live records would assert something false
        // about every file on the volume.
        if buf.get(..4) == Some(FILE_MAGIC)
            && let Some(record) = Record::parse(&buf, at)
            && record.deleted
        {
            found.push(record.into_deleted_file(&geom));
        }
        at += record_len as u64;
    }
    Ok(found)
}

/// A parsed `FILE` record after fixup verification.
struct Record {
    deleted: bool,
    /// Absolute offset the record was read from.
    position: u64,
    name: Option<String>,
    timestamps: Timestamps,
    size: u64,
    /// Resident payload as (offset within record, length), or runs.
    data: RecordData,
    /// MFT records holding the rest of this file's `$DATA` run list, in file
    /// order. Empty for a file whose runs fit in one record.
    extensions: Vec<u64>,
}

enum RecordData {
    None,
    Resident { at: usize, len: u32, record_at: u64 },
    Runs { runs: Vec<Run>, real_size: u64 },
}

impl Record {
    /// Parses and fixup-verifies one record read from absolute offset
    /// `record_at`, which resident `$DATA` extents are resolved against.
    fn parse(raw: &[u8], record_at: u64) -> Option<Self> {
        if raw.get(..4)? != FILE_MAGIC {
            return None;
        }
        let mut record = raw.to_vec();
        apply_fixups(&mut record)?;

        let flags = u16_le(&record, 22)?;
        let attrs_at = usize::from(u16_le(&record, 20)?);
        let used = usize::try_from(u32_le(&record, 24)?).ok()?;
        if used > record.len() || attrs_at >= used {
            return None;
        }

        let mut name: Option<String> = None;
        let mut namespace_rank = u8::MAX;
        let mut timestamps = Timestamps::default();
        let mut fn_timestamps = Timestamps::default();
        let mut size = 0_u64;
        let mut data = RecordData::None;
        let mut extensions: Vec<u64> = Vec::new();

        // The record's own number, so an attribute list that points back at it
        // is recognised. Source: NTFS 3.1 record header, offset 44.
        let record_number = u64::from(u32_le(&record, 44)?);

        let mut at = attrs_at;
        for _ in 0..MAX_ATTRS {
            let kind = u32_le(&record, at)?;
            if kind == ATTR_END {
                break;
            }
            let len = usize::try_from(u32_le(&record, at + 4)?).ok()?;
            if len < 16 || at.checked_add(len)? > used {
                return None;
            }
            let attr = &record[at..at + len];
            let non_resident = *attr.get(8)? != 0;

            match kind {
                ATTR_STANDARD_INFORMATION if !non_resident => {
                    let (value_at, _) = resident_value(attr)?;
                    timestamps = Timestamps {
                        created: filetime(u64_le(attr, value_at)?),
                        modified: filetime(u64_le(attr, value_at + 8)?),
                    };
                }
                ATTR_FILE_NAME if !non_resident => {
                    let (value_at, _) = resident_value(attr)?;
                    if let Some(parsed) = FileName::parse(attr.get(value_at..)?) {
                        // Prefer Win32/POSIX names over the DOS 8.3 alias.
                        let rank = match parsed.namespace {
                            2 => 2, // DOS
                            _ => 0,
                        };
                        if rank < namespace_rank {
                            namespace_rank = rank;
                            name = Some(parsed.name);
                            fn_timestamps = parsed.timestamps;
                        }
                    }
                }
                // The unnamed $DATA stream (name length zero at offset 9) is
                // the file content.
                ATTR_DATA if *attr.get(9)? == 0 => {
                    if non_resident {
                        let real_size = u64_le(attr, 48)?;
                        let runs_at = usize::from(u16_le(attr, 32)?);
                        let runs = decode_runs(attr.get(runs_at..)?)?;
                        size = real_size;
                        data = RecordData::Runs { runs, real_size };
                    } else {
                        let (value_at, value_len) = resident_value(attr)?;
                        size = u64::from(value_len);
                        data = RecordData::Resident {
                            at: at + value_at,
                            len: value_len,
                            record_at,
                        };
                    }
                }
                // A file whose run list outgrew this record keeps the rest in
                // extension records, and says so here. Without following them
                // the file's extents stop partway through — silently, and for
                // exactly the files that are most fragmented.
                ATTR_ATTRIBUTE_LIST if !non_resident => {
                    let (value_at, value_len) = resident_value(attr)?;
                    let list = attr.get(value_at..value_at + usize::try_from(value_len).ok()?)?;
                    extensions = data_extension_records(list, record_number);
                }
                _ => {}
            }
            at = at.checked_add(len)?;
        }

        if timestamps == Timestamps::default() {
            timestamps = fn_timestamps;
        }
        Some(Self {
            deleted: flags & 0x01 == 0,
            position: record_at,
            name,
            timestamps,
            size,
            data,
            extensions,
        })
    }

    /// Appends the runs an extension record holds for this file's `$DATA`.
    ///
    /// The entries of an `$ATTRIBUTE_LIST` are in starting-VCN order, so
    /// following them in order appends the file's runs in file order. Each
    /// attribute's run list is its own chain of signed deltas, so decoding one
    /// per record is correct.
    fn absorb_extension(&mut self, raw: &[u8], record_at: u64) {
        let Some(extension) = Self::parse(raw, record_at) else {
            return;
        };
        let (RecordData::Runs { runs, .. }, RecordData::Runs { runs: mine, .. }) =
            (extension.data, &mut self.data)
        else {
            return;
        };
        mine.extend(runs);
    }

    /// Content extents of the unnamed `$DATA` stream, absolute on the medium.
    fn data_extents(&self, geom: &Ntfs) -> Option<Vec<ByteRange>> {
        match &self.data {
            RecordData::None => None,
            RecordData::Resident { at, len, record_at } => Some(vec![ByteRange::new(
                ByteOffset::new(record_at.checked_add(*at as u64)?),
                u64::from(*len),
            )]),
            RecordData::Runs { runs, real_size } => {
                let mut extents = Vec::with_capacity(runs.len());
                let mut remaining = *real_size;
                for &run in runs {
                    let bytes = run.clusters.checked_mul(geom.cluster_bytes)?.min(remaining);
                    match run.lcn {
                        // A sparse run is a hole: it maps no medium bytes but
                        // still consumes file offsets, so later extents stay
                        // attributed to the right part of the file.
                        None => remaining = remaining.saturating_sub(bytes),
                        Some(lcn) => {
                            let start = geom
                                .volume_offset
                                .checked_add(lcn.checked_mul(geom.cluster_bytes)?)?;
                            extents.push(ByteRange::new(start, bytes));
                            remaining = remaining.saturating_sub(bytes);
                        }
                    }
                }
                Some(extents)
            }
        }
    }

    fn into_deleted_file(self, geom: &Ntfs) -> DeletedFile {
        let extents = self.data_extents(geom).unwrap_or_default();
        DeletedFile {
            name: self.name,
            timestamps: self.timestamps,
            size: self.size,
            extents,
            fs: FsKind::Ntfs,
            confidence: Confidence::FsMetadata,
            source_object: Some(self.position),
        }
    }
}

/// MFT record numbers an `$ATTRIBUTE_LIST` names as holding `$DATA`.
///
/// Only the unnamed stream, and only records other than the base one: an entry
/// pointing back at the record being parsed is already accounted for, and
/// following it would read the same runs twice.
fn data_extension_records(list: &[u8], base: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut at = 0_usize;
    while at + ATTRIBUTE_LIST_ENTRY_BYTES <= list.len() && out.len() < MAX_EXTENSION_RECORDS {
        let (Some(kind), Some(len)) = (u32_le(list, at), u16_le(list, at + 4)) else {
            break;
        };
        let len = usize::from(len);
        // A zero or unaligned length would not advance the walk.
        if len < ATTRIBUTE_LIST_ENTRY_BYTES || at.saturating_add(len) > list.len() {
            break;
        }
        let name_len = list.get(at + 6).copied().unwrap_or(0);
        if kind == ATTR_DATA
            && name_len == 0
            && let Some(reference) = u64_le(list, at + 16)
        {
            // The low 48 bits are the record number; the top 16 are its
            // sequence, which says nothing about where the record is.
            let record = reference & 0x0000_FFFF_FFFF_FFFF;
            if record != base && !out.contains(&record) {
                out.push(record);
            }
        }
        at += len;
    }
    out
}

/// One entry of a non-resident run list. A sparse run maps no medium bytes
/// but still covers file offsets.
#[derive(Clone, Copy)]
struct Run {
    lcn: Option<u64>,
    clusters: u64,
}

/// A `$FILE_NAME` attribute value.
struct FileName {
    name: String,
    namespace: u8,
    timestamps: Timestamps,
}

impl FileName {
    fn parse(value: &[u8]) -> Option<Self> {
        let name_len = usize::from(*value.get(64)?);
        let namespace = *value.get(65)?;
        let raw = value.get(66..66_usize.checked_add(name_len.checked_mul(2)?)?)?;
        Some(Self {
            name: utf16le_name(raw, MAX_NAME_CHARS)?,
            namespace,
            timestamps: Timestamps {
                created: filetime(u64_le(value, 8)?),
                modified: filetime(u64_le(value, 16)?),
            },
        })
    }
}

/// Verifies and removes the update-sequence fixups in place.
fn apply_fixups(record: &mut [u8]) -> Option<()> {
    let (usa_at, usa_count, _) = fixup_header(record)?;
    fixups_verify(record)?;
    for index in 1..usa_count {
        let saved = u16_le(record, usa_at.checked_add(index.checked_mul(2)?)?)?;
        let sector_end = index.checked_mul(512)?;
        let tail = sector_end.checked_sub(2)?;
        record
            .get_mut(tail..sector_end)?
            .copy_from_slice(&saved.to_le_bytes());
    }
    Some(())
}

/// The update-sequence array's position, entry count and sequence number.
fn fixup_header(record: &[u8]) -> Option<(usize, usize, u16)> {
    let usa_at = usize::from(u16_le(record, 4)?);
    let usa_count = usize::from(u16_le(record, 6)?);
    if usa_count < 2 {
        return None;
    }
    Some((usa_at, usa_count, u16_le(record, usa_at)?))
}

/// Checks a record's update-sequence array without writing to it.
///
/// The residue sweep tests this at every `FILE` signature on the medium, so it
/// must not copy a kibibyte of record just to decide whether the record is
/// plausible (`M-MEM-REUSE`). Applying the fixups — which does need a mutable
/// copy — is a separate step, for records that already passed.
fn fixups_verify(record: &[u8]) -> Option<()> {
    let (usa_at, usa_count, usn) = fixup_header(record)?;
    for index in 1..usa_count {
        // Bounds-check the slot the apply step will read and the one it will
        // write, so that step cannot fail after having written part of it.
        u16_le(record, usa_at.checked_add(index.checked_mul(2)?)?)?;
        let sector_end = index.checked_mul(512)?;
        let tail = sector_end.checked_sub(2)?;
        if u16_le(record, tail)? != usn {
            return None;
        }
        record.get(tail..sector_end)?;
    }
    Some(())
}

/// Extracts a resident attribute's (value offset, value length).
fn resident_value(attr: &[u8]) -> Option<(usize, u32)> {
    let len = u32_le(attr, 16)?;
    let at = usize::from(u16_le(attr, 20)?);
    attr.get(at..at.checked_add(usize::try_from(len).ok()?)?)?;
    Some((at, len))
}

/// Decodes a non-resident run list: per run, a header nibble pair sizing the
/// cluster count and the signed LCN delta. Sparse runs (offset size 0) are
/// skipped — they map no medium bytes.
fn decode_runs(raw: &[u8]) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut lcn = 0_i64;
    let mut at = 0_usize;
    for _ in 0..MAX_RUNS {
        let header = *raw.get(at)?;
        if header == 0 {
            return Some(runs);
        }
        let len_size = usize::from(header & 0x0F);
        let off_size = usize::from(header >> 4);
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }
        at = at.checked_add(1)?;
        let clusters = read_uint_le(raw.get(at..at.checked_add(len_size)?)?);
        at += len_size;
        if off_size == 0 {
            // Sparse run: covers file offsets, maps no medium bytes.
            runs.push(Run {
                lcn: None,
                clusters,
            });
            continue;
        }
        let delta = read_int_le(raw.get(at..at.checked_add(off_size)?)?);
        at += off_size;
        lcn = lcn.checked_add(delta)?;
        if lcn < 0 {
            return None;
        }
        #[expect(clippy::cast_sign_loss, reason = "lcn was just checked non-negative")]
        runs.push(Run {
            lcn: Some(lcn as u64),
            clusters,
        });
    }
    None
}

fn read_uint_le(raw: &[u8]) -> u64 {
    let mut value = 0_u64;
    for (index, &byte) in raw.iter().enumerate() {
        value |= u64::from(byte) << (8 * index);
    }
    value
}

fn read_int_le(raw: &[u8]) -> i64 {
    let mut value = read_uint_le(raw);
    let bits = raw.len() * 8;
    if bits < 64 && value & (1 << (bits - 1)) != 0 {
        // Sign-extend.
        value |= u64::MAX << bits;
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "the sign extension above makes this the intended two's-complement value"
    )]
    {
        value as i64
    }
}

/// A name recovered from a `$I30` index buffer.
///
/// The directory remembered a file the MFT may no longer describe; the entry
/// carries no extents, only identity.
#[derive(Clone, PartialEq, Eq)]
pub struct NameGhost {
    /// File name from the index entry.
    pub name: String,
    /// MFT record number the entry pointed at.
    pub mft_record: u64,
    /// Timestamps stored in the entry's `$FILE_NAME` copy.
    pub timestamps: Timestamps,
}

impl std::fmt::Debug for NameGhost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameGhost")
            .field("name", &"<redacted>")
            .field("mft_record", &self.mft_record)
            .field("timestamps", &self.timestamps)
            .finish()
    }
}

/// Parses every `$FILE_NAME`-bearing entry out of an `INDX` buffer,
/// including slack entries past the official end — deleted names live there.
#[must_use]
pub fn indx_names(raw: &[u8]) -> Vec<NameGhost> {
    let mut out = Vec::new();
    let Some(magic) = raw.get(..4) else {
        return out;
    };
    if magic != INDX_MAGIC {
        return out;
    }
    let mut buf = raw.to_vec();
    if apply_fixups(&mut buf).is_none() {
        return out;
    }
    // Index header at 0x18; entries start relative to it.
    let Some(entries_rel) = u32_le(&buf, 0x18) else {
        return out;
    };
    let Some(mut at) = usize::try_from(entries_rel)
        .ok()
        .and_then(|rel| rel.checked_add(0x18))
    else {
        return out;
    };
    // Walk entries to the end of the buffer (not the official size): slack
    // beyond `total_size` is exactly where deleted entries survive.
    for _ in 0..MAX_RUNS {
        let Some(entry_len) = u16_le(&buf, at.saturating_add(8)) else {
            break;
        };
        let entry_len = usize::from(entry_len);
        if entry_len < 16 {
            break;
        }
        let (Some(file_ref), Some(entry)) =
            (u64_le(&buf, at), buf.get(at..at.saturating_add(entry_len)))
        else {
            break;
        };
        if let Some(parsed) = entry.get(16..).and_then(FileName::parse) {
            out.push(NameGhost {
                name: parsed.name,
                // Low 48 bits are the record number; high 16 the sequence.
                mft_record: file_ref & 0x0000_FFFF_FFFF_FFFF,
                timestamps: parsed.timestamps,
            });
        }
        at = at.saturating_add(entry_len);
        if at >= buf.len() {
            break;
        }
    }
    out
}

/// A deletion event from the `$UsnJrnl:$J` change journal.
#[derive(Clone, PartialEq, Eq)]
pub struct UsnDeletion {
    /// Name of the file the event concerns.
    pub name: String,
    /// MFT record number of the file.
    pub mft_record: u64,
    /// Event timestamp.
    pub timestamp: Option<SystemTime>,
}

impl std::fmt::Debug for UsnDeletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsnDeletion")
            .field("name", &"<redacted>")
            .field("mft_record", &self.mft_record)
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

/// `USN_REASON_FILE_DELETE`. Source: Windows `winioctl.h`.
const USN_REASON_FILE_DELETE: u32 = 0x0000_0200;

/// Parses `USN_RECORD_V2` entries out of a `$J` byte run, keeping deletions.
#[must_use]
pub fn usn_deletions(raw: &[u8]) -> Vec<UsnDeletion> {
    let mut out = Vec::new();
    let mut at = 0_usize;
    while at.saturating_add(60) <= raw.len() {
        let Some(record_len) = u32_le(raw, at) else {
            break;
        };
        let record_len = record_len as usize;
        if record_len < 60 {
            // $J is sparse; skip zero padding to the next 8-byte boundary.
            at = at.saturating_add(8) & !7;
            continue;
        }
        let Some(record) = raw.get(at..at.saturating_add(record_len)) else {
            break;
        };
        if u16_le(record, 4) == Some(2)
            && let (Some(file_ref), Some(ticks), Some(reason)) =
                (u64_le(record, 8), u64_le(record, 32), u32_le(record, 40))
            && reason & USN_REASON_FILE_DELETE != 0
            && let (Some(name_len), Some(name_off)) = (u16_le(record, 56), u16_le(record, 58))
            && let Some(name_raw) = record.get(
                usize::from(name_off)..usize::from(name_off).saturating_add(usize::from(name_len)),
            )
            && let Some(name) = utf16le_name(name_raw, MAX_NAME_CHARS)
        {
            out.push(UsnDeletion {
                name,
                mft_record: file_ref & 0x0000_FFFF_FFFF_FFFF,
                timestamp: filetime(ticks),
            });
        }
        at = at.saturating_add(record_len);
    }
    out
}

/// FILETIME (100 ns ticks since 1601) to `SystemTime`; zero means unset.
fn filetime(ticks: u64) -> Option<SystemTime> {
    if ticks == 0 {
        return None;
    }
    let unix_ticks = i128::from(ticks) - i128::from(FILETIME_UNIX_DIFF_SECS) * 10_000_000;
    let nanos = unix_ticks.checked_mul(100)?;
    if nanos >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(u64::try_from(nanos).ok()?))
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_nanos(u64::try_from(-nanos).ok()?))
    }
}
