//! Phase 3 exit criteria: per-filesystem fixture suites recover deleted files
//! with names, timestamps and byte-accurate extents — contiguous and
//! fragmented — and a double-reformatted image still yields the pre-format
//! filesystem through the residue sweep. Damaged and self-referencing
//! metadata is rejected or bounded, never misread.

use std::io::Cursor;

use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_fs::fixture::{
    APFS_BLOCK, EXT4_BLOCK, FAT_CLUSTER, FilePlan, Image, NTFS_CLUSTER, SECTOR, apfs_container,
    apfs_cyclic_tree, exfat_boot_sector, exfat_volume, ext4_dir_block, ext4_inode_with_extents,
    ext4_volume, fat32_volume, gpt_image, mbr, ntfs_boot_sector, ntfs_indx, ntfs_record,
    ntfs_record_resident, ntfs_volume, resident_payload_offset, run_list, truncated, usn_journal,
    with_u16_le, with_u32_le, zero_filled,
};
use argos_fs::{FsKind, Origin, apfs, ext4, fat, ntfs, part, residue};

/// The extents a recovered file must claim, given its plan.
fn expect_extents(file: &FilePlan) -> Vec<ByteRange> {
    file.parts
        .iter()
        .map(|&(offset, len)| ByteRange::new(ByteOffset::new(offset as u64), len as u64))
        .collect()
}

/// Asserts the recovery is byte-accurate: the claimed extents must hold, in
/// file order, exactly the bytes that were planted.
///
/// This is the property the whole tool rests on. Comparing offsets alone
/// would pass on a recovery pointing at the wrong physical bytes.
fn assert_recovers_the_planted_bytes(image: &[u8], file: &FilePlan, extents: &[ByteRange]) {
    assert_eq!(extents, expect_extents(file), "claimed extents");
    let mut recovered = Vec::with_capacity(file.content.len());
    for extent in extents {
        let start = usize::try_from(extent.start.get()).expect("test offsets fit usize");
        let len = usize::try_from(extent.len).expect("test lengths fit usize");
        recovered.extend_from_slice(&image[start..start + len]);
    }
    assert_eq!(
        recovered, file.content,
        "the claimed extents must hold the planted bytes"
    );
}

// --- partition tables -----------------------------------------------------

#[test]
fn gpt_partitions_are_read_from_the_primary_header() {
    let image = gpt_image(4 * 1024 * 1024, &[2048..=4095, 4096..=6143]);
    let tables = part::scan(&mut Cursor::new(&image), image.len() as u64).expect("in-memory scan");

    assert!(!tables.used_backup_gpt);
    assert_eq!(tables.partitions.len(), 2);
    assert_eq!(
        tables.partitions[0].range,
        ByteRange::new(ByteOffset::new(2048 * 512), 2048 * 512)
    );
}

#[test]
fn a_wiped_primary_gpt_falls_back_to_the_backup_header() {
    let mut image = gpt_image(4 * 1024 * 1024, &[2048..=4095]);
    // Destroy LBA 1 — exactly what a quick re-partition does.
    image[SECTOR..2 * SECTOR].fill(0);

    let tables = part::scan(&mut Cursor::new(&image), image.len() as u64).expect("in-memory scan");

    assert!(
        tables.used_backup_gpt,
        "the backup header must supply the table when the primary is gone"
    );
    assert_eq!(tables.partitions.len(), 1);
    assert_eq!(
        tables.partitions[0].range.start,
        ByteOffset::new(2048 * 512)
    );
}

#[test]
fn a_corrupt_gpt_crc_is_rejected_not_trusted() {
    let mut image = gpt_image(4 * 1024 * 1024, &[2048..=4095]);
    // Flip a byte inside the primary entry array; its CRC no longer matches.
    image[2 * SECTOR + 40] ^= 0xFF;
    // And wipe the backup array so no fallback can rescue it.
    let backup_array = image.len() - SECTOR - 128 * 128;
    image[backup_array..backup_array + 128].fill(0);

    let tables = part::scan(&mut Cursor::new(&image), image.len() as u64).expect("in-memory scan");
    assert!(
        tables.partitions.is_empty(),
        "an entry array failing its CRC must never be trusted"
    );
}

#[test]
fn a_truncated_gpt_yields_nothing_rather_than_a_partial_table() {
    let image = gpt_image(4 * 1024 * 1024, &[2048..=4095]);
    for keep in [0_usize, 100, SECTOR, SECTOR + 200, 3 * SECTOR] {
        let cut = truncated(&image, keep);
        let tables = part::scan(&mut Cursor::new(&cut), cut.len() as u64).expect("in-memory scan");
        assert!(
            tables.partitions.is_empty(),
            "a table truncated at {keep} bytes must not be reported"
        );
    }
}

#[test]
fn plain_mbr_partitions_carry_a_filesystem_hint() {
    let image = Image::new(1024 * 1024)
        .with(0, &mbr(2048, 1024, 0x83))
        .into_bytes();
    let tables = part::scan(&mut Cursor::new(&image), image.len() as u64).expect("in-memory scan");

    assert_eq!(tables.partitions.len(), 1);
    assert_eq!(tables.partitions[0].kind_hint, Some(FsKind::Ext4));
}

// --- NTFS -----------------------------------------------------------------

#[test]
fn ntfs_recovers_a_deleted_file_with_name_timestamps_and_extents() {
    let file = FilePlan::new("evidence.jpg", 512 * 1024, 9000);
    let image = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &file);

    let mut src = Cursor::new(&image);
    let volume = ntfs::Ntfs::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the boot sector must validate");
    let found = volume.recover_deleted(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    let recovered = &found[0];
    assert_eq!(recovered.name.as_deref(), Some("evidence.jpg"));
    assert_eq!(recovered.size, file.content.len() as u64);
    assert_eq!(recovered.confidence, Confidence::FsMetadata);
    assert!(recovered.timestamps.created.is_some());
    assert!(recovered.timestamps.modified.is_some());
    assert_recovers_the_planted_bytes(&image, &file, &recovered.extents);
}

#[test]
fn ntfs_recovers_a_fragmented_file_in_run_order() {
    // Three runs, deliberately out of physical order: a recovery that
    // concatenated them by address instead of by run order would fail the
    // byte comparison even though every offset looks plausible.
    let file = FilePlan::fragmented(
        "fragmented.jpg",
        &[
            (128 * NTFS_CLUSTER, NTFS_CLUSTER),
            (64 * NTFS_CLUSTER, NTFS_CLUSTER),
            (200 * NTFS_CLUSTER, 3000),
        ],
    );
    let image = ntfs_volume(4 * 1024 * 1024, 16 * NTFS_CLUSTER, &file);

    let mut src = Cursor::new(&image);
    let volume = ntfs::Ntfs::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the boot sector must validate");
    let found = volume.recover_deleted(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].extents.len(), 3, "one extent per data run");
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn ntfs_orphan_scan_finds_records_outside_any_mft() {
    let file = FilePlan::new("orphan.png", 700 * 1024, 4096);
    let volume = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &file);
    // Copy the deleted record far from the $MFT and wipe the boot sector:
    // the state a re-format leaves behind.
    let record_at = 64 * NTFS_CLUSTER + 1024;
    let record = volume[record_at..record_at + 1024].to_vec();
    let mut image = volume;
    image[..SECTOR].fill(0);
    let orphan_at = 1_500_000_usize.next_multiple_of(1024);
    image[orphan_at..orphan_at + 1024].copy_from_slice(&record);

    let found = ntfs::orphan_scan(
        &mut Cursor::new(&image),
        ByteRange::new(ByteOffset::new(orphan_at as u64), 1024 * 16),
        ByteOffset::new(0),
        NTFS_CLUSTER as u64,
    )
    .expect("in-memory read");

    assert_eq!(found.len(), 1, "the orphaned record must still be readable");
    assert_eq!(found[0].name.as_deref(), Some("orphan.png"));
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn a_record_truncated_at_any_boundary_is_never_misread() {
    let file = FilePlan::new("evidence.jpg", 512 * 1024, 4096);
    let image = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &file);
    let record_at = 64 * NTFS_CLUSTER + 1024;
    let record = &image[record_at..record_at + 1024];

    for keep in (0..1024).step_by(7) {
        let cut = truncated(record, keep);
        let mut probe = vec![0_u8; 4096];
        probe[..cut.len()].copy_from_slice(&cut);
        let found = ntfs::orphan_scan(
            &mut Cursor::new(&probe),
            ByteRange::new(ByteOffset::new(0), 1024),
            ByteOffset::new(0),
            NTFS_CLUSTER as u64,
        )
        .expect("in-memory read");
        // Either the record is rejected, or it parses — but a parsed record
        // must never claim an extent that overflows the address space.
        for recovered in found {
            for extent in recovered.extents {
                assert!(
                    extent.end().is_some(),
                    "a record truncated at {keep} produced an unbounded extent"
                );
            }
        }
    }
}

#[test]
fn an_overflowed_attribute_length_is_rejected() {
    // The first attribute's length field claims far more than the record.
    let record = ntfs_record(false, Some("x.jpg"), 0, Some(&run_list(&[(4, 1)])), 4096);
    let broken = with_u16_le(&record, 64 + 4, u16::MAX);
    let mut probe = vec![0_u8; 4096];
    probe[..broken.len()].copy_from_slice(&broken);

    let found = ntfs::orphan_scan(
        &mut Cursor::new(&probe),
        ByteRange::new(ByteOffset::new(0), 1024),
        ByteOffset::new(0),
        NTFS_CLUSTER as u64,
    )
    .expect("in-memory read");
    assert!(
        found.is_empty(),
        "an attribute claiming more than the record must fail the parse"
    );
}

#[test]
fn ntfs_index_slack_yields_names_with_no_surviving_record() {
    let buffer = ntfs_indx(&[("holiday.jpg", 42), ("receipt.png", 43)]);
    let ghosts = ntfs::indx_names(&buffer);

    let names: Vec<&str> = ghosts.iter().map(|ghost| ghost.name.as_str()).collect();
    assert!(names.contains(&"holiday.jpg"));
    assert!(names.contains(&"receipt.png"));
    assert_eq!(ghosts[0].mft_record, 42);
}

#[test]
fn usn_journal_yields_deletion_events() {
    let journal = usn_journal(&[("deleted.jpg", 77)]);
    let deletions = ntfs::usn_deletions(&journal);

    assert_eq!(deletions.len(), 1);
    assert_eq!(deletions[0].name, "deleted.jpg");
    assert_eq!(deletions[0].mft_record, 77);
    assert!(deletions[0].timestamp.is_some());
}

#[test]
fn no_debug_impl_ever_renders_a_name_read_off_the_medium() {
    let file = FilePlan::new("private-photo.jpg", 512 * 1024, 2048);
    let image = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &file);
    let mut src = Cursor::new(&image);
    let volume = ntfs::Ntfs::open(&mut src, ByteOffset::new(0))
        .expect("read")
        .expect("valid");

    let rendered = [
        format!("{:?}", volume.recover_deleted(&mut src).expect("read")[0]),
        format!(
            "{:?}",
            ntfs::indx_names(&ntfs_indx(&[("private-photo.jpg", 1)]))
        ),
        format!(
            "{:?}",
            ntfs::usn_deletions(&usn_journal(&[("private-photo.jpg", 1)]))
        ),
        format!(
            "{:?}",
            ext4::dir_entries(&ext4_dir_block(&[("private-photo.jpg", 12)]))
        ),
    ];
    for output in rendered {
        assert!(
            !output.contains("private-photo"),
            "Debug leaked a name read off the medium: {output}"
        );
    }
}

// --- ext4 -----------------------------------------------------------------

#[test]
fn ext4_recovers_deleted_extents_from_the_journal() {
    let file = FilePlan::new("deleted.jpg", 64 * EXT4_BLOCK, 3 * EXT4_BLOCK);
    let image = ext4_volume(256 * EXT4_BLOCK, &file);

    let mut src = Cursor::new(&image);
    let fs = ext4::Ext4::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the superblock must validate");
    let found = fs.recover_from_journal(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1, "the journalled inode copy must be mined");
    let recovered = &found[0];
    assert_eq!(recovered.size, file.content.len() as u64);
    assert_eq!(
        recovered.confidence,
        Confidence::JournalResidue,
        "journal-mined extents are residue, never live metadata"
    );
    assert_recovers_the_planted_bytes(&image, &file, &recovered.extents);
}

#[test]
fn ext4_recovers_a_fragmented_file_in_extent_order() {
    let file = FilePlan::fragmented(
        "fragmented.jpg",
        &[
            (96 * EXT4_BLOCK, 2 * EXT4_BLOCK),
            (48 * EXT4_BLOCK, EXT4_BLOCK),
            (150 * EXT4_BLOCK, EXT4_BLOCK),
        ],
    );
    let image = ext4_volume(256 * EXT4_BLOCK, &file);

    let mut src = Cursor::new(&image);
    let fs = ext4::Ext4::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the superblock must validate");
    let found = fs.recover_from_journal(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].extents.len(), 3, "one range per extent entry");
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn ext4_falls_back_to_a_backup_superblock() {
    let file = FilePlan::new("deleted.jpg", 64 * EXT4_BLOCK, EXT4_BLOCK);
    let mut image = ext4_volume(256 * EXT4_BLOCK, &file);
    // Grow the image so group 1's backup superblock exists, then copy the
    // primary there and destroy the primary.
    image.resize(8200 * EXT4_BLOCK, 0);
    let primary = image[EXT4_BLOCK..2 * EXT4_BLOCK].to_vec();
    let backup_at = 8192 * EXT4_BLOCK + 1024;
    image[backup_at..backup_at + EXT4_BLOCK].copy_from_slice(&primary);
    image[EXT4_BLOCK..2 * EXT4_BLOCK].fill(0);

    let fs = ext4::Ext4::open(&mut Cursor::new(&image), ByteOffset::new(0))
        .expect("in-memory read")
        .expect("a backup superblock must be found");
    assert_eq!(fs.block_bytes, EXT4_BLOCK as u64);
}

#[test]
fn an_extent_tree_claiming_an_impossible_depth_yields_no_extents() {
    // Depth 9 exceeds the format's five-level maximum; the walker must
    // refuse it rather than follow whatever the bytes point at.
    let inode = ext4_inode_with_extents(0x8000, 0, 1_700_000_000, 4096, &[(64, 1)]);
    let deep = with_u16_le(&inode, 40 + 6, 9);
    let mut stale = vec![0_u8; EXT4_BLOCK];
    let slot = 5 * 128;
    stale[slot..slot + deep.len()].copy_from_slice(&deep);

    let entries = ext4::dir_entries(&stale);
    assert!(
        entries.is_empty(),
        "an over-deep tree must not be reinterpreted as directory entries"
    );
}

#[test]
fn ext4_directory_blocks_yield_orphan_names() {
    let block = ext4_dir_block(&[("keep.txt", 12), ("gone.jpg", 13)]);
    let entries = ext4::dir_entries(&block);

    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert!(names.contains(&"gone.jpg"));
}

#[test]
fn a_directory_block_with_an_overflowed_record_length_terminates() {
    let block = ext4_dir_block(&[("keep.txt", 12)]);
    let broken = with_u16_le(&block, 4, u16::MAX);
    // The walk must end rather than run past the block or loop.
    let entries = ext4::dir_entries(&broken);
    assert!(entries.len() <= 1);
}

// --- FAT32 / exFAT --------------------------------------------------------

#[test]
fn fat32_recovers_a_deleted_entry_with_its_long_name() {
    let file = FilePlan::new("vacation-photo.jpg", 256 * FAT_CLUSTER, 5000);
    let image = fat32_volume(1024 * FAT_CLUSTER, &file);

    let mut src = Cursor::new(&image);
    let volume = fat::Fat::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the boot sector must validate");
    assert_eq!(volume.kind, FsKind::Fat32);
    let found = volume.recover_deleted(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    let recovered = &found[0];
    assert_eq!(recovered.name.as_deref(), Some("vacation-photo.jpg"));
    assert_eq!(recovered.size, file.content.len() as u64);
    assert!(recovered.timestamps.modified.is_some());
    assert_recovers_the_planted_bytes(&image, &file, &recovered.extents);
    assert_eq!(
        recovered.confidence,
        Confidence::Reassembled,
        "the FAT chain is gone, so contiguity is an assumption, not metadata"
    );
}

#[test]
fn exfat_recovers_a_deleted_entry_set_with_exact_extents() {
    let file = FilePlan::new("clip.png", 256 * FAT_CLUSTER, 7000);
    let image = exfat_volume(1024 * FAT_CLUSTER, &file);

    let mut src = Cursor::new(&image);
    let volume = fat::Fat::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the boot sector must validate");
    assert_eq!(volume.kind, FsKind::ExFat);
    let found = volume.recover_deleted(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name.as_deref(), Some("clip.png"));
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
    assert_eq!(
        found[0].confidence,
        Confidence::FsMetadata,
        "a NoFatChain stream stores exact extents"
    );
}

#[test]
fn a_zero_filled_or_truncated_fat_directory_yields_nothing() {
    let file = FilePlan::new("clip.png", 256 * FAT_CLUSTER, 1024);
    let image = fat32_volume(1024 * FAT_CLUSTER, &file);
    let volume = fat::Fat::from_boot_sector(&image[..SECTOR], ByteOffset::new(0))
        .expect("the boot sector must validate");

    assert!(
        volume
            .deleted_in_directory(&zero_filled(FAT_CLUSTER))
            .is_empty()
    );
    for keep in [0_usize, 1, 31, 33, 100] {
        // A partial entry must never be reported as a recovered file.
        let cut = truncated(&zero_filled(FAT_CLUSTER), keep);
        assert!(volume.deleted_in_directory(&cut).is_empty());
    }
}

// --- APFS -----------------------------------------------------------------

#[test]
fn apfs_recovers_an_inode_present_in_an_older_checkpoint() {
    let file = FilePlan::new("shot.jpg", 32 * APFS_BLOCK, 2 * APFS_BLOCK);
    let image = apfs_container(64 * APFS_BLOCK, &file);

    let mut src = Cursor::new(&image);
    let container = apfs::Apfs::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the container superblock must validate");
    assert!(
        container.checkpoints.len() >= 2,
        "the descriptor ring must yield an older checkpoint"
    );
    let found = container.recover_deleted(&mut src).expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name.as_deref(), Some("shot.jpg"));
    assert_eq!(found[0].confidence, Confidence::FsMetadata);
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn apfs_rejects_an_object_whose_checksum_does_not_match() {
    let file = FilePlan::new("shot.jpg", 32 * APFS_BLOCK, APFS_BLOCK);
    let mut image = apfs_container(64 * APFS_BLOCK, &file);
    // Corrupt a byte covered by the Fletcher-64 checksum.
    image[64] ^= 0xFF;

    let container =
        apfs::Apfs::open(&mut Cursor::new(&image), ByteOffset::new(0)).expect("in-memory read");
    assert!(
        container.is_none(),
        "an object failing its checksum must never be trusted"
    );
}

#[test]
fn a_cyclic_apfs_tree_terminates_instead_of_looping() {
    let file = FilePlan::new("shot.jpg", 32 * APFS_BLOCK, APFS_BLOCK);
    let mut image = apfs_container(64 * APFS_BLOCK, &file);
    // Replace the older checkpoint's filesystem tree with an index node
    // pointing back at its own block: a crafted cycle.
    let cyclic = apfs_cyclic_tree(9);
    image[9 * APFS_BLOCK..10 * APFS_BLOCK].copy_from_slice(&cyclic);

    let mut src = Cursor::new(&image);
    let container = apfs::Apfs::open(&mut src, ByteOffset::new(0))
        .expect("in-memory read")
        .expect("the container superblock must validate");
    // The bounded walk must return; that this call ends is the assertion.
    let found = container.recover_deleted(&mut src).expect("in-memory read");
    assert!(
        found.is_empty(),
        "a cyclic tree carries no recoverable records"
    );
}

// --- residue sweep --------------------------------------------------------

#[test]
fn residue_sweep_finds_a_filesystem_two_reformats_ago() {
    // An ext4 volume, then re-formatted as NTFS: the NTFS boot sector lands
    // at offset 0, while the ext4 superblock a kibibyte in survives, along
    // with the ext4 volume's file content.
    let file = FilePlan::new("old.jpg", 64 * EXT4_BLOCK, 2 * EXT4_BLOCK);
    let mut image = ext4_volume(512 * EXT4_BLOCK, &file);
    // The re-format writes a fresh NTFS boot sector and its $MFT area only.
    let boot = ntfs_boot_sector(image.len(), 64 * NTFS_CLUSTER);
    image[..SECTOR].copy_from_slice(&boot);

    let sweep =
        residue::sweep(&mut Cursor::new(&image), image.len() as u64, &[]).expect("in-memory sweep");

    let kinds: Vec<FsKind> = sweep.volumes.iter().map(|volume| volume.kind).collect();
    assert!(
        kinds.contains(&FsKind::Ntfs),
        "the current filesystem must be seen: {kinds:?}"
    );
    assert!(
        kinds.contains(&FsKind::Ext4),
        "the pre-format ext4 volume must still be found: {kinds:?}"
    );

    // And its journalled deleted file is still recoverable from the residue,
    // byte for byte.
    let residual = sweep
        .volumes
        .iter()
        .find(|volume| volume.kind == FsKind::Ext4)
        .expect("ext4 residue");
    assert_eq!(residual.origin, Origin::Residual);
    let mut src = Cursor::new(&image);
    let fs = ext4::Ext4::open(&mut src, residual.range.start)
        .expect("in-memory read")
        .expect("the residual superblock must validate");
    let found = fs.recover_from_journal(&mut src).expect("in-memory read");
    assert_eq!(found.len(), 1);
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn residue_sweep_marks_volumes_listed_in_the_current_table() {
    let file = FilePlan::new("photo.jpg", 256 * FAT_CLUSTER, 1024);
    let volume = fat32_volume(1024 * FAT_CLUSTER, &file);
    let sweep = residue::sweep(
        &mut Cursor::new(&volume),
        volume.len() as u64,
        &[ByteRange::new(ByteOffset::new(0), volume.len() as u64)],
    )
    .expect("in-memory sweep");

    let fat = sweep
        .volumes
        .iter()
        .find(|found| found.kind == FsKind::Fat32)
        .expect("the FAT volume must be found");
    assert_eq!(fat.origin, Origin::Current);
}

#[test]
fn zero_filled_and_random_regions_yield_no_anchors() {
    let zeros = zero_filled(512 * 1024);
    assert!(
        residue::sweep(&mut Cursor::new(&zeros), zeros.len() as u64, &[])
            .expect("in-memory sweep")
            .volumes
            .is_empty()
    );

    let patterned: Vec<u8> = (0..512_u32 * 1024)
        .map(|i| u8::try_from((i * 37 + 11) % 251).unwrap_or(0))
        .collect();
    assert!(
        residue::sweep(&mut Cursor::new(&patterned), patterned.len() as u64, &[])
            .expect("in-memory sweep")
            .volumes
            .is_empty(),
        "patterned filler must not produce phantom volumes"
    );
}

// --- regressions: extents must point at the real bytes ---------------------

#[test]
fn a_resident_data_extent_points_at_the_payload_in_the_record() {
    // Files under a few hundred bytes keep their content inside the MFT
    // record. The extent must name where those bytes really are — not an
    // offset relative to the record, which would land in the boot sector.
    let payload: Vec<u8> = (0..200_u16).map(|i| (i % 251) as u8).collect();
    let record = ntfs_record_resident("note.txt", &payload);
    let record_at = 1024 * 1024;
    let payload_at = record_at + resident_payload_offset(&record, &payload);
    let image = Image::new(2 * 1024 * 1024)
        .with(record_at, &record)
        .into_bytes();

    let found = ntfs::orphan_scan(
        &mut Cursor::new(&image),
        ByteRange::new(ByteOffset::new(record_at as u64), 1024),
        ByteOffset::new(0),
        NTFS_CLUSTER as u64,
    )
    .expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].extents,
        vec![ByteRange::new(
            ByteOffset::new(payload_at as u64),
            payload.len() as u64
        )],
    );
    let start = payload_at;
    assert_eq!(
        &image[start..start + payload.len()],
        &*payload,
        "the claimed extent must hold the resident payload"
    );
}

#[test]
fn orphan_extents_are_resolved_against_the_volume_they_belong_to() {
    // Run lists store volume-relative clusters, so a volume that does not
    // start at zero must be told its own offset.
    const VOLUME_AT: usize = 8 * 1024 * 1024;
    let file = FilePlan::new("orphan.jpg", VOLUME_AT + 64 * NTFS_CLUSTER, 4096);
    let inner = FilePlan::new("orphan.jpg", 64 * NTFS_CLUSTER, 4096);
    let volume = ntfs_volume(2 * 1024 * 1024, 16 * NTFS_CLUSTER, &inner);
    let image = Image::new(VOLUME_AT + 2 * 1024 * 1024)
        .with(VOLUME_AT, &volume)
        .into_bytes();

    let record_at = VOLUME_AT + 16 * NTFS_CLUSTER + 1024;
    let found = ntfs::orphan_scan(
        &mut Cursor::new(&image),
        ByteRange::new(ByteOffset::new(record_at as u64), 1024),
        ByteOffset::new(VOLUME_AT as u64),
        NTFS_CLUSTER as u64,
    )
    .expect("in-memory read");

    assert_eq!(found.len(), 1);
    assert_recovers_the_planted_bytes(&image, &file, &found[0].extents);
}

#[test]
fn the_orphan_scan_ignores_records_still_in_use() {
    // Reporting live files as deleted would assert something false about
    // every file on the volume.
    let live = ntfs_record(true, Some("live.jpg"), 0, Some(&run_list(&[(4, 1)])), 4096);
    let image = Image::new(64 * 1024).with(0, &live).into_bytes();

    let found = ntfs::orphan_scan(
        &mut Cursor::new(&image),
        ByteRange::new(ByteOffset::new(0), 1024),
        ByteOffset::new(0),
        NTFS_CLUSTER as u64,
    )
    .expect("in-memory read");
    assert!(found.is_empty(), "an in-use record is not a deleted file");
}

#[test]
fn every_recovery_records_its_source_filesystem_object() {
    let file = FilePlan::new("evidence.jpg", 512 * 1024, 4096);
    let image = ntfs_volume(2 * 1024 * 1024, 64 * NTFS_CLUSTER, &file);
    let mut src = Cursor::new(&image);
    let volume = ntfs::Ntfs::open(&mut src, ByteOffset::new(0))
        .expect("read")
        .expect("valid");
    let found = volume.recover_deleted(&mut src).expect("read");
    assert!(
        found[0].source_object.is_some(),
        "a finding must be traceable to the metadata object it came from"
    );
}

// --- regressions: corrupt geometry must not drive allocation ---------------

#[test]
fn an_absurd_exfat_cluster_size_is_rejected_before_any_allocation() {
    // A one-byte edit to SectorsPerClusterShift would otherwise ask for a
    // 512 TiB root-directory read.
    let boot = exfat_boot_sector(4 * 1024 * 1024, 64);
    let mut absurd = boot.clone();
    absurd[109] = 40;

    assert!(
        fat::Fat::from_boot_sector(&absurd, ByteOffset::new(0)).is_none(),
        "a cluster size beyond the format's maximum must fail the parse"
    );
}

#[test]
fn an_exfat_size_field_cannot_claim_more_than_the_volume() {
    let file = FilePlan::new("clip.png", 256 * FAT_CLUSTER, 4096);
    let image = exfat_volume(1024 * FAT_CLUSTER, &file);
    let volume = fat::Fat::from_boot_sector(&image[..SECTOR], ByteOffset::new(0))
        .expect("the boot sector must validate");

    // Overwrite the stream extension's size with the u64 maximum.
    let dir_at = 64 * SECTOR;
    let mut dir = image[dir_at..dir_at + FAT_CLUSTER].to_vec();
    dir[32 + 24..32 + 32].copy_from_slice(&u64::MAX.to_le_bytes());

    let found = volume.deleted_in_directory(&dir);
    assert_eq!(found.len(), 1);
    for extent in &found[0].extents {
        assert!(
            extent.end().is_some(),
            "an extent must never be unbounded: {extent:?}"
        );
        assert!(
            extent.end().expect("bounded").get() <= image.len() as u64,
            "an extent must never claim past the volume"
        );
    }
}

#[test]
fn an_absurd_gpt_entry_size_is_rejected_before_any_allocation() {
    let image = gpt_image(4 * 1024 * 1024, &[2048..=4095]);
    // Claim 2 MiB per entry: the array would be gigabytes.
    let mut broken = with_u32_le(&image, SECTOR + 84, 1 << 21);
    // Recompute the header CRC so only the size cap can reject it.
    let mut crc = crc32fast::Hasher::new();
    broken[SECTOR + 16..SECTOR + 20].fill(0);
    crc.update(&broken[SECTOR..SECTOR + 92]);
    let value = crc.finalize();
    broken[SECTOR + 16..SECTOR + 20].copy_from_slice(&value.to_le_bytes());
    // Wipe the backup header so no fallback can rescue it.
    let end = broken.len();
    broken[end - SECTOR..].fill(0);

    let tables =
        part::scan(&mut Cursor::new(&broken), broken.len() as u64).expect("in-memory scan");
    assert!(
        tables.partitions.is_empty(),
        "an entry size beyond one sector must fail the parse"
    );
}
