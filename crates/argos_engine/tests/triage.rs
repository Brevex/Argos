//! What triage may and may not do to a scan's output.
//!
//! The central property of this stage is negative: whatever a classifier
//! says, thinks or does, the set of artifacts a scan produces must be exactly
//! the set it would produce with no classifier at all (A-TRIAGE-NOT-VERDICT).
//! These tests prove it by running the same medium twice — once with no
//! classifier, once with a hostile one — and comparing the sink's output byte
//! for byte.

use std::convert::Infallible;
use std::io::Cursor;
use std::num::NonZeroUsize;

use argos_core::ports::Discard;
use argos_core::ports::{
    Classifier, Decision, ModelIdentity, PixelImage, TriageLabel, TriageScore,
};
use argos_engine::fixture::{Collected, Collector};
use argos_engine::{Medium, ScanConfig, ScanReport, ScanSession};

const CHUNK: usize = argos_engine::config::MIN_CHUNK_BYTES;

/// A classifier that condemns everything it sees, with maximum confidence.
///
/// If any code path anywhere could act on a score, this is the input that
/// would make it act: every artifact is a synthetic asset with probability
/// zero of being a photograph.
#[derive(Debug, Default)]
struct CondemnEverything {
    seen: usize,
}

impl Classifier for CondemnEverything {
    type Error = Infallible;

    fn model(&self) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            version: "condemn-everything",
        })
    }

    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        self.seen += images.len();
        Ok(vec![
            Some(TriageScore {
                label: TriageLabel::SyntheticAsset,
                decided_by: Decision::SensorTexture,
            });
            images.len()
        ])
    }
}

/// A classifier that fails on every call.
#[derive(Debug, Default)]
struct AlwaysBroken;

#[derive(Debug)]
struct Broken;

impl std::fmt::Display for Broken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this classifier is broken")
    }
}

impl std::error::Error for Broken {}

impl Classifier for AlwaysBroken {
    type Error = Broken;

    fn model(&self) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            version: "always-broken",
        })
    }

    fn score_batch(
        &mut self,
        _images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        Err(Broken)
    }
}

/// A disk holding decodable photographs, so triage has real pixels to score.
fn disk_with_photographs(count: usize) -> Vec<u8> {
    let mut disk = argos_carve::fixture::Disk::noisy(CHUNK * 4, 0x7A1D_0001);
    for index in 0..count {
        let photo = argos_carve::fixture::photo_jpeg(160, 120, 0xBEEF + index as u64);
        disk = disk.with(CHUNK / 2 + index * CHUNK, &photo);
    }
    disk.into_bytes()
}

fn config() -> ScanConfig {
    ScanConfig::builder()
        .workers(NonZeroUsize::new(2).expect("two workers"))
        .chunk_bytes(CHUNK)
        // The generated fixtures are a few dozen pixels across; these tests
        // are about the pipeline, not about which sizes reach a directory.
        .min_long_side(0)
        .build()
        .expect("valid configuration")
}

fn views(image: &[u8]) -> Vec<Cursor<Vec<u8>>> {
    (0..2).map(|_| Cursor::new(image.to_vec())).collect()
}

fn scan_without_triage(image: &[u8]) -> (Vec<Collected>, ScanReport) {
    let session = ScanSession::new(config());
    let medium = Medium::new(views(image), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let report = session.start(medium, &mut sink, &Discard).expect("scan");
    (sink.artifacts().to_vec(), report)
}

fn scan_with_triage<C: Classifier + Send>(
    image: &[u8],
    classifier: &mut C,
) -> (Vec<Collected>, ScanReport) {
    let session = ScanSession::new(config());
    let medium = Medium::new(views(image), image.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let report = session
        .start_with_classifier(medium, &mut sink, &Discard, classifier)
        .expect("scan");
    (sink.artifacts().to_vec(), report)
}

#[test]
fn a_classifier_condemning_every_artifact_removes_none_of_them() {
    let disk = disk_with_photographs(3);

    let (without, plain_report) = scan_without_triage(&disk);
    let mut hostile = CondemnEverything::default();
    let (with, triaged) = scan_with_triage(&disk, &mut hostile);

    assert!(!without.is_empty(), "the fixture must recover something");
    assert_eq!(
        without, with,
        "triage changed the recovered artifacts: the set a scan produces must not \
         depend on what a classifier says about it"
    );
    assert_eq!(plain_report.artifacts, triaged.artifacts);
    assert!(hostile.seen > 0, "the classifier must actually have run");
    // And every one of them was condemned, so the equality above is not
    // vacuous.
    assert!(
        triaged
            .triage
            .iter()
            .filter_map(|outcome| outcome.score)
            .all(|score| score.label == TriageLabel::SyntheticAsset),
        "the hostile classifier's verdict did not reach the report"
    );
}

#[test]
fn a_classifier_that_fails_leaves_artifacts_reported_but_unscored() {
    let disk = disk_with_photographs(2);

    let (without, _) = scan_without_triage(&disk);
    let (with, report) = scan_with_triage(&disk, &mut AlwaysBroken);

    assert_eq!(
        without, with,
        "a broken classifier must not cost a single recovered artifact"
    );
    assert!(report.triage_degraded, "the failure must be reported");
    assert_eq!(report.triage_scored, 0);
    assert!(
        report.triage_unscored > 0,
        "artifacts the classifier could not score are counted, not hidden"
    );
}

#[test]
fn every_persisted_artifact_gets_an_outcome_keyed_by_its_hash() {
    let disk = disk_with_photographs(3);

    let mut hostile = CondemnEverything::default();
    let (artifacts, report) = scan_with_triage(&disk, &mut hostile);

    assert_eq!(
        report.triage.len(),
        artifacts.len(),
        "one triage outcome per persisted artifact"
    );
    for artifact in &artifacts {
        assert!(
            report
                .triage
                .iter()
                .any(|outcome| outcome.sha256 == artifact.sha256),
            "a persisted artifact has no triage outcome to attribute a label to"
        );
    }
}

#[test]
fn identical_images_collapse_into_one_decision_and_share_its_score() {
    // The same photograph planted twice at different offsets. Content-hash
    // dedup drops the second copy before triage, so plant a re-encoded
    // near-duplicate instead: same picture, different bytes.
    let photo = argos_carve::fixture::photo_jpeg(160, 120, 0xD00D);
    let similar = argos_carve::fixture::photo_jpeg_quality(160, 120, 0xD00D, 70);
    assert_ne!(photo, similar, "the two encodings must differ in bytes");

    let disk = argos_carve::fixture::Disk::noisy(CHUNK * 3, 0x7A1D_0002)
        .with(CHUNK / 2, &photo)
        .with(CHUNK + CHUNK / 2, &similar)
        .into_bytes();

    let mut hostile = CondemnEverything::default();
    let (artifacts, report) = scan_with_triage(&disk, &mut hostile);

    assert_eq!(artifacts.len(), 2, "both encodings are separate artifacts");
    assert_eq!(
        hostile.seen, 1,
        "a near-duplicate must not be scored on its own"
    );
    assert!(
        report
            .triage
            .iter()
            .any(|outcome| outcome.near_duplicate_of.is_some()),
        "the near-duplicate relationship must be recorded"
    );
    // Both still carry the score, and both are still artifacts.
    assert_eq!(report.triage_scored, 2);
}

#[test]
fn an_artifact_the_floor_kept_out_is_never_shown_to_the_classifier() {
    // The port is reached from one place: after the sink has taken the
    // artifact and recorded its digest. So an artifact the run decided not to
    // store is not one the classifier gets an opinion about — there is nothing
    // for an opinion to be attached to, and offering one would be the first
    // half of a path from a score to a disposal (`A-TRIAGE-NOT-VERDICT`).
    let disk = disk_with_photographs(3);
    let floored = ScanConfig::builder()
        .workers(NonZeroUsize::new(2).expect("two workers"))
        .chunk_bytes(CHUNK)
        // Far above the fixtures, so every one of them is recorded and none is
        // written.
        .min_long_side(4096)
        .previews(true)
        .build()
        .expect("valid configuration");

    let medium = Medium::new(views(&disk), disk.len() as u64).expect("medium");
    let mut sink = Collector::new();
    let mut hostile = CondemnEverything::default();
    let report = ScanSession::new(floored)
        .start_with_classifier(medium, &mut sink, &Discard, &mut hostile)
        .expect("scan");

    assert!(
        report.omitted_assets > 0,
        "the floor must actually have kept something out"
    );
    assert_eq!(report.artifacts, 0, "and stored nothing");
    assert_eq!(
        hostile.seen, 0,
        "the classifier was shown an artifact the run never stored"
    );
    assert!(
        report.triage.is_empty(),
        "an unstored artifact has no outcome"
    );
    assert_eq!(report.previews_written, 0);
}
