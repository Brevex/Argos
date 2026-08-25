//! Stage C: recovering deleted files from every volume the sweep located.
//!
//! Filesystem metadata is the strongest evidence a scan has — it names the
//! file, dates it and says which extents held it — so this stage runs before
//! anything that has to re-derive those facts from the bytes.

use super::*;

/// Recovers deleted files from every volume the sweep located, current and
/// residual, plus the orphaned NTFS records left behind by a re-format.
pub(super) fn recover_filesystems<V: Read + Seek, P: ProgressSink + ?Sized>(
    control: &Control,
    views: &mut [V],
    medium_len: u64,
    sweep: &mut residue::Sweep,
    progress: &P,
    report: &mut ScanReport,
) -> Vec<Finding> {
    let Some(view) = views.first_mut() else {
        return Vec::new();
    };

    // Before anything is resolved against them: an anchor is a sector that
    // parsed, and the sweep has no way to tell the boot sector at a volume's
    // start from the copy at its end, or either from 512 bytes that satisfied
    // the checks by chance.
    // The geometry is kept, not just the corrected range: when the primary
    // boot sector is what a later format overwrote, re-deriving it from the
    // volume's start would read the very sector that is gone.
    let geometries = confirm_ntfs(view, &mut sweep.volumes, medium_len);

    let current: Vec<ByteRange> = argos_fs::part::scan(view, medium_len)
        .map(|tables| {
            tables
                .partitions
                .iter()
                .map(|partition| partition.range)
                .collect()
        })
        .unwrap_or_default();
    sweep.mark_current(&current);

    // One scratch for the whole stage: structural validation of a recovered
    // file reuses the same working memory as every other validation.
    let mut scratch = Scratch::new();
    let mut found = Vec::new();
    // Counted in volumes: one volume's metadata can take minutes to walk, and
    // a count of volumes is the only honest denominator this stage has before
    // it opens them.
    let counter = Counter::start(
        progress,
        Stage::Filesystem,
        sweep.volumes.len() as u64,
        Unit::Items,
    );
    for volume in &sweep.volumes {
        if control.is_cancelled() {
            return found;
        }
        let geometry = geometries
            .iter()
            .find(|geometry| geometry.volume_offset == volume.range.start)
            .copied();
        found.extend(recover_volume(view, *volume, geometry, &mut scratch));
        counter.step();
    }

    // Orphaned `FILE` records store volume-relative cluster numbers, so they
    // can only be resolved against the volume they belong to. A region no
    // confirmed NTFS volume covers is counted, never resolved against a guess.
    let ntfs_volumes: Vec<Volume> = sweep
        .volumes
        .iter()
        .copied()
        .filter(|volume| volume.kind == FsKind::Ntfs)
        .collect();
    for region in &sweep.ntfs_records {
        if control.is_cancelled() {
            return found;
        }
        let Some(geometry) = ntfs_volumes
            .iter()
            .find(|volume| covers(volume.range, *region))
            .and_then(|volume| {
                geometries
                    .iter()
                    .find(|geometry| geometry.volume_offset == volume.range.start)
            })
        else {
            report.unattributed_residue += 1;
            // No geometry means no extents, and that is the whole of what is
            // missing: a record still names its file, states its size and
            // carries the times it was made and last written, none of which
            // depend on where the volume began. Reading them costs one pass
            // over records already located, and skipping it is what made a
            // re-formatted disk look like a disk that never held anything.
            if let Ok(lost) = argos_fs::ntfs::orphan_records(view, *region) {
                report.lost_files.extend(lost);
            }
            continue;
        };
        if let Ok(files) = argos_fs::ntfs::orphan_scan(
            view,
            *region,
            geometry.volume_offset,
            geometry.cluster_bytes,
        ) {
            found.extend(
                files
                    .into_iter()
                    .filter_map(|file| finding_from(view, file, &mut scratch)),
            );
        }
    }

    name_from_index_slack(
        control,
        view,
        &sweep.ntfs_indexes,
        &ntfs_volumes,
        &mut found,
    );
    name_from_change_journal(control, view, &geometries, &mut found, report);
    found
}

/// Names and dates findings from what each volume's change journal recorded.
///
/// The `$UsnJrnl:$J` stream is the only place an NTFS volume records *when* a
/// file stopped existing — a `FILE` record keeps the times the file was made
/// and last written, not the moment it was removed. So a batch of files
/// deleted in one action is recognisable here and nowhere else, as a run of
/// entries sharing a timestamp.
///
/// Nothing here creates an extent or raises a tier. An event is evidence that
/// a file was deleted, which is not evidence that its bytes survived, and a
/// finding that already carries a name from its own record keeps it — that
/// record is the better evidence (`A-CONFIDENCE-HONEST`).
fn name_from_change_journal<V: Read + Seek>(
    control: &Control,
    view: &mut V,
    geometries: &[argos_fs::ntfs::Ntfs],
    found: &mut [Finding],
    report: &mut ScanReport,
) {
    if found.iter().all(|finding| finding.source_object.is_none()) {
        return;
    }
    for geometry in geometries {
        if control.is_cancelled() {
            return;
        }
        let Ok(events) = geometry.change_journal(view) else {
            continue;
        };
        report.journal_deletions = report.journal_deletions.saturating_add(events.len() as u64);
        for event in events {
            for finding in found.iter_mut() {
                if finding.source_object != Some(event.source_object) {
                    continue;
                }
                if finding.name.is_none() {
                    finding.name = Some(event.event.name.clone().into_boxed_str());
                }
                // The deletion time is a fact about the event, not about the
                // file's own timestamps, so it never overwrites one.
                if finding.deleted.is_none() {
                    finding.deleted = event.event.timestamp;
                }
            }
        }
    }
}

/// Confirms every NTFS anchor against the volume it claims, in place.
///
/// The residue sweep reports a volume for any sector that parses as an NTFS
/// boot sector, and three different things do:
///
/// - the boot sector at a volume's first sector, which is what it looks like;
/// - the copy NTFS keeps in the volume's **last** sector, byte-identical, which
///   read as a start puts the volume and its `$MFT` almost a volume's length
///   past where they are;
/// - 512 bytes that satisfied the structural checks by coincidence, which a
///   sweep of a terabyte produces in quantity.
///
/// Only the first is usable as it stands, and nothing in the bytes tells them
/// apart. [`argos_fs::ntfs::locate`] settles it by reading the `$MFT` each
/// interpretation implies: the one with a real record behind it is the volume.
/// A copy is corrected to the volume it belongs to, and a coincidence is
/// dropped rather than offered to the stages that resolve extents against a
/// volume's geometry (`A-CONFIDENCE-HONEST`).
///
/// Corrected anchors collapse: the two ends of one volume name one volume.
///
/// Returns the geometry of each confirmed volume, which is what the stages
/// below resolve against. Handing back the geometry rather than re-deriving it
/// from the volume's start is the point: the case this exists for is a primary
/// boot sector a later format overwrote, and reading it again would read the
/// sector that is gone.
fn confirm_ntfs<V: Read + Seek>(
    view: &mut V,
    volumes: &mut Vec<Volume>,
    medium_len: u64,
) -> Vec<argos_fs::ntfs::Ntfs> {
    let mut confirmed: Vec<Volume> = Vec::with_capacity(volumes.len());
    let mut geometries: Vec<argos_fs::ntfs::Ntfs> = Vec::new();
    for volume in volumes.drain(..) {
        if volume.kind != FsKind::Ntfs {
            confirmed.push(volume);
            continue;
        }
        // Unreadable counts as unconfirmed: a geometry that cannot be checked
        // is one nothing may be resolved against.
        let Ok(Some(geometry)) = argos_fs::ntfs::locate(view, volume.range.start) else {
            continue;
        };
        let remaining = medium_len.saturating_sub(geometry.volume_offset.get());
        confirmed.push(Volume {
            kind: FsKind::Ntfs,
            range: ByteRange::new(geometry.volume_offset, geometry.volume_bytes.min(remaining)),
            origin: volume.origin,
            allocation_bytes: geometry.cluster_bytes,
        });
        geometries.push(geometry);
    }
    confirmed.sort_by_key(|volume| (volume.range.start, volume.range.len));
    confirmed.dedup();
    geometries.sort_by_key(|geometry| geometry.volume_offset);
    geometries.dedup();
    *volumes = confirmed;
    geometries
}

/// Names findings from the `$FILE_NAME` copies a directory index kept.
///
/// A directory that removes an entry leaves it in the index buffer's slack, so
/// a file's name can survive when its own record's `$FILE_NAME` did not. The
/// entry carries the MFT record number, which is what ties a name to a
/// recovery — nothing here creates an extent, and a finding that already has a
/// name keeps it, because its own record is the better evidence
/// (`A-CONFIDENCE-HONEST`).
fn name_from_index_slack<V: Read + Seek>(
    control: &Control,
    view: &mut V,
    regions: &[ByteRange],
    volumes: &[Volume],
    found: &mut [Finding],
) {
    let nameless: Vec<usize> = found
        .iter()
        .enumerate()
        .filter(|(_, finding)| finding.name.is_none() && finding.source_object.is_some())
        .map(|(index, _)| index)
        .collect();
    if nameless.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for region in regions {
        if control.is_cancelled() {
            return;
        }
        // An index entry numbers a record; a finding is identified by where
        // its record sat. The two meet only through the geometry of the volume
        // the index belongs to, so an index no located volume covers names
        // nothing rather than naming by coincidence.
        let Some(geometry) = volumes
            .iter()
            .find(|volume| covers(volume.range, *region))
            .and_then(|volume| {
                argos_fs::ntfs::Ntfs::open(view, volume.range.start)
                    .ok()
                    .flatten()
            })
        else {
            continue;
        };
        let len = usize::try_from(region.len).unwrap_or(0);
        buf.clear();
        buf.resize(len, 0);
        if len == 0 || read_exact_at(view, region.start.get(), &mut buf).is_err() {
            continue;
        }
        for ghost in argos_fs::ntfs::indx_names(&buf) {
            // Where that record number sits, for an unfragmented `$MFT`. A
            // fragmented one puts it elsewhere, and then this names nothing —
            // a miss, never a wrong name.
            let Some(at) = ghost
                .mft_record
                .checked_mul(u64::from(geometry.record_size))
                .and_then(|offset| geometry.mft_offset.checked_add(offset))
            else {
                continue;
            };
            for &index in &nameless {
                let finding = &mut found[index];
                if finding.name.is_none() && finding.source_object == Some(at.get()) {
                    finding.name = Some(ghost.name.clone().into_boxed_str());
                    if finding.timestamps == argos_core::Timestamps::default() {
                        finding.timestamps = ghost.timestamps;
                    }
                }
            }
        }
    }
}

/// Recovers what one located volume's metadata still describes.
///
/// `ntfs` is the geometry [`confirm_ntfs`] established for an NTFS volume,
/// carried in rather than read again: the volume this stage most needs to
/// recover from is one whose first sector a later format overwrote, and that
/// is exactly the sector re-deriving it would read.
fn recover_volume<V: Read + Seek>(
    view: &mut V,
    volume: Volume,
    ntfs: Option<argos_fs::ntfs::Ntfs>,
    scratch: &mut Scratch,
) -> Vec<Finding> {
    let at = volume.range.start;
    let files = match volume.kind {
        FsKind::Ntfs => ntfs.and_then(|fs| fs.recover_deleted(view).ok()),
        FsKind::Ext4 => argos_fs::ext4::Ext4::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_from_journal(view).ok()),
        FsKind::Fat32 | FsKind::ExFat => argos_fs::fat::Fat::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_deleted(view).ok()),
        FsKind::Apfs => argos_fs::apfs::Apfs::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| fs.recover_deleted(view).ok()),
        // Two paths, and the second is the one that pays on a used volume: the
        // backup-root ring reaches four generations, while copy-on-write leaves
        // a deleted file's leaf on the surface long after the ring has rotated
        // past it. Both are diffed against the live trees, so a file that still
        // exists is never reported as deleted.
        FsKind::Btrfs => argos_fs::btrfs::Btrfs::open(view, at)
            .ok()
            .flatten()
            .and_then(|fs| {
                let live = fs.live(view).ok()?;
                let mut files = fs.recover_deleted(view, &live).ok()?;
                if let Ok(stale) = fs.orphan_scan(view, &live, volume.range) {
                    files.extend(stale);
                }
                Some(files)
            }),
        // A filesystem family with no metadata parser yet. Carving still
        // covers its surface; claiming a recovery here would not be honest.
        _ => None,
    };
    files
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| finding_from(view, file, scratch))
        .collect()
}

/// Turns a filesystem's claim about a deleted file into a finding — but only
/// once the medium confirms the claim.
///
/// Metadata that survives a format can point anywhere: at a boot sector, at
/// clusters since reallocated, at nothing. So the claim is checked twice. The
/// signature must be there, and the *assembled* extents must pass the format's
/// state machine — the same validation a carved candidate has to pass. A tier
/// is a statement about evidence, so the strongest tier cannot be the least
/// checked one (A-CONFIDENCE-HONEST).
///
/// A file whose bytes are there but whose structure breaks — the shape a
/// spliced hole or a reallocated run produces — is still reported, because the
/// metadata is real evidence that a file lived here, but it is reported as the
/// partial recovery it is.
fn finding_from<V: Read + Seek>(
    view: &mut V,
    file: DeletedFile,
    scratch: &mut Scratch,
) -> Option<Finding> {
    let first = file.extents.first()?;
    let mut prefix = [0_u8; argos_carve::MAX_SIGNATURE_BYTES];
    read_exact_at(view, first.start.get(), &mut prefix).ok()?;
    let format = argos_carve::identify(&prefix)?;

    let extents = file.extents.into_boxed_slice();
    let recovered = extents
        .iter()
        .fold(0_u64, |sum, extent| sum.saturating_add(extent.len));
    // Metadata claiming more bytes than the extents cover is already a partial
    // recovery, whatever the structure says.
    // Whole means: the metadata's own size fits in what was recovered, and the
    // assembled bytes really are an image. Trailing slack past the image's end
    // is not a truncation — some cameras append to their own files — so any
    // complete verdict counts.
    let whole = file.size <= recovered
        && structure_of(view, &extents, recovered, format, scratch).is_some();

    Some(Finding {
        format,
        stage: Stage::Filesystem,
        confidence: if whole {
            file.confidence
        } else {
            Confidence::PartialOrThumbnail
        },
        extents,
        declared_size: Some(file.size),
        timestamps: file.timestamps,
        deleted: None,
        name: file.name.map(String::into_boxed_str),
        source_object: file.source_object,
        parent: None,
    })
}

/// Validates the concatenated extents as `format`, returning the image length
/// the state machine confirmed.
///
/// The extents are presented as one contiguous stream, so a file the
/// filesystem stored in pieces is validated as the file it was, not as
/// whatever follows its first fragment on the medium.
fn structure_of<V: Read + Seek>(
    view: &mut V,
    extents: &[ByteRange],
    length: u64,
    format: Format,
    scratch: &mut Scratch,
) -> Option<u64> {
    let mut assembled = Assembled::new(view, extents);
    match argos_carve::validate(format, &mut assembled, ByteOffset::new(0), length, scratch) {
        Ok(Verdict::Complete { length, .. }) => Some(length),
        Ok(Verdict::Corrupt { .. }) | Err(_) => None,
    }
}
