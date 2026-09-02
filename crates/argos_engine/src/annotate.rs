//! Annotating artifacts the writing stage has already stored.
//!
//! Everything here runs strictly after an artifact is persisted: [`Annotating`]
//! is handed the bytes the sink has already accepted, hashed and recorded. Its
//! entire output is annotation — a preview file and a list of scores keyed by
//! content hash.
//!
//! **Every method returns `()`.** That is what makes `A-TRIAGE-NOT-VERDICT` a
//! property of the types rather than of the order two statements happen to be
//! in: there is no value an annotation can hand back, so no path exists from
//! anything computed here to an artifact's existence, its extents or its
//! confidence tier. The [`Classifier`] port is reachable from this module
//! alone, and only from a method the writing stage calls after its sink has
//! taken the artifact.
//!
//! Two annotations want the same expensive thing: the artifact decoded to
//! pixels. So they share one decode, of bytes the writing stage already holds
//! — which is also why the medium is not read again for either. That is why
//! previews do not depend on triage being enabled: a scan with `--no-triage`
//! renders previews exactly as one with triage does.
//!
//! Perceptual-hash dedup runs before scoring, so near-duplicate images
//! collapse into one decision and share it. Scoring itself runs on one
//! dedicated worker thread, in batches, overlapping the writing of the next
//! artifacts with the scoring of the previous ones.

use std::collections::HashMap;

use argos_classify::{NEAR_DUPLICATE_DISTANCE, hamming, perceptual_hash};
use argos_core::Format;
use argos_core::ports::{ArtifactSink, Classifier, Digest, ModelIdentity, PixelImage, TriageScore};

use crate::finding::ScanReport;

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

/// Artifacts a scoring batch may hold at once.
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

/// Pixel budget of one scoring batch.
///
/// Decoded planes are four bytes per pixel, so a batch holds at most 128 MiB
/// of input (A-BOUNDED-ALLOC). The budget is checked against what a batch
/// would become rather than against what it already is: testing it
/// beforehand would let a batch just under the limit admit one more image of
/// up to [`decode::MAX_RGBA_PIXELS`], putting the real ceiling at three times
/// the stated one. An image larger than the whole budget still forms a batch
/// of one, because refusing to score it would be worse than holding it.
const BATCH_MAX_PIXELS: u64 = 32 * 1024 * 1024;

/// One artifact already persisted by the writing stage.
pub struct Emitted {
    /// Content hash the sink recorded.
    pub sha256: Digest,
    /// Where the artifact starts on the medium.
    pub offset: u64,
    /// Decoded dimensions, when it decoded.
    pub pixels: Option<(u32, u32)>,
    /// What the report stage established about the picture, for ordering.
    pub evidence: argos_classify::rank::Evidence,
}

/// What the writing stage produced for one artifact and this module scores.
#[derive(Debug, Default)]
pub(crate) struct Scored {
    /// The procedure that decided the labels, for the manifest.
    pub model: Option<ModelIdentity>,
    /// Whether the classifier gave up partway.
    pub degraded: bool,
    /// One outcome per persisted artifact, in the order they were written.
    pub outcomes: Vec<TriageOutcome>,
}

/// Runs `write` with an annotator beside it, and collects what came of it.
///
/// The scoring worker is spawned around the whole writing pass, so an artifact
/// is scored while the next one is being read and stored. It is joined here,
/// whatever `write` returned — a writing stage that failed still owes the
/// worker an end to its channel.
pub(crate) fn alongside<C, F, T>(
    classifier: Option<&mut C>,
    previews: bool,
    artifacts: usize,
    write: F,
) -> (T, Scored)
where
    C: Classifier + Send,
    F: FnOnce(&mut Annotating) -> T,
{
    let model = classifier.as_deref().and_then(Classifier::model);
    let (image_tx, image_rx) = crossbeam_channel::bounded::<(usize, PixelImage)>(1);
    std::thread::scope(|scope| {
        // No classifier, no worker, no channel: the writing stage still decodes
        // for the previews, and nothing is scored.
        let worker =
            classifier.map(|classifier| scope.spawn(move || score_worker(&image_rx, classifier)));
        let mut annotating = Annotating {
            previews,
            dedup: worker.is_some().then(|| Dedup::with_capacity(artifacts)),
            images: worker.is_some().then_some(image_tx),
        };

        let produced = write(&mut annotating);

        // Ends the channel, which is what tells the worker the run is over.
        let dedup = annotating.close();
        let Some(worker) = worker else {
            return (
                produced,
                Scored {
                    model,
                    ..Scored::default()
                },
            );
        };
        let (scores, degraded) = worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        let outcomes = dedup.map(|dedup| dedup.resolve(scores)).unwrap_or_default();
        (
            produced,
            Scored {
                model,
                degraded,
                outcomes,
            },
        )
    })
}

/// The annotation half of the writing pass.
///
/// Constructed only by [`alongside`], and reachable only from the point in the
/// writing stage that has just handed an artifact to its sink.
pub(crate) struct Annotating {
    /// Whether a preview is wanted of every artifact that decodes.
    previews: bool,
    /// Near-duplicate grouping, present only when a classifier is scoring.
    dedup: Option<Dedup>,
    /// Images on their way to the scoring worker.
    images: Option<crossbeam_channel::Sender<(usize, PixelImage)>>,
}

impl Annotating {
    /// Whether anything at all is wanted, so the writing stage can skip the
    /// decode entirely.
    pub(crate) fn wanted(&self) -> bool {
        self.previews || self.dedup.is_some()
    }

    /// Records a stored artifact that produced no image to annotate.
    ///
    /// It is reported unscored, never omitted: an artifact past the decode
    /// ceiling or whose pixels will not come back is still an artifact, and
    /// the manifest says so.
    pub(crate) fn undecoded(&mut self, sha256: Digest) {
        if let Some(dedup) = self.dedup.as_mut() {
            dedup.push_undecoded(sha256);
        }
    }

    /// Annotates one artifact from the bytes it was stored from.
    ///
    /// Called after the sink has accepted it, and returns nothing: whatever
    /// the classifier makes of the picture cannot reach the decision that put
    /// it on disk, because there is no value here for that decision to read
    /// (`A-TRIAGE-NOT-VERDICT`).
    pub(crate) fn annotate<S: ArtifactSink>(
        &mut self,
        sink: &mut S,
        sha256: Digest,
        format: Format,
        bytes: &[u8],
        report: &mut ScanReport,
    ) {
        if !self.wanted() {
            return;
        }
        let Some(image) = argos_carve::decode::decode_rgba(format, bytes) else {
            self.undecoded(sha256);
            return;
        };

        if self.previews {
            match sink.preview(&sha256, &image) {
                Ok(()) => report.previews_written += 1,
                // The artifact is stored, hashed and recorded already. A full
                // output directory costs its thumbnail and nothing else, and
                // the count is what says so.
                Err(_) => report.previews_failed += 1,
            }
        }

        if let Some(dedup) = self.dedup.as_mut()
            && let Some(index) = dedup.push(sha256, &image)
            && let Some(images) = self.images.as_ref()
        {
            // A closed channel means the worker died with the classifier's
            // error; keep going so every artifact still gets its outcome.
            let _ = images.send((index, image));
        }
    }

    /// Ends the run: closes the channel and gives up the grouping.
    fn close(mut self) -> Option<Dedup> {
        drop(self.images.take());
        self.dedup.take()
    }
}

/// Collapses near-duplicate images so one decision speaks for a group.
///
/// Two artifacts whose perceptual hashes are within
/// [`NEAR_DUPLICATE_DISTANCE`] are the same picture as far as a label
/// is concerned — a thumbnail and its parent, or one file recovered twice from
/// two places on the medium. Both stay in the manifest; only one is scored.
struct Dedup {
    /// One outcome per artifact, in emit order.
    outcomes: Vec<TriageOutcome>,
    /// Group representatives: perceptual hash and outcome index.
    groups: Vec<(u64, usize)>,
    /// Group positions by (band, band value).
    ///
    /// Comparing a new hash against every group is quadratic, and a disk of
    /// distinct images makes every artifact a new group — so on a real medium
    /// that is the whole recovery compared against itself, in a stage that
    /// runs after the images are already written and looks like a hang.
    index: HashMap<(u32, u16), Vec<usize>>,
    /// Near-duplicates waiting for their representative's score, as
    /// (follower, representative) outcome indices.
    followers: Vec<(usize, usize)>,
}

/// Bands the 64-bit perceptual hash is split into for the lookup above.
///
/// Two hashes within [`NEAR_DUPLICATE_DISTANCE`] differ in at most that
/// many bits, so with strictly more bands than that distance at least one band
/// has to be identical. Four bands cover a distance of three, which is what
/// that constant is — the index therefore finds every pair the exhaustive
/// search found, and the pigeonhole is what proves it rather than a measured
/// hit rate.
const BANDS: u32 = 4;

/// Bits per band. Sixteen, and it follows from [`BANDS`].
const BAND_BITS: u32 = u64::BITS / BANDS;

/// The value of one band of a hash.
const fn band(hash: u64, band: u32) -> u16 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "masked to BAND_BITS, which is exactly u16's width"
    )]
    let value = (hash >> (band * BAND_BITS)) as u16;
    value
}

impl Dedup {
    fn with_capacity(artifacts: usize) -> Self {
        Self {
            outcomes: Vec::with_capacity(artifacts),
            groups: Vec::new(),
            index: HashMap::new(),
            followers: Vec::new(),
        }
    }

    /// The earliest group within the near-duplicate distance of `hash`.
    ///
    /// Earliest, not any: which artifact speaks for a group decides which
    /// content hash every follower is recorded against, and that has to be
    /// the same on two runs over the same medium.
    fn nearest(&self, hash: u64) -> Option<usize> {
        let mut best: Option<usize> = None;
        for band_index in 0..BANDS {
            let Some(bucket) = self.index.get(&(band_index, band(hash, band_index))) else {
                continue;
            };
            // Buckets are appended in group order, so the first match in one is
            // the earliest it holds.
            let found = bucket
                .iter()
                .copied()
                .find(|&at| hamming(hash, self.groups[at].0) <= NEAR_DUPLICATE_DISTANCE);
            if let Some(at) = found {
                best = Some(best.map_or(at, |current| current.min(at)));
            }
        }
        best.map(|at| self.groups[at].1)
    }

    /// Records a new group and files it under each of its bands.
    fn add_group(&mut self, hash: u64, at: usize) {
        let position = self.groups.len();
        self.groups.push((hash, at));
        for band_index in 0..BANDS {
            self.index
                .entry((band_index, band(hash, band_index)))
                .or_default()
                .push(position);
        }
    }

    /// Records an artifact that could not be read back or decoded. It is
    /// reported unscored, never omitted.
    fn push_undecoded(&mut self, sha256: Digest) {
        self.outcomes.push(TriageOutcome {
            sha256,
            perceptual_hash: None,
            near_duplicate_of: None,
            score: None,
        });
    }

    /// Records a decoded image.
    ///
    /// Returns the outcome index to score, or `None` when a near-duplicate
    /// already in the set will speak for this one.
    fn push(&mut self, sha256: Digest, image: &PixelImage) -> Option<usize> {
        // An image with no structure gets no hash, and therefore no group:
        // sharing a score between two pictures the hash cannot tell apart
        // would attribute one image's label to another.
        let hash = perceptual_hash(image);
        let near = hash.and_then(|hash| self.nearest(hash));
        let at = self.outcomes.len();

        if let Some(representative) = near {
            self.followers.push((at, representative));
            self.outcomes.push(TriageOutcome {
                sha256,
                perceptual_hash: hash,
                near_duplicate_of: Some(self.outcomes[representative].sha256),
                score: None,
            });
            return None;
        }

        if let Some(hash) = hash {
            self.add_group(hash, at);
        }
        self.outcomes.push(TriageOutcome {
            sha256,
            perceptual_hash: hash,
            near_duplicate_of: None,
            score: None,
        });
        Some(at)
    }

    /// Applies the worker's scores and hands each near-duplicate its
    /// representative's.
    fn resolve(mut self, scores: Vec<(usize, Option<TriageScore>)>) -> Vec<TriageOutcome> {
        for (index, score) in scores {
            if let Some(outcome) = self.outcomes.get_mut(index) {
                outcome.score = score;
            }
        }
        for (follower, representative) in self.followers {
            let score = self.outcomes.get(representative).and_then(|rep| rep.score);
            if let Some(outcome) = self.outcomes.get_mut(follower) {
                outcome.score = score;
            }
        }
        self.outcomes
    }
}

/// The dedicated scoring worker: drains the channel in batches and scores
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

#[cfg(test)]
mod tests {
    use argos_classify::{NEAR_DUPLICATE_DISTANCE, hamming};

    use super::{BANDS, Dedup, band};

    /// What the lookup replaced: compare against every group, earliest wins.
    fn exhaustive(groups: &[(u64, usize)], hash: u64) -> Option<usize> {
        groups
            .iter()
            .find(|(other, _)| hamming(hash, *other) <= NEAR_DUPLICATE_DISTANCE)
            .map(|&(_, at)| at)
    }

    /// A deterministic spread of hashes, with deliberate near-misses.
    fn hashes(count: usize) -> Vec<u64> {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.push(match index % 7 {
                // Every so often, a hash a hair away from an earlier one: at
                // the distance limit, one bit inside it, and one bit outside.
                1 if !out.is_empty() => out[index / 7] ^ 0b111,
                2 if !out.is_empty() => out[index / 7] ^ 0b1,
                3 if !out.is_empty() => out[index / 7] ^ 0b1111,
                _ => state,
            });
        }
        out
    }

    #[test]
    fn the_banded_lookup_finds_exactly_what_comparing_against_everything_found() {
        // The index is an optimisation, and an optimisation that changes which
        // artifact speaks for a group changes the manifest. The pigeonhole
        // argument in `BANDS` says it cannot; this checks it.
        let mut dedup = Dedup::with_capacity(0);
        let mut mirror: Vec<(u64, usize)> = Vec::new();

        for (at, hash) in hashes(4000).into_iter().enumerate() {
            let indexed = dedup.nearest(hash);
            let scanned = exhaustive(&mirror, hash);
            assert_eq!(
                indexed, scanned,
                "the index and the exhaustive scan disagreed at {at} on {hash:#018x}"
            );
            if indexed.is_none() {
                dedup.add_group(hash, at);
                mirror.push((hash, at));
            }
        }
        assert!(
            mirror.len() > 1000,
            "a test whose hashes all collapse into one group proves nothing"
        );
    }

    #[test]
    fn every_band_of_a_hash_is_a_distinct_slice_of_it() {
        // The pigeonhole holds only if the bands partition the hash. A band
        // that overlapped another would leave bits unaccounted for.
        let hash = 0x0123_4567_89AB_CDEF_u64;
        let rebuilt = (0..BANDS).fold(0_u64, |acc, index| {
            acc | (u64::from(band(hash, index)) << (index * super::BAND_BITS))
        });
        assert_eq!(rebuilt, hash);
    }

    #[test]
    fn two_hashes_at_the_distance_limit_always_share_a_band() {
        // The property the whole lookup rests on, checked over every way of
        // spreading three differing bits across the sixty-four.
        for a in 0..u64::BITS {
            for b in 0..u64::BITS {
                for c in 0..u64::BITS {
                    let other = (1 << a) | (1 << b) | (1 << c);
                    assert!(hamming(0, other) <= NEAR_DUPLICATE_DISTANCE);
                    assert!(
                        (0..BANDS).any(|index| band(0, index) == band(other, index)),
                        "bits {a},{b},{c} spoiled every band"
                    );
                }
            }
        }
    }
}
