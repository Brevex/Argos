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
    assert!(!report.reassembly_budget_exhausted);
}
