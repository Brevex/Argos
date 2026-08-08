//! Stage G: ML triage of persisted artifacts.
//!
//! Triage runs strictly after the report stage: every artifact it sees is
//! already streamed to the sink, hashed and recorded. Its entire output is a
//! list of annotations keyed by content hash — there is no path from a score
//! back to an artifact's existence (A-TRIAGE-NOT-VERDICT).
//!
//! Perceptual-hash dedup runs first, so near-duplicate images collapse into
//! one inference and share its score. Inference itself runs on one dedicated
//! worker thread, in batches, overlapping the decode of the next images with
//! the scoring of the previous ones.

use std::io::{Read, Seek};

use argos_carve::decode;
use argos_classify::phash;
use argos_core::Stage;
use argos_core::artifact::Digest;
use argos_core::classify::{Classifier, PixelImage, TriageScore};
use argos_core::progress::{ProgressSink, ScanEvent};
use sha2::{Digest as _, Sha256};

use crate::finding::{Finding, ScanReport};
use crate::session::Control;

/// One artifact's triage annotation, keyed by the artifact's content hash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriageOutcome {
    /// SHA-256 of the artifact the annotation belongs to.
    pub sha256: Digest,
    /// Perceptual hash of the decoded image; absent when it did not decode
    /// within bounds.
    pub perceptual_hash: Option<u64>,
    /// Content hash of the artifact this one is a near-duplicate of, when the
    /// perceptual hashes collapsed them. The duplicate shares that artifact's
    /// score; both stay in the manifest.
    pub near_duplicate_of: Option<Digest>,
    /// The classifier's opinion; absent when the image was not scored.
    pub score: Option<TriageScore>,
}

/// Artifacts an inference batch may hold at once.
///
/// Batching buys no throughput here, and the number says so. Measured
/// (`cargo bench -p argos_classify --bench classify`): 2.61 ms per image
/// alone, 2.68 ms each in fours, 2.98 ms each in eights. The classifier runs
/// a plain loop over a fixed-size input, so there is no per-call setup to
/// amortize — only a working set that grows until it stops fitting cache
/// (`M-HOTPATH`).
///
/// What the worker below is actually for is pipelining: it decodes the next
/// artifacts while scoring the previous ones, on separate threads. The batch
/// is kept small because a batch is resident pixel planes, and four is enough
/// to keep the worker from idling between images.
const BATCH_MAX_IMAGES: usize = 4;

/// Pixel budget of one inference batch.
///
/// Decoded planes are four bytes per pixel, so a batch holds at most 128 MiB
/// of input (A-BOUNDED-ALLOC). The budget is checked against what a batch
/// would become rather than against what it already is: testing it
/// beforehand would let a batch just under the limit admit one more image of
/// up to [`decode::MAX_RGBA_PIXELS`], putting the real ceiling at three times
/// the stated one. An image larger than the whole budget still forms a batch
/// of one, because refusing to score it would be worse than holding it.
const BATCH_MAX_PIXELS: u64 = 32 * 1024 * 1024;

/// One artifact already persisted by the report stage.
pub(crate) struct Emitted {
    /// Index into the findings the report stage emitted from.
    pub finding: usize,
    /// Content hash the sink recorded.
    pub sha256: Digest,
}

/// Scores every emitted artifact and annotates the report.
///
/// Never fails: an image that cannot be read back, decoded within bounds or
/// scored is counted and left unscored. A classifier hard failure stops
/// scoring but keeps every annotation produced so far.
pub(crate) fn run<V, C, P>(
    control: &Control,
    view: &mut V,
    findings: &[Finding],
    emitted: &[Emitted],
    classifier: &mut C,
    progress: &P,
    report: &mut ScanReport,
) where
    V: Read + Seek,
    C: Classifier + Send,
    P: ProgressSink + ?Sized,
{
    progress.emit(ScanEvent::StageStarted {
        stage: Stage::Triage,
        bytes_total: 0,
    });
    report.triage_model = classifier.model();

    let (image_tx, image_rx) = crossbeam_channel::bounded::<(usize, PixelImage)>(1);
    let scored = std::thread::scope(|scope| {
        let worker = scope.spawn(move || score_worker(&image_rx, classifier));

        let mut outcomes: Vec<TriageOutcome> = Vec::with_capacity(emitted.len());
        // Group representatives: perceptual hash and outcome index.
        let mut groups: Vec<(u64, usize)> = Vec::new();
        // Near-duplicates waiting for their representative's score.
        let mut followers: Vec<(usize, usize)> = Vec::new();
        let mut buf = Vec::new();

        for entry in emitted {
            let unscored = TriageOutcome {
                sha256: entry.sha256,
                perceptual_hash: None,
                near_duplicate_of: None,
                score: None,
            };
            if control.is_cancelled() {
                outcomes.push(unscored);
                continue;
            }
            let Some(finding) = findings.get(entry.finding) else {
                outcomes.push(unscored);
                continue;
            };
            let Some(image) = decode_finding(view, finding, entry.sha256, &mut buf) else {
                outcomes.push(unscored);
                continue;
            };

            // An image with no structure gets no hash, and therefore no
            // group: sharing a score between two pictures the hash cannot
            // tell apart would attribute one image's label to another.
            let hash = phash::perceptual_hash(&image);
            let near = hash.and_then(|hash| {
                groups.iter().find(|(other, _)| {
                    phash::hamming(hash, *other) <= phash::NEAR_DUPLICATE_DISTANCE
                })
            });
            if let Some(&(_, representative)) = near {
                followers.push((outcomes.len(), representative));
                outcomes.push(TriageOutcome {
                    perceptual_hash: hash,
                    near_duplicate_of: Some(outcomes[representative].sha256),
                    ..unscored
                });
                continue;
            }

            if let Some(hash) = hash {
                groups.push((hash, outcomes.len()));
            }
            outcomes.push(TriageOutcome {
                perceptual_hash: hash,
                ..unscored
            });
            // A closed channel means the worker died with the classifier's
            // error; keep going so every artifact still gets its outcome.
            let _ = image_tx.send((outcomes.len() - 1, image));
        }
        drop(image_tx);

        let (scores, failed) = worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        report.triage_degraded = failed;
        for (index, score) in scores {
            if let Some(outcome) = outcomes.get_mut(index) {
                outcome.score = score;
            }
        }
        // Near-duplicates share their representative's score: the pictures
        // are the same, so one inference speaks for the group.
        for (follower, representative) in followers {
            let score = outcomes.get(representative).and_then(|rep| rep.score);
            if let Some(outcome) = outcomes.get_mut(follower) {
                outcome.score = score;
            }
        }
        outcomes
    });

    report.triage_scored = scored
        .iter()
        .filter(|outcome| outcome.score.is_some())
        .count() as u64;
    report.triage_unscored = (scored.len() as u64).saturating_sub(report.triage_scored);
    report.triage = scored;

    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Triage,
        findings: report.triage_scored,
    });
}

/// The dedicated inference worker: drains the channel in batches and scores
/// them.
///
/// Returns the scores it produced and whether the classifier hard-failed. On
/// failure the remaining images are left unscored — never guessed at.
fn score_worker<C: Classifier>(
    images: &crossbeam_channel::Receiver<(usize, PixelImage)>,
    classifier: &mut C,
) -> (Vec<(usize, Option<TriageScore>)>, bool) {
    let mut scores = Vec::new();
    let mut batch: Vec<PixelImage> = Vec::with_capacity(BATCH_MAX_IMAGES);
    let mut indices: Vec<usize> = Vec::with_capacity(BATCH_MAX_IMAGES);
    // An image taken off the channel that did not fit the batch being built;
    // it opens the next one.
    let mut pending: Option<(usize, PixelImage)> = None;

    // Block for the first image of a batch, then take whatever else is
    // already waiting, up to the batch bounds.
    while let Some((index, image)) = pending.take().or_else(|| images.recv().ok()) {
        let mut pixels = image.pixel_count();
        batch.push(image);
        indices.push(index);
        while batch.len() < BATCH_MAX_IMAGES {
            let Ok((index, image)) = images.try_recv() else {
                break;
            };
            let with_image = pixels.saturating_add(image.pixel_count());
            if with_image > BATCH_MAX_PIXELS {
                // Over budget with this image in it. Score what is here and
                // let it open the next batch, rather than exceeding the
                // ceiling by a whole image.
                pending = Some((index, image));
                break;
            }
            pixels = with_image;
            batch.push(image);
            indices.push(index);
        }

        let Ok(answers) = classifier.score_batch(&batch) else {
            // The classifier itself broke. Everything still queued stays
            // unscored; annotating fewer artifacts is the honest failure.
            scores.extend(indices.drain(..).map(|index| (index, None)));
            return (scores, true);
        };
        scores.extend(indices.drain(..).zip(answers));
        batch.clear();
    }
    (scores, false)
}

/// Reads an artifact's bytes back and decodes them for scoring.
///
/// `None` when the artifact exceeds the decode ceiling, cannot be read back,
/// does not hash to what was stored, or does not decode — all of which leave
/// it unscored, not unreported.
///
/// This is the *third* read of these extents: the report stage hashed them
/// and then streamed them to the sink. An annotation is keyed by the digest
/// that stage recorded, so bytes that no longer hash to it would attach a
/// label and a perceptual hash to a content hash they do not describe
/// (A-PROVENANCE). Re-hashing here costs one pass over an image already in
/// memory and makes the annotation say something true.
fn decode_finding<V: Read + Seek>(
    view: &mut V,
    finding: &Finding,
    sha256: Digest,
    buf: &mut Vec<u8>,
) -> Option<PixelImage> {
    let length = usize::try_from(finding.length()).ok()?;
    if length > decode::MAX_DECODE_BYTES {
        return None;
    }
    buf.clear();
    buf.reserve(length);
    let mut bytes = crate::pipeline::ExtentReader::new(view, &finding.extents);
    bytes.read_to_end(buf).ok()?;
    if buf.len() != length {
        return None;
    }
    if Digest::new(Sha256::digest(&*buf).into()) != sha256 {
        // The medium answered differently than it did during the report
        // stage. The artifact stands as written and hashed; what cannot be
        // done is describe it from bytes that are not it.
        return None;
    }
    decode::decode_rgba(finding.format, buf)
}
