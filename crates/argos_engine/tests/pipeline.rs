//! End-to-end scans over synthetic media.
//!
//! Every test asserts on the bytes the sink received, not on counts alone: a
//! pipeline that reports the right number of artifacts with the wrong content
//! is worse than one that reports nothing.

use std::io::Cursor;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use argos_core::progress::{Discard, ProgressSink, RunState, ScanEvent, Unit};
use argos_core::{Confidence, Format, Stage};
use argos_engine::fixture::{Collected, Collector, Events};
use argos_engine::{Medium, ScanConfig, ScanReport, ScanSession, Stages};

/// Chunk size used throughout: the configured minimum, so a few hundred
/// kilobytes of fixture still exercises multi-chunk reading and the overlap.
const CHUNK: usize = argos_engine::config::MIN_CHUNK_BYTES;

fn views(image: &[u8], count: usize) -> Vec<Cursor<Vec<u8>>> {
    (0..count).map(|_| Cursor::new(image.to_vec())).collect()
}

fn config(workers: usize) -> ScanConfig {
    ScanConfig::builder()
        .workers(NonZeroUsize::new(workers).expect("at least one worker"))
        .chunk_bytes(CHUNK)
        // The generated fixtures are a few dozen pixels across; these tests
        // are about the pipeline, not about which sizes reach a directory.
        .min_long_side(0)
        .build()
        .expect("valid configuration")
}

/// Runs a full scan and returns what the sink collected alongside the report.
fn scan_with(image: &[u8], config: ScanConfig) -> (Vec<Collected>, ScanReport) {
    let workers = config.workers().get();
    let session = ScanSession::new(config);
    let medium = Medium::new(views(image, workers), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");
    (sink.artifacts().to_vec(), report)
}

fn scan(image: &[u8]) -> (Vec<Collected>, ScanReport) {
    scan_with(image, config(4))
}

/// A disk with `count` distinct JPEGs and one PNG, spread far enough apart to
/// straddle several chunk boundaries.
fn disk_with_images(count: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut disk = argos_carve::fixture::Disk::filled(CHUNK * 4);
    let mut planted = Vec::new();
    for index in 0..count {
        let image = argos_carve::fixture::Jpeg::new()
            .with_entropy_bytes(64 + index * 7)
            .build();
        disk = disk.with(CHUNK / 2 + index * CHUNK, &image);
        planted.push(image);
    }
    let png = argos_carve::fixture::png(9, 4);
    disk = disk.with(CHUNK * 3 + 1024, &png);
    planted.push(png);
    (disk.into_bytes(), planted)
}

#[test]
fn every_planted_image_comes_back_byte_identical() {
    let (image, planted) = disk_with_images(3);

    let (artifacts, report) = scan(&image);

    assert_eq!(artifacts.len(), planted.len(), "one artifact per image");
    for expected in &planted {
        assert!(
            artifacts.iter().any(|got| &got.bytes == expected),
            "a planted image did not come back byte-identical"
        );
    }
    assert_eq!(report.state, RunState::Finished);
    assert!(report.unreadable.is_empty());
    assert_eq!(report.artifacts, planted.len() as u64);
}

#[test]
fn a_carved_artifact_is_reported_at_the_carving_tier_with_its_extents() {
    let mut disk = argos_carve::fixture::Disk::filled(CHUNK * 2);
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let at = CHUNK + 4096;
    disk = disk.with(at, &jpeg);

    let (artifacts, _) = scan(&disk.into_bytes());

    let [artifact] = artifacts.as_slice() else {
        panic!("expected exactly one artifact, got {}", artifacts.len());
    };
    assert_eq!(artifact.format, Format::Jpeg);
    assert_eq!(artifact.stage, Stage::Carve);
    // Carving proves contiguity and nothing more: never a metadata tier.
    assert_eq!(artifact.confidence, Confidence::ContiguousCarve);
    assert_eq!(artifact.extents.len(), 1);
    assert_eq!(artifact.extents[0].start.get(), at as u64);
    assert_eq!(artifact.extents[0].len, jpeg.len() as u64);
    assert_eq!(artifact.bytes, jpeg);
}

#[test]
fn the_result_does_not_depend_on_how_many_workers_ran() {
    let (image, _) = disk_with_images(4);

    let (one, report_one) = scan_with(&image, config(1));
    let (many, report_many) = scan_with(&image, config(8));

    assert_eq!(one, many, "the manifest must not depend on worker count");
    assert_eq!(report_one.artifacts, report_many.artifacts);
    assert_eq!(
        report_one.rejected_candidates,
        report_many.rejected_candidates
    );
}

#[test]
fn the_search_finds_the_same_images_however_many_threads_ran_it() {
    // A region's headers are searched across threads, and one recovery changes
    // which bytes the next header may be offered. What a thread happened to
    // finish first must not decide that: the region's outcome is settled in
    // header order, so a run on twelve cores reports exactly what a run on one
    // reports. The images above are whole, and cover none of this.
    let block = argos_carve::classify::BLOCK_BYTES;
    let mut disk = argos_carve::fixture::Disk::noisy(96 * block, 0x51ED_2A11_0000_0001);
    for nth in 0..4_usize {
        let photo = argos_carve::fixture::photo_jpeg(320, 240, 0xC0FF_EE00 ^ nth as u64);
        let at = (8 + nth * 20) * block;
        disk = disk.with(at, &photo[..block]);
        // Two of the four have a remainder to find; the others spend the whole
        // search and recover nothing, which is the medium's usual answer.
        if nth % 2 == 1 {
            disk = disk.with(at + 6 * block, &photo[block..]);
        }
    }
    let image = disk.into_bytes();

    let (one, report_one) = scan_with(&image, config(1));
    let (many, report_many) = scan_with(&image, config(8));

    assert!(
        report_one.reassembled > 0,
        "the fixture must reassemble something for this to mean anything"
    );
    assert_eq!(
        one, many,
        "the same medium searched on more threads must yield the same artifacts"
    );
    assert_eq!(report_one.reassembled, report_many.reassembled);
    assert_eq!(
        report_one.reassembly_attempted,
        report_many.reassembly_attempted
    );
}

#[test]
fn two_runs_over_the_same_medium_produce_the_same_manifest() {
    let (image, _) = disk_with_images(3);

    let (first, _) = scan(&image);
    let (second, _) = scan(&image);

    assert_eq!(first, second);
}

#[test]
fn an_image_stored_twice_is_reported_once_with_the_earlier_extent() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let image = argos_carve::fixture::Disk::filled(CHUNK * 3)
        .with(4096, &jpeg)
        .with(CHUNK * 2, &jpeg)
        .into_bytes();

    let (artifacts, report) = scan(&image);

    let [artifact] = artifacts.as_slice() else {
        panic!("identical content must collapse to one artifact");
    };
    assert_eq!(artifact.extents[0].start.get(), 4096);
    assert_eq!(report.duplicates, 1);
    assert_eq!(artifact.bytes, jpeg);
}

#[test]
fn a_deleted_file_recovered_from_ntfs_keeps_its_name_and_beats_carving() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let file = argos_fs::fixture::FilePlan::new("holiday.jpg", 128 * 1024, jpeg.len())
        .with_content(jpeg.clone());
    let image = argos_fs::fixture::ntfs_volume(CHUNK * 6, 32 * 1024, &file);

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.recovered_name.is_some())
        .expect("the deleted file must come back with its name");
    assert_eq!(recovered.recovered_name.as_deref(), Some("holiday.jpg"));
    assert_eq!(recovered.stage, Stage::Filesystem);
    assert_eq!(recovered.confidence, Confidence::FsMetadata);
    assert!(
        recovered.source_object.is_some(),
        "a finding must be traceable back to the filesystem object it came from"
    );
    assert_eq!(recovered.bytes, jpeg);
    // The carver finds the same bytes; merging keeps the stronger evidence.
    assert_eq!(
        artifacts.len(),
        1,
        "one file recovered twice is still one artifact"
    );
}

#[test]
fn a_deleted_file_recovered_from_btrfs_keeps_its_name_and_beats_carving() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let file = argos_fs::fixture::FilePlan::new(
        "holiday.jpg",
        argos_fs::fixture::BTRFS_DATA_AT,
        jpeg.len(),
    )
    .with_content(jpeg.clone());
    let image = argos_fs::fixture::btrfs_volume(CHUNK * 12, &file);

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.recovered_name.is_some())
        .expect("the deleted file must come back with its name");
    assert_eq!(recovered.recovered_name.as_deref(), Some("holiday.jpg"));
    assert_eq!(recovered.stage, Stage::Filesystem);
    assert_eq!(recovered.confidence, Confidence::FsMetadata);
    assert!(
        recovered.source_object.is_some(),
        "a finding must be traceable back to the filesystem object it came from"
    );
    assert_eq!(recovered.bytes, jpeg);
    assert_eq!(
        artifacts.len(),
        1,
        "one file recovered twice is still one artifact"
    );
}

#[test]
fn a_truncated_png_is_recorded_as_a_break_and_never_written_as_a_header() {
    // PNG verifies per chunk, so a file whose tail is gone has a truncated
    // `IDAT` that cannot verify and a confirmed prefix that stops at the
    // `IHDR` — thirty-three bytes describing a picture, with none of it. A
    // JPEG in the same state has a proven share of its frame, because its
    // entropy decoder confirms MCU by MCU.
    //
    // So the two are not symmetric, and this asserts the honest half: the
    // break is recorded, the header is not written as a recovery. Reporting
    // one would be reporting a description as a photograph.
    // Noise rather than a gradient: a compressible fixture makes a PNG smaller
    // than the prefix threshold, which would prove nothing.
    let (width, height) = (96_u32, 96_u32);
    let mut raw = Vec::new();
    let mut seed = 0x2E7F_1B93_u32;
    for _ in 0..height {
        // Filter byte, then a row of noise.
        raw.extend_from_slice(&[0]);
        for _ in 0..width * 3 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            raw.push((seed >> 24) as u8);
        }
    }
    let png = argos_carve::fixture::png_from_raw(width, height, &raw);
    assert!(
        png.len() > 2048,
        "the fixture must be big enough that half of it is a meaningful prefix"
    );
    let at = 8192;
    let image = argos_carve::fixture::Disk::filled(CHUNK * 3)
        .with(at, &png[..png.len() / 2])
        .into_bytes();

    let (artifacts, report) = scan(&image);

    assert_eq!(
        report.partial_prefixes, 0,
        "a PNG header is not a recovery: {report:?}"
    );
    assert!(
        artifacts.is_empty(),
        "nothing verified past the header, so nothing may be written"
    );

    // But the break is recorded with what the frame declares, so a later run
    // can search it and a reader can see it was there at all.
    let [point] = report.fragmentation.as_slice() else {
        panic!("the break must be recorded: {report:?}");
    };
    assert_eq!(point.header.get(), at as u64);
    assert_eq!(point.format, Format::Png);
    assert_eq!(
        point.declared,
        Some((width, height)),
        "and it must state the size it claims, so a floor can act on it"
    );
}

#[test]
fn a_volume_whose_first_sector_was_overwritten_is_still_found_by_its_copy() {
    // The shape a re-formatted disk actually has. A later format writes its own
    // structures over the start of the volume that was there, so the NTFS boot
    // sector is gone — but the copy in the volume's last sector, thousands of
    // clusters away, is not, and neither is the `$MFT`.
    //
    // Read as a volume start, that copy points a volume's length past itself:
    // the `$MFT` lands outside the medium, every extent resolves to unrelated
    // bytes, and the deleted file the metadata still describes is reported by
    // nobody. Confirming the anchor against the `$MFT` is what turns the copy
    // back into the volume it belongs to.
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let file = argos_fs::fixture::FilePlan::new("childhood.jpg", 128 * 1024, jpeg.len())
        .with_content(jpeg.clone());
    let volume = argos_fs::fixture::ntfs_volume(CHUNK * 6, 32 * 1024, &file);

    // Wipe the primary boot sector, leaving the copy at the last sector.
    let mut image = volume;
    let sector = argos_fs::fixture::SECTOR;
    image[..sector].fill(0);
    assert_ne!(
        image[image.len() - sector..],
        [0; 512],
        "the copy is the only anchor left, so it has to still be there"
    );

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.recovered_name.is_some())
        .expect("the copy must lead back to the volume and its deleted file");
    assert_eq!(recovered.recovered_name.as_deref(), Some("childhood.jpg"));
    assert_eq!(recovered.stage, Stage::Filesystem);
    assert_eq!(recovered.confidence, Confidence::FsMetadata);
    assert_eq!(recovered.bytes, jpeg);
}

#[test]
fn a_fragmented_deleted_file_is_reassembled_from_its_extents_in_order() {
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(8192)
        .build();
    // Runs are cluster-granular and deliberately out of physical order: a
    // recovery that concatenated by address would fail the byte comparison.
    let cluster = argos_fs::fixture::NTFS_CLUSTER;
    let file = argos_fs::fixture::FilePlan::fragmented(
        "split.jpg",
        &[
            (64 * cluster, cluster),
            (32 * cluster, jpeg.len() - cluster),
        ],
    )
    .with_content(jpeg.clone());
    let image = argos_fs::fixture::ntfs_volume(CHUNK * 8, 4 * cluster, &file);

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.recovered_name.is_some())
        .expect("the fragmented file must be recovered from metadata");
    assert_eq!(recovered.extents.len(), 2, "both fragments are reported");
    assert_eq!(
        recovered.bytes, jpeg,
        "the extents must concatenate to the planted bytes, in file order"
    );
}

#[test]
fn a_deleted_file_whose_volume_is_gone_is_still_named_and_dated() {
    // The state a re-format leaves: the `FILE` record survives on the surface,
    // the boot sector that said where the volume began does not. Its content
    // cannot be placed — a run list counts clusters of a volume nobody can
    // find — but the record still says which file it was, how large, and when.
    // Losing that is losing the only evidence the file ever existed.
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(4096)
        .build();
    let file =
        argos_fs::fixture::FilePlan::new("holiday.jpg", 128 * 1024, jpeg.len()).with_content(jpeg);
    let mft_at = 32 * 1024;
    let mut image = argos_fs::fixture::ntfs_volume(CHUNK * 6, mft_at, &file);
    // Neither boot sector: NTFS keeps one at the volume's first sector and a
    // copy at its last, and a re-format that leaves either behind leaves a
    // volume the sweep can still resolve. Both gone is the case with nothing
    // left to resolve against — and the case the record has to survive.
    let sector = argos_fs::fixture::SECTOR;
    image[..sector].fill(0);
    let end = image.len() - sector;
    image[end..].fill(0);
    let record_at = mft_at + 1024;

    let (_, report) = scan(&image);

    assert!(
        report.unattributed_residue > 0,
        "the record sits in a region no volume covers"
    );
    let named = report
        .lost_files
        .iter()
        .find(|lost| lost.name.as_deref() == Some("holiday.jpg"))
        .expect("a record no volume covers must still name its file");
    assert_eq!(named.size, file.content.len() as u64);
    assert!(
        !named.timestamps.is_empty(),
        "the record carries the times the file was made and last written"
    );
    assert_eq!(
        named.record_at, record_at as u64,
        "where the record lay is where the lost $MFT lay"
    );
}

#[test]
fn a_file_from_the_filesystem_before_the_last_format_still_comes_back() {
    // An ext4 volume re-formatted as NTFS: the new boot sector lands at
    // offset 0, while the ext4 superblock a kibibyte in, its journal and the
    // file content all survive elsewhere on the surface.
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(2048)
        .build();
    let block = argos_fs::fixture::EXT4_BLOCK;
    let file = argos_fs::fixture::FilePlan::new("older.jpg", 64 * block, jpeg.len())
        .with_content(jpeg.clone());
    let mut image = argos_fs::fixture::ext4_volume(512 * block, &file);
    let boot =
        argos_fs::fixture::ntfs_boot_sector(image.len(), 64 * argos_fs::fixture::NTFS_CLUSTER);
    image[..argos_fs::fixture::SECTOR].copy_from_slice(&boot);

    let (artifacts, report) = scan(&image);

    // An ext4 inode carries no name — names live in directory entries — so
    // what the journal gives back is the extent tree, at the journal tier.
    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.confidence == Confidence::JournalResidue)
        .expect("the pre-format file must be recovered through the residue sweep");
    assert_eq!(recovered.stage, Stage::Filesystem);
    assert!(
        recovered.source_object.is_some(),
        "a journal-recovered finding must name the inode it came from"
    );
    assert_eq!(recovered.bytes, jpeg);
    assert!(
        report
            .volumes
            .iter()
            .any(|volume| volume.origin == argos_fs::Origin::Residual),
        "the pre-format volume must be reported as residue"
    );
}

#[test]
fn metadata_pointing_at_bytes_that_are_not_an_image_yields_nothing() {
    // Extents that survived a format can point anywhere. Without a signature
    // at the start, there is no image to claim — at any tier.
    let file = argos_fs::fixture::FilePlan::new("stale.jpg", 128 * 1024, 4096)
        .with_content(vec![0x5A; 4096]);
    let image = argos_fs::fixture::ntfs_volume(CHUNK * 6, 32 * 1024, &file);

    let (artifacts, report) = scan(&image);

    assert!(
        artifacts.is_empty(),
        "stale metadata must not become a top-tier finding"
    );
    assert_eq!(report.state, RunState::Finished);
}

#[test]
fn a_corrupt_candidate_is_counted_never_reported() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let truncated = argos_carve::fixture::truncated(&jpeg, jpeg.len() / 2);
    let image = argos_carve::fixture::Disk::filled(CHUNK * 2)
        .with(4096, &truncated)
        .into_bytes();

    let (artifacts, report) = scan(&image);

    assert!(artifacts.is_empty());
    assert_eq!(report.rejected_candidates, 1);
}

/// Broken candidates planted for the budget test.
///
/// Enough of them that a stage which charges every failure its whole
/// per-candidate ceiling runs out partway, which is what a used disk does:
/// its fragmentation points number in the thousands.
const BROKEN_CANDIDATES: usize = 200;

const MIB: u64 = 1024 * 1024;

#[test]
fn a_photograph_whose_tail_is_gone_comes_back_as_the_part_that_decodes() {
    // A photograph overwritten partway through cannot be reassembled: the
    // bytes are not on the medium to be found. What is there is its beginning,
    // and the decoder can say exactly how much of it decodes. Before, such a
    // candidate produced no file at all.
    //
    // Beside it, a thumbnail-cache entry truncated the same way. It must
    // produce nothing: the search floor is what keeps a used disk's caches out
    // of the output, and it has to hold for partials as it does for whole
    // files (docs/defects/02-thumbnail-provenance.md).
    let photo = argos_carve::fixture::photo_jpeg(1600, 1200, 0x9E11_0000_0000_0007);
    let cache = argos_carve::fixture::photo_jpeg(258, 258, 0x9E11_0000_0000_0009);
    let disk = argos_carve::fixture::Disk::noisy(3 * 1024 * 1024, 0x0FF1_CE00_0000_0001)
        .with(
            64 * 1024,
            &argos_carve::fixture::truncated(&photo, photo.len() * 6 / 10),
        )
        .with(
            2 * 1024 * 1024,
            &argos_carve::fixture::truncated(&cache, cache.len() * 6 / 10),
        )
        .into_bytes();

    let config = ScanConfig::builder()
        .workers(NonZeroUsize::new(2).expect("at least one worker"))
        .chunk_bytes(CHUNK)
        .min_long_side(argos_engine::DEFAULT_MIN_LONG_SIDE)
        .build()
        .expect("valid configuration");
    let (artifacts, report) = scan_with(&disk, config);

    let partials: Vec<_> = artifacts
        .iter()
        .filter(|artifact| artifact.confidence == Confidence::PartialOrThumbnail)
        .collect();
    let [partial] = partials.as_slice() else {
        panic!("expected exactly the photograph's prefix, got {partials:?}");
    };
    assert_eq!(partial.extents[0].start.get(), 64 * 1024);
    // The decoder cannot see where the planted bytes ended, so it may walk a
    // little way into whatever followed before a Huffman code stops matching.
    // Those bytes are on the medium and did decode, so reporting them is
    // honest; what must hold is that everything the photograph did cover comes
    // back exactly, and nothing is padded or invented.
    let planted = photo.len() * 6 / 10;
    let overlap = partial.bytes.len().min(planted);
    assert_eq!(
        &partial.bytes[..overlap],
        &photo[..overlap],
        "the prefix must be the photograph's own bytes"
    );
    assert!(
        partial.bytes.len() >= planted * 9 / 10,
        "the decoder should reach nearly all of what survived: {} of {planted}",
        partial.bytes.len()
    );
    assert!(
        partial.bytes.len() <= planted + 64 * 1024,
        "and must not run far past it: {} of {planted}",
        partial.bytes.len()
    );
    assert_eq!(report.partial_prefixes, 1);
    assert_eq!(
        report.reassembly_skipped_small, 1,
        "the cache entry declares 258 pixels and is left to the floor"
    );
}

#[test]
fn a_fragmented_photograph_far_into_a_large_medium_is_reassembled() {
    // The condition the region search exists for, and the one no fixture of a
    // few hundred kilobytes can pose: a header two hundred megabytes in, whose
    // continuation is near it and whose candidate blocks are nowhere near the
    // start of the medium. The medium is generated per byte read rather than
    // held, so the distances are a disk's.
    let photo = argos_carve::fixture::photo_jpeg(320, 240, 0xC0FF_EE00_0000_0001);
    let block = argos_carve::classify::BLOCK_BYTES as u64;
    let header = 200 * MIB;
    let layout =
        argos_carve::fixture::planted(260 * MIB, &photo, &[header, header + 6 * MIB], block);

    let workers = 4;
    let session = ScanSession::new(config(workers));
    let views: Vec<_> = (0..workers).map(|_| layout.source()).collect();
    let medium = Medium::new(views, layout.disk.len()).expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");

    let recovered = sink
        .artifacts()
        .iter()
        .find(|artifact| artifact.confidence == Confidence::Reassembled)
        .expect("the fragmented photograph must be reassembled");
    assert_eq!(
        recovered.bytes, photo,
        "a reassembly must be the planted bytes, not merely something that decoded"
    );
    assert_eq!(
        recovered.extents.len(),
        2,
        "both fragments are reported: {:?}",
        recovered.extents
    );
    assert_eq!(report.reassembled, 1);
}

#[test]
fn every_fragmentation_point_is_offered_to_reassembly() {
    // None of these can be reassembled — their remainders are not on the
    // medium — so each costs the search one failed attempt and nothing else.
    // A stage that stops before the last one has spent its budget on the
    // accounting rather than on the medium, and the candidates it never
    // reached are the ones a real disk holds furthest in.
    // Real encoded photographs, so the entropy decoder actually walks a scan
    // and reports a fragmentation point; a distinct one each, so a search
    // cannot complete one candidate out of another's bytes.
    let stride = 16384;
    let mut disk =
        argos_carve::fixture::Disk::noisy(stride * (BROKEN_CANDIDATES + 2), 0x0BEE_F00D_0000_0001);
    for index in 0..BROKEN_CANDIDATES {
        let photo =
            argos_carve::fixture::photo_jpeg(160, 120, 0x51ED_0000_0000_0001 + index as u64);
        let truncated = argos_carve::fixture::truncated(&photo, photo.len() / 2);
        disk = disk.with(stride * (index + 1), &truncated);
    }

    let (_, report) = scan(&disk.into_bytes());

    assert_eq!(
        report.reassembly_attempted, BROKEN_CANDIDATES as u64,
        "every fragmentation point the sweep found must reach the search"
    );
    assert_eq!(
        report.reassembled, 0,
        "no remainder was planted, so reassembling one would be inventing it"
    );
}

/// A view that fails every read overlapping a chosen range, the way a medium
/// with a bad sector does.
struct Damaged {
    inner: Cursor<Vec<u8>>,
    bad: std::ops::Range<u64>,
}

impl std::io::Read for Damaged {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let at = self.inner.position();
        let end = at.saturating_add(buf.len() as u64);
        if at < self.bad.end && self.bad.start < end {
            return Err(std::io::Error::other("medium reports bad sector"));
        }
        std::io::Read::read(&mut self.inner, buf)
    }
}

impl std::io::Seek for Damaged {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.inner, pos)
    }
}

#[test]
fn an_unreadable_region_is_reported_and_never_recovered_from() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let readable_at = 4096;
    let damaged_at = CHUNK + 8192;
    let image = argos_carve::fixture::Disk::filled(CHUNK * 3)
        .with(readable_at, &jpeg)
        .with(damaged_at, &jpeg)
        .into_bytes();
    let bad = damaged_at as u64..(damaged_at + jpeg.len()) as u64;

    let session = ScanSession::new(config(2));
    let medium = Medium::new(
        (0..2)
            .map(|_| Damaged {
                inner: Cursor::new(image.clone()),
                bad: bad.clone(),
            })
            .collect(),
        image.len() as u64,
    )
    .expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");

    assert!(
        !report.unreadable.is_empty(),
        "damage must be reported, not silently skipped"
    );
    // The intact copy is still recovered; nothing is reported from the damage.
    let artifacts = sink.artifacts();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].extents[0].start.get(), readable_at as u64);
    assert!(
        !report.is_complete(),
        "a damaged scan is not a complete one"
    );
}

#[test]
fn a_recovery_lost_to_neighbouring_damage_is_counted_rather_than_vanishing() {
    // Damage is recorded at retry-span granularity, so a bad sector condemns
    // the whole span around it — including images that were individually
    // readable and did validate. Dropping them is right: their bytes overlap a
    // range whose content is unknown, and reporting one would report zeroes as
    // evidence. Doing it silently is not, because the run then reads as a
    // medium that held nothing rather than as one where damage cost a photo.
    const SPAN: usize = 64 * 1024;
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    // Straddling the span boundary: the header lands in the span that reads,
    // so the sweep still finds it, and the tail lands in the span that does
    // not, so the finding overlaps the damage.
    let straddling = SPAN - jpeg.len() / 2;
    // Far enough past the image that validating it never reads these bytes:
    // the finding must be lost to the *recorded range*, not to a failed read,
    // which is already counted as `unrecoverable`.
    let bad = (SPAN + 55 * 1024) as u64..(SPAN + 55 * 1024 + 512) as u64;
    assert!(
        ((straddling + jpeg.len()) as u64) < bad.start,
        "the fixture must not put the image itself inside the bad sector"
    );

    let image = argos_carve::fixture::Disk::filled(SPAN * 4)
        .with(straddling, &jpeg)
        .into_bytes();
    let config = ScanConfig::builder()
        .workers(NonZeroUsize::new(1).expect("at least one worker"))
        // Four retry spans to a chunk, so the damage costs a span rather than
        // the whole read and the span before it still yields its signature.
        .chunk_bytes(SPAN * 4)
        .min_long_side(0)
        .build()
        .expect("valid configuration");

    let session = ScanSession::new(config);
    let medium = Medium::new(
        vec![Damaged {
            inner: Cursor::new(image.clone()),
            bad,
        }],
        image.len() as u64,
    )
    .expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");

    assert_eq!(
        report.dropped_unreadable, 1,
        "an image that ran into damage must be counted: {report:?}"
    );

    // The tail is bytes the medium never gave and is not reported. The head
    // is: those extents read cleanly and are, byte for byte, the start of the
    // picture — losing them to a bad sector a kilobyte later is losing a
    // photograph to the granularity of a retry.
    let [head] = sink.artifacts() else {
        panic!(
            "the part before the damage must come back, got {}",
            sink.artifacts().len()
        );
    };
    assert_eq!(head.confidence, Confidence::PartialOrThumbnail);
    assert_eq!(head.extents.len(), 1);
    assert_eq!(head.extents[0].start.get(), straddling as u64);
    assert_eq!(
        head.extents[0].end_saturating().get(),
        SPAN as u64,
        "it must stop exactly where the damage starts"
    );
    assert_eq!(
        head.bytes,
        image[straddling..SPAN],
        "and hold the medium's own bytes, with nothing padded"
    );
    assert!(
        head.expected_length
            .is_some_and(|whole| whole > head.bytes.len() as u64),
        "the shortfall must be stated rather than the head presented as whole"
    );
}

/// A progress sink that cancels the run the first time the sweep reports.
struct CancelOnProgress {
    session: ScanSession,
    seen: Arc<AtomicU64>,
}

impl ProgressSink for CancelOnProgress {
    fn emit(&self, event: ScanEvent) {
        if let ScanEvent::StageProgress { .. } = event {
            self.seen.fetch_add(1, Ordering::Relaxed);
            self.session.cancel();
        }
    }
}

#[test]
fn cancelling_stops_the_scan_and_keeps_what_was_already_found() {
    let (image, _) = disk_with_images(2);
    let session = ScanSession::new(config(2));

    // Cancel as soon as the sweep reports its first chunk of progress.
    let seen_progress = Arc::new(AtomicU64::new(0));
    let progress = CancelOnProgress {
        session: session.clone(),
        seen: Arc::clone(&seen_progress),
    };

    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &progress).expect("scan");

    assert_eq!(report.state, RunState::Cancelled);
    assert_eq!(session.state(), RunState::Cancelled);
    assert!(seen_progress.load(Ordering::Relaxed) >= 1);
    // Cancellation takes effect within one chunk of the request.
    assert!(
        report.bytes_swept < image.len() as u64,
        "cancelling must stop the sweep, not merely mark it"
    );
}

/// A progress sink that cancels the run once the writing stage has started.
struct CancelOnStage {
    session: ScanSession,
    stage: Stage,
}

impl ProgressSink for CancelOnStage {
    fn emit(&self, event: ScanEvent) {
        if let ScanEvent::StageStarted { stage, .. } = event
            && stage == self.stage
        {
            self.session.cancel();
        }
    }
}

#[test]
fn cancelling_stops_the_stage_that_writes_artifacts() {
    // The stage a run spends its time in on a real disk, and the one where a
    // cancel button that does nothing is most visible: reading each finding
    // back and handing it to the sink takes as long as there are findings.
    // Cancellation is read between two artifacts, so what was written is whole
    // and the manifest still describes it.
    let mut disk = argos_carve::fixture::Disk::filled(CHUNK * 6);
    for index in 0..24 {
        let jpeg = argos_carve::fixture::Jpeg::new()
            .with_entropy_bytes(512 + index * 3)
            .build();
        disk = disk.with(4096 + index * 8192, &jpeg);
    }
    let image = disk.into_bytes();

    let session = ScanSession::new(config(2));
    let progress = CancelOnStage {
        session: session.clone(),
        stage: Stage::Report,
    };
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();

    let report = session.start(medium, &mut sink, &progress).expect("scan");

    assert_eq!(report.state, RunState::Cancelled);
    assert_eq!(
        sink.artifacts().len(),
        0,
        "cancelling before the first artifact must stop the stage, not run it to the end"
    );

    // And the same medium, uncancelled, has plenty for it to have written —
    // otherwise the assertion above would hold for the wrong reason.
    let (all, _) = scan(&image);
    assert!(
        all.len() > 8,
        "the fixture recovers {} artifacts",
        all.len()
    );
}

/// A progress sink that cancels the run as a given stage finishes.
struct CancelWhenStageEnds {
    session: ScanSession,
    stage: Stage,
}

impl ProgressSink for CancelWhenStageEnds {
    fn emit(&self, event: ScanEvent) {
        if let ScanEvent::StageFinished { stage, .. } = event
            && stage == self.stage
        {
            self.session.cancel();
        }
    }
}

#[test]
fn cancelling_the_search_still_writes_what_the_search_found() {
    // Cancel means "stop searching and write what you have". What the earlier
    // stages established is what the run has to show for itself — on a large
    // medium, hours of reading — and a stop aimed at the search must not take
    // it along. Stopping the writing as well is a second request.
    let mut disk = argos_carve::fixture::Disk::filled(CHUNK * 6);
    for index in 0..24 {
        let jpeg = argos_carve::fixture::Jpeg::new()
            .with_entropy_bytes(512 + index * 3)
            .build();
        disk = disk.with(4096 + index * 8192, &jpeg);
    }
    let image = disk.into_bytes();

    let session = ScanSession::new(config(2));
    // As validation ends: every carved finding exists and none has been
    // written yet, which is where a cancel during a long search lands.
    let progress = CancelWhenStageEnds {
        session: session.clone(),
        stage: Stage::Validation,
    };
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();

    let report = session.start(medium, &mut sink, &progress).expect("scan");

    assert_eq!(report.state, RunState::Cancelled);
    let (all, _) = scan(&image);
    assert!(
        all.len() > 8,
        "the fixture recovers {} artifacts",
        all.len()
    );
    assert_eq!(
        sink.artifacts().len(),
        all.len(),
        "a cancelled search must write the findings it had, not discard them"
    );
}

#[test]
fn pausing_suspends_the_run_until_it_is_resumed() {
    let (image, _) = disk_with_images(2);
    let session = ScanSession::new(config(2));
    session.pause();
    assert_eq!(session.state(), RunState::Paused);

    let resumer = session.clone();
    let waiter = std::thread::spawn(move || {
        // The run cannot progress while paused; resume it from outside.
        std::thread::sleep(std::time::Duration::from_millis(50));
        resumer.resume();
    });

    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");
    waiter.join().expect("resumer thread");

    assert_eq!(report.state, RunState::Finished);
    assert_eq!(report.bytes_swept, image.len() as u64);
}

#[test]
fn a_sink_that_refuses_an_artifact_stops_the_scan() {
    let (image, _) = disk_with_images(3);
    let session = ScanSession::new(config(2));
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::failing_at(1);

    let err = session
        .start(medium, &mut sink, &Discard)
        .expect_err("a sink failure must not be swallowed");

    assert!(err.is_sink());
    assert_eq!(
        sink.artifacts().len(),
        1,
        "results before the failure stand"
    );
}

#[test]
fn progress_events_bracket_every_stage_and_carry_no_content() {
    let (image, _) = disk_with_images(1);
    let session = ScanSession::new(config(2));
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let events = Events::new();

    session.start(medium, &mut sink, &events).expect("scan");
    let seen = events.seen();

    assert!(seen.contains(&ScanEvent::StateChanged {
        state: RunState::Running
    }));
    assert!(seen.contains(&ScanEvent::StateChanged {
        state: RunState::Finished
    }));
    for stage in [
        Stage::Carve,
        Stage::Filesystem,
        Stage::Validation,
        Stage::Report,
    ] {
        assert!(
            seen.iter().any(
                |event| matches!(event, ScanEvent::StageStarted { stage: s, .. } if *s == stage)
            ),
            "stage {stage} never announced itself"
        );
    }
    // Progress is batched per chunk, never per sector: a 256 KiB medium in
    // 64 KiB chunks cannot produce hundreds of events.
    let progress = seen
        .iter()
        .filter(|event| matches!(event, ScanEvent::StageProgress { .. }))
        .count();
    assert!(
        (1..=16).contains(&progress),
        "unbatched progress: {progress} events"
    );
}

#[test]
fn every_stage_that_can_run_long_reports_progress_while_it_runs() {
    // A display can only show what the pipeline says. Validation drives every
    // signature hit through a state machine and the report stage reads each
    // finding back: on a real medium either can run for minutes, and a stage
    // that reports nothing for minutes is indistinguishable from a stalled
    // one. The sweep having reached its total is not an answer, because by
    // then it is over.
    let (image, _) = disk_with_images(3);
    let session = ScanSession::new(config(2));
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let events = Events::new();

    session.start(medium, &mut sink, &events).expect("scan");
    let seen = events.seen();

    for stage in [Stage::Carve, Stage::Validation, Stage::Report] {
        assert!(
            seen.iter().any(
                |event| matches!(event, ScanEvent::StageProgress { stage: s, .. } if *s == stage)
            ),
            "stage {stage} ran without ever reporting progress"
        );
    }

    // And each says what it counted, so a percentage over candidates is never
    // read as one over bytes.
    let unit_of = |wanted: Stage| {
        seen.iter().find_map(|event| match event {
            ScanEvent::StageProgress { stage, unit, .. } if *stage == wanted => Some(*unit),
            _ => None,
        })
    };
    assert_eq!(unit_of(Stage::Carve), Some(Unit::Bytes));
    assert_eq!(unit_of(Stage::Validation), Some(Unit::Items));
    assert_eq!(unit_of(Stage::Report), Some(Unit::Bytes));
}

/// Scans a medium holding one fragmented photograph under `budget`, and hands
/// back what reassembly announced itself as, every event seen, and the report.
fn reassembly_under(
    budget: Option<std::time::Duration>,
) -> ((Unit, u64), Vec<ScanEvent>, ScanReport) {
    let photo = argos_carve::fixture::photo_jpeg(320, 240, 0xC0FF_EE00_0000_0001);
    let block = argos_carve::classify::BLOCK_BYTES as u64;
    let header = 2 * MIB;
    let layout = argos_carve::fixture::planted(8 * MIB, &photo, &[header, header + MIB], block);

    let workers = 2;
    let config = ScanConfig::builder()
        .workers(NonZeroUsize::new(workers).expect("at least one worker"))
        .chunk_bytes(CHUNK)
        .min_long_side(0)
        .reassembly_budget(budget)
        .build()
        .expect("valid configuration");
    let session = ScanSession::new(config);
    let views: Vec<_> = (0..workers).map(|_| layout.source()).collect();
    let medium = Medium::new(views, layout.disk.len()).expect("medium");
    let mut sink = Collector::new();
    let events = Events::new();

    let report = session.start(medium, &mut sink, &events).expect("scan");
    let seen = events.seen();
    let announced = seen
        .iter()
        .find_map(|event| match event {
            ScanEvent::StageStarted { stage, unit, total } if *stage == Stage::Reassembly => {
                Some((*unit, *total))
            }
            _ => None,
        })
        .expect("reassembly must announce itself");
    (announced, seen, report)
}

/// Every reassembly progress event, so a test can hold the whole stage to one
/// unit rather than to whichever event it happened to look at.
fn reassembly_units(seen: &[ScanEvent]) -> Vec<Unit> {
    seen.iter()
        .filter_map(|event| match event {
            ScanEvent::StageProgress { stage, unit, .. } if *stage == Stage::Reassembly => {
                Some(*unit)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_deadline_bounded_reassembly_reports_against_the_deadline() {
    // The stage ends on its budget, so that is what it reports against. Its
    // queue cannot stand in: a step costs anything from seconds to over an
    // hour, and `plan_search` hands the expensive ones out first, so a
    // fraction of the steps runs far behind the work actually done — the field
    // run of `docs/defects/09` showed 1.75% of them having covered the regions
    // three quarters of the queue's weight sat in, and was stopped for it.
    let budget = std::time::Duration::from_secs(3600);
    let ((unit, total), seen, _) = reassembly_under(Some(budget));

    assert_eq!(
        unit,
        Unit::Seconds,
        "a stage that ends on a clock reports against that clock"
    );
    assert_eq!(
        total,
        budget.as_secs(),
        "the denominator is the budget, which is what the stage actually reaches"
    );
    assert!(
        unit.supports_percentage(),
        "elapsed of a budget is a fraction a display may show"
    );
    for unit in reassembly_units(&seen) {
        assert_eq!(unit, Unit::Seconds, "the unit must not change mid-stage");
    }
}

#[test]
fn reassembly_counts_steps_and_does_not_offer_them_as_a_candidate_count() {
    // Without a deadline there is nothing proportional to the time left, so
    // the stage falls back to what it can honestly say: how much of its queue
    // it has been through. That total is steps and not headers — the stage
    // searches every header twice, a gap search and a walk, and reads a region
    // besides. Reported as `items` it invites the arithmetic it cannot
    // support: a reader takes a quarter of the queue searched for an eighth,
    // and the manifest's `reassembly_attempted` is the number that means
    // headers (`A-CONFIDENCE-HONEST`).
    let ((unit, total), seen, report) = reassembly_under(None);

    assert_eq!(
        unit,
        Unit::Steps,
        "a stage whose item costs several steps counts steps, and says so"
    );
    assert!(
        total > report.reassembly_attempted,
        "the premise: {total} steps stands for fewer headers, so reading it as headers \
         understates what was searched"
    );
    assert!(
        !unit.supports_percentage(),
        "steps cost different amounts and the dear ones go first, so no display may \
         render a fraction of them as a fraction of the work"
    );
    for unit in reassembly_units(&seen) {
        assert_eq!(unit, Unit::Steps, "the unit must not change mid-stage");
    }
}

#[test]
fn the_report_stage_reaches_its_total_even_when_findings_are_not_stored() {
    // Progress measures the work a stage got through, and every finding costs
    // it a read whatever becomes of it. The duplicate below is read, hashed and
    // then not stored — as is any finding that reads back short, and any a
    // caller asked to leave unwritten. If those were missing from the numerator
    // the bar would stop short of the end on a run that did everything it
    // could, which reads on screen as a failure. What was *stored* is a
    // separate figure and stays separate.
    let mut disk = argos_carve::fixture::Disk::filled(CHUNK * 4);
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(96)
        .build();
    disk = disk.with(CHUNK / 2, &jpeg);
    disk = disk.with(CHUNK * 2, &jpeg);
    let image = disk.into_bytes();

    let session = ScanSession::new(config(2));
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let events = Events::new();

    let report = session.start(medium, &mut sink, &events).expect("scan");

    assert_eq!(report.duplicates, 1, "the fixture must produce a duplicate");
    assert_eq!(report.artifacts, 1, "and only one copy may be stored");

    let last = events
        .seen()
        .into_iter()
        .filter_map(|event| match event {
            ScanEvent::StageProgress {
                stage: Stage::Report,
                done,
                total,
                ..
            } => Some((done, total)),
            _ => None,
        })
        .next_back()
        .expect("the report stage reports progress");
    assert_eq!(
        last.0, last.1,
        "the report stage ended at {}/{} of its own work",
        last.0, last.1
    );
}

#[test]
fn stored_events_count_recoveries_and_never_candidates() {
    // What a live display shows while a scan runs comes from these events, so
    // what they count matters: a signature hit that has not passed its
    // format's state machine is not a recovery. The fixture disk below holds
    // three real images in noise that also yields hits which fail validation —
    // and the counts must follow the three (A-CONFIDENCE-HONEST).
    let (image, expected) = disk_with_images(3);
    let session = ScanSession::new(config(2));
    let medium = Medium::new(views(&image, 2), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let events = Events::new();

    let report = session.start(medium, &mut sink, &events).expect("scan");
    let stored: Vec<(u64, u64)> = events
        .seen()
        .iter()
        .filter_map(|event| match event {
            ScanEvent::ArtifactStored { artifacts, bytes } => Some((*artifacts, *bytes)),
            _ => None,
        })
        .collect();

    assert_eq!(
        stored.len() as u64,
        report.artifacts,
        "one event per artifact stored, no more and no fewer"
    );
    // Cumulative and monotonic: a display reads the latest and shows it, so a
    // figure that ever went backwards would be a figure that lied.
    for pair in stored.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        assert_eq!(after.0, before.0 + 1, "artifact counts skip nothing");
        assert!(after.1 > before.1, "byte counts only grow");
    }
    let last = stored.last().copied().expect("at least one artifact");
    assert_eq!(last.0, report.artifacts);
    assert_eq!(
        last.1,
        expected.iter().map(|image| image.len() as u64).sum::<u64>(),
        "the bytes reported are the bytes of the images that were recovered"
    );
}

#[test]
fn restricting_the_range_restricts_what_is_found() {
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    let image = argos_carve::fixture::Disk::filled(CHUNK * 4)
        .with(4096, &jpeg)
        .with(CHUNK * 3, &jpeg)
        .into_bytes();
    let config = ScanConfig::builder()
        .workers(NonZeroUsize::new(2).expect("two workers"))
        .chunk_bytes(CHUNK)
        .min_long_side(0)
        .range(argos_core::geometry::ByteOffset::new(CHUNK as u64)..)
        .stages(Stages {
            filesystem: false,
            carving: true,
            reassembly: false,
        })
        .build()
        .expect("valid configuration");

    let (artifacts, _) = scan_with(&image, config);

    let [artifact] = artifacts.as_slice() else {
        panic!("expected only the image inside the range");
    };
    assert_eq!(artifact.extents[0].start.get(), (CHUNK * 3) as u64);
}

#[test]
fn an_embedded_thumbnail_is_a_separate_lower_tier_artifact() {
    let thumbnail = argos_carve::fixture::Jpeg::new().build();
    let parent = argos_carve::fixture::Jpeg::new()
        .with_exif_thumbnail(thumbnail.clone())
        .build();
    let image = argos_carve::fixture::Disk::filled(CHUNK * 2)
        .with(4096, &parent)
        .into_bytes();

    let (artifacts, _) = scan(&image);

    let thumb = artifacts
        .iter()
        .find(|artifact| artifact.parent.is_some())
        .expect("the embedded thumbnail must be reported");
    assert_eq!(thumb.confidence, Confidence::PartialOrThumbnail);
    assert_eq!(thumb.bytes, thumbnail);
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact.bytes == parent && artifact.parent.is_none()),
        "the parent image is reported in its own right"
    );
}

#[test]
fn a_scan_that_covers_nothing_is_a_configuration_error() {
    let err = ScanConfig::builder()
        .stages(Stages {
            filesystem: false,
            carving: false,
            reassembly: false,
        })
        .build()
        .expect_err("a scan with no stages finds nothing and must be refused");
    assert!(err.to_string().contains("finds nothing"));

    let err = Medium::new(Vec::<Cursor<Vec<u8>>>::new(), 4096)
        .expect_err("a medium with no views cannot be read");
    assert!(err.to_string().contains("at least one"));
}

#[test]
fn filesystem_metadata_whose_bytes_do_not_validate_drops_to_the_partial_tier() {
    // Metadata pointing at bytes that do not assemble into a valid image —
    // the shape a sparse run, a lost fragment or a reallocated cluster
    // produces. The metadata is still evidence a file lived here, but the
    // result must not be reported as a whole file at the strongest tier
    // there is (A-CONFIDENCE-HONEST).
    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(8192)
        .build();
    let cluster = argos_fs::fixture::NTFS_CLUSTER;
    let kept = jpeg.len() - cluster;
    let mut file = argos_fs::fixture::FilePlan::fragmented(
        "holed.jpg",
        &[(64 * cluster, cluster), (32 * cluster, kept - cluster)],
    )
    .with_content(jpeg[..kept].to_vec());
    // The metadata insists the file is longer than the extents describe.
    file.content = jpeg[..kept].to_vec();
    let image = argos_fs::fixture::ntfs_volume(CHUNK * 8, 4 * cluster, &file);

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.recovered_name.is_some())
        .expect("the metadata is still evidence that a file lived here");
    // Structure broke, so the claim drops to the partial tier rather than
    // presenting a spliced file as filesystem-confirmed.
    assert_eq!(recovered.confidence, Confidence::PartialOrThumbnail);
    assert!(
        recovered.expected_length.is_some(),
        "an artifact must state the length its metadata claimed"
    );
}

#[test]
fn a_name_is_never_reported_against_a_different_filesystem_object() {
    use argos_core::geometry::{ByteOffset, ByteRange};
    use argos_engine::Finding;

    // Two findings over identical extents: one named, one not, from different
    // objects. Merging them must not attach the name to the other's object.
    let extents: Box<[ByteRange]> = Box::from([ByteRange::new(ByteOffset::new(4096), 16)]);
    let named = Finding {
        format: Format::Jpeg,
        stage: Stage::Filesystem,
        confidence: Confidence::FsMetadata,
        extents: extents.clone(),
        declared_size: Some(16),
        timestamps: argos_core::Timestamps::default(),
        deleted: None,
        name: Some("IMG_4471.JPG".into()),
        source_object: Some(100),
        parent: None,
    };
    let anonymous = Finding {
        name: None,
        source_object: Some(250),
        ..named.clone()
    };

    let merged = argos_engine::merge_for_test(vec![anonymous, named]);

    let [only] = merged.as_slice() else {
        panic!("identical extents collapse to one finding");
    };
    assert!(
        only.name.is_none() || only.source_object == Some(100),
        "a recovered name must stay with the object it was read from, got \
         name on object {:?}",
        only.source_object
    );
}

#[test]
fn a_fragmented_image_comes_back_through_the_pipeline_as_a_reassembly() {
    // The medium stored this image in two pieces with unrelated data between
    // them, which is what stage E exists for. It must come out of the scan as
    // an artifact — at the reassembled tier, with both extents recorded so the
    // claim can be replayed against the medium.
    let block = argos_carve::classify::BLOCK_BYTES;
    let image = argos_carve::fixture::photo_jpeg(320, 240, 0x51ED_2A11_0000_0001);
    let layout =
        argos_carve::fixture::fragmented(96 * block, &image, &[4 * block, 20 * block], block);

    let (artifacts, report) = scan_with(&layout.disk, config(2));

    let reassembled = artifacts
        .iter()
        .find(|artifact| artifact.confidence == Confidence::Reassembled)
        .unwrap_or_else(|| panic!("the fragmented image must be reassembled, got {artifacts:?}"));
    assert_eq!(reassembled.stage, Stage::Reassembly);
    assert_eq!(reassembled.extents.len(), 2, "both fragments are recorded");
    assert_eq!(
        reassembled.bytes, image,
        "the extents must hold the planted image, byte for byte"
    );
    assert_eq!(report.reassembled, 1);
    assert!(report.reassembly_attempted >= 1);
}

#[test]
fn a_medium_of_noise_costs_reassembly_nothing() {
    // False signature hits are the common case on a used disk. None of them
    // decodes a single MCU, so none becomes a fragmentation point and the
    // stage does no searching at all — which is what keeps it affordable
    // enough to run by default.
    let disk = argos_carve::fixture::Disk::noisy(CHUNK * 4, 0x2468_ACE0_1357_9BDF).into_bytes();

    let (artifacts, report) = scan_with(&disk, config(2));

    assert!(artifacts.is_empty());
    assert_eq!(report.reassembly_attempted, 0);
    assert!(!report.ceilings.reassembly_decodes);
}

#[test]
fn a_recovered_photograph_carries_its_camera_and_date_into_the_report() {
    // Two photographs from one camera on one afternoon, and a third from
    // another camera years earlier. Nothing about their offsets or their byte
    // counts separates them; what does is what each records about itself.
    //
    // The second is truncated, so it comes back only as the part that decodes
    // — and it must still say when it was taken, because that is exactly the
    // recovery a person cannot identify any other way.
    let whole = argos_carve::fixture::Jpeg::new()
        .with_capture(
            "NIKON CORPORATION",
            "NIKON D80",
            "2009:07:14 16:22:05",
            (3872, 2592),
        )
        .with_entropy_bytes(4096)
        .build();
    let partial_source = argos_carve::fixture::photo_jpeg(1600, 1200, 0x0CA5_0000_0000_0003);
    let older = argos_carve::fixture::Jpeg::new()
        .with_capture(
            "Canon",
            "Canon PowerShot A590",
            "2003:01:02 08:00:00",
            (2048, 1536),
        )
        .with_entropy_bytes(4096)
        .build();

    let disk = argos_carve::fixture::Disk::noisy(3 * 1024 * 1024, 0x0DA7_E000_0000_0001)
        .with(64 * 1024, &whole)
        .with(512 * 1024, &partial_source[..partial_source.len() * 6 / 10])
        .with(2 * 1024 * 1024, &older)
        .into_bytes();

    let (artifacts, _) = scan_with(&disk, config(2));

    let nikon = artifacts
        .iter()
        .find(|artifact| artifact.capture.model.as_deref() == Some("NIKON D80"))
        .expect("the whole photograph must report its camera");
    assert_eq!(nikon.capture.taken.as_deref(), Some("2009:07:14 16:22:05"));
    assert_eq!(nikon.capture.pixels, Some((3872, 2592)));

    let canon = artifacts
        .iter()
        .find(|artifact| artifact.capture.make.as_deref() == Some("Canon"))
        .expect("the older photograph must report its camera");
    assert_eq!(canon.capture.taken.as_deref(), Some("2003:01:02 08:00:00"));

    // A photograph without EXIF records nothing rather than something invented.
    let partial = artifacts
        .iter()
        .find(|artifact| artifact.confidence == Confidence::PartialOrThumbnail)
        .expect("the truncated photograph must come back as its decodable part");
    assert!(
        partial.capture.is_empty(),
        "nothing may be reported about a picture that recorded nothing: {:?}",
        partial.capture
    );
}

#[test]
fn a_recovery_whose_own_record_lost_its_name_is_named_by_the_index_slack() {
    // A directory that removes an entry leaves the entry in its index
    // buffer's slack, so a file's name can outlive the `$FILE_NAME` in its own
    // record. The index entry carries the MFT record number, which is what
    // ties the name to a recovery — it creates no extent, and a recovery that
    // still has its own name keeps it.
    let jpeg = argos_carve::fixture::Jpeg::new().build();
    // The record is planted nameless; only the index knows what it was called.
    let file =
        argos_fs::fixture::FilePlan::new("", 128 * 1024, jpeg.len()).with_content(jpeg.clone());
    let mut image = argos_fs::fixture::ntfs_volume(CHUNK * 6, 32 * 1024, &file);

    // An INDX buffer naming MFT record 1 — the record the fixture plants the
    // deleted file in — somewhere the sweep will meet it.
    let index = argos_fs::fixture::ntfs_indx(&[("childhood-birthday.jpg", 1)]);
    let at = CHUNK * 4;
    image[at..at + index.len()].copy_from_slice(&index);

    let (artifacts, _) = scan(&image);

    let recovered = artifacts
        .iter()
        .find(|artifact| artifact.stage == Stage::Filesystem)
        .expect("the deleted file must be recovered from its record");
    assert_eq!(
        recovered.recovered_name.as_deref(),
        Some("childhood-birthday.jpg"),
        "a name surviving only in index slack must reach the artifact"
    );
    assert_eq!(recovered.bytes, jpeg, "and it names the right bytes");
}
