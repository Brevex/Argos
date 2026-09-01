//! What the rules may claim, and where each direction has to stay silent.
//!
//! The two errors are not equally expensive. A run may be asked to leave
//! artifacts labelled `SyntheticAsset` unwritten, so a photograph called an
//! asset is a photograph missing from the output; an asset called a photograph
//! is a file on disk. Every test here is about that asymmetry — and about the
//! rule that keeps it honest: an image satisfying neither direction is
//! `Ambiguous`, which is reported and never treated as an asset.
//!
//! The eval harness at the end is the gate every rule or threshold change
//! has to pass. Its corpus is fixed by its seeds and disjoint from the range
//! the thresholds were derived over, so these numbers grade the rules against
//! samples that did not shape them, and it measures the *shipped* classifier,
//! because that is what an examiner's results are ordered by (A-EVAL-GATED).
//! Precision and recall are printed per slice and overall and held to
//! documented floors; changing a rule or a threshold means re-running this
//! and committing the new numbers with the change. The thresholds themselves
//! are derived in `tests/thresholds.rs`, over a disjoint range of the
//! generator — deriving them here would leave nothing grading anything.
//!
//! Nothing here decides what a scan reports. A classifier that scored every
//! sample wrongly would still recover every artifact; that property is proved
//! in `argos_engine`'s triage suite, not by these numbers.

use std::fmt::Write as _;

use argos_classify::Triage;
use argos_classify::fixture::{Slice, sample};
use argos_classify::rules::{self, features, screen};
use argos_core::ports::{Classifier, Decision, PixelImage, TriageLabel};

/// The label one image gets.
fn verdict(image: &PixelImage) -> TriageLabel {
    screen(&features(image), image.pixel_count()).label
}

#[test]
fn no_photograph_in_the_corpus_is_called_an_asset() {
    // The expensive direction. An artifact labelled asset is one a run may be
    // told not to write, so a wrong label here is evidence that never reaches
    // the output directory.
    for slice in [
        Slice::Photograph,
        Slice::PhotographThumbnail,
        Slice::PhotographGreyscale,
    ] {
        for index in 0..40 {
            let labeled = sample(slice, index * 7 + 1);
            assert_ne!(
                verdict(&labeled.image),
                TriageLabel::SyntheticAsset,
                "a {} was called a synthetic asset",
                slice.name()
            );
        }
    }
}

#[test]
fn no_asset_in_the_corpus_is_called_a_photograph() {
    for slice in [
        Slice::Icon,
        Slice::Sprite,
        Slice::UiChrome,
        Slice::HighResAsset,
    ] {
        for index in 0..40 {
            let labeled = sample(slice, index * 7 + 1);
            assert_ne!(
                verdict(&labeled.image),
                TriageLabel::Photograph,
                "a {} was called a photograph",
                slice.name()
            );
        }
    }
}

#[test]
fn a_photograph_label_always_rests_on_positive_evidence() {
    // Structural, not statistical. Every asset rule works by absence — no
    // transparency, no small palette — and a photograph verdict that also
    // rested on absence would rest on nothing. The only rule that may return
    // `Photograph` is the one that measured the sensor's own high-frequency
    // floor, and this asserts that no other path can.
    for slice in Slice::ALL {
        for index in 0..20 {
            let image = sample(slice, index * 13 + 3).image;
            let score = screen(&features(&image), image.pixel_count());
            if score.label == TriageLabel::Photograph {
                assert_eq!(score.decided_by, Decision::SensorTexture);
            }
            if score.label == TriageLabel::Ambiguous {
                assert_eq!(score.decided_by, Decision::Inconclusive);
            }
        }
    }
}

#[test]
fn an_image_with_no_texture_is_never_called_a_photograph() {
    // A flat fill has no high-frequency content at all, so it fails the one
    // rule that can claim a photograph however else it looks.
    let flat = PixelImage::new(64, 64, vec![200; 64 * 64 * 4]);
    let flat_features = features(&flat);
    assert!(
        flat_features.textured_fraction < f32::EPSILON,
        "a single-colour image has no high-frequency content at all, got {}",
        flat_features.textured_fraction
    );
    assert!(
        flat_features.flat_run_fraction > rules::FLAT_RUN_ASSET_FRACTION,
        "a single-colour image must read as entirely flat, got {}",
        flat_features.flat_run_fraction
    );
    assert_eq!(verdict(&flat), TriageLabel::SyntheticAsset);
}

#[test]
fn a_photograph_carries_the_high_frequency_floor_a_fill_does_not() {
    let photo = sample(Slice::Photograph, 11).image;
    let photo_features = features(&photo);
    assert!(
        photo_features.textured_fraction >= rules::PHOTOGRAPH_MIN_TEXTURE,
        "a photograph must carry sensor texture, got {}",
        photo_features.textured_fraction
    );
    assert!(
        photo_features.flat_run_fraction <= rules::PHOTOGRAPH_MAX_FLAT_RUN,
        "a photograph must not read as flat, got {}",
        photo_features.flat_run_fraction
    );
    assert_eq!(verdict(&photo), TriageLabel::Photograph);
}

#[test]
fn transparency_settles_an_image_on_its_own() {
    // JPEG has no alpha at all and a photograph stored as PNG is an opaque
    // scan or screenshot; transparency is an authoring feature. Half the
    // pixels here are transparent, which no sensor produces.
    let mut pixels = vec![255_u8; 64 * 64 * 4];
    for (index, chunk) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        chunk[3] = if index % 2 == 0 { 0 } else { 255 };
    }
    let image = PixelImage::new(64, 64, pixels);

    let score = screen(&features(&image), image.pixel_count());

    assert_eq!(score.label, TriageLabel::SyntheticAsset);
    assert_eq!(score.decided_by, Decision::Transparency);
}

#[test]
fn an_image_that_satisfies_neither_direction_is_reported_unclear() {
    // A smooth gradient: not flat enough to be an asset by its runs, not
    // textured enough to be a photograph. An honest "unclear" is what such an
    // image gets, and what keeps it out of the label a run may act on.
    let (width, height) = (256_u32, 256_u32);
    let mut pixels = Vec::with_capacity((width * height) as usize * 4);
    for y in 0..height {
        for x in 0..width {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "both coordinates are below 256 by construction"
            )]
            let (r, g) = (x as u8, y as u8);
            pixels.extend_from_slice(&[r, g, 128, 255]);
        }
    }
    let image = PixelImage::new(width, height, pixels);

    let score = screen(&features(&image), image.pixel_count());

    assert_eq!(score.label, TriageLabel::Ambiguous);
    assert_eq!(score.decided_by, Decision::Inconclusive);
}

/// Samples drawn per slice. Fixed: the eval set is these seeds and no others.
const EVAL_PER_SLICE: u64 = 40;

/// Seed of the sample at index `i` of a slice.
///
/// Stays under three hundred, far below `500_000`, where the validation range
/// in `tests/thresholds.rs` starts — so no sample here shaped a threshold it is
/// graded against.
fn eval_seed(index: u64) -> u64 {
    index * 7 + 1
}

/// A count over a count, as a fraction.
///
/// Every number this harness divides is a sample count in the low hundreds,
/// so the conversion is exact.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts here are in the hundreds and exact in f32"
)]
fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f32 / denominator as f32
}

/// What one slice's samples came to.
struct SliceResult {
    slice: Slice,
    /// Samples labeled correctly against [`Slice::truth`].
    correct: usize,
    /// Samples the classifier called `Ambiguous`.
    ambiguous: usize,
    total: usize,
}

impl SliceResult {
    fn accuracy(&self) -> f32 {
        ratio(self.correct, self.total)
    }
}

/// Counts for the photograph class, over the whole corpus.
#[derive(Default)]
struct Confusion {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
}

impl Confusion {
    fn precision(&self) -> f32 {
        ratio(self.true_positive, self.true_positive + self.false_positive)
    }

    fn recall(&self) -> f32 {
        ratio(self.true_positive, self.true_positive + self.false_negative)
    }
}

/// Floors this harness records. A change that drops below one fails here
/// rather than quietly reordering what an examiner sees first.
///
/// Each sits under what `rules-v1` measures on this corpus — precision 1.000,
/// recall 1.000, high-res asset accuracy 1.000 — so a threshold moved by a
/// fraction does not fail the build while a real regression does.
mod floor {
    /// Photograph precision: of everything called a photograph, how much
    /// really is one. This is the number that matters most — a synthetic
    /// asset sorted to the top of a results list wastes the examiner's
    /// attention on the one thing triage exists to move out of the way.
    pub const PHOTOGRAPH_PRECISION: f32 = 0.95;

    /// Photograph recall: of the real photographs, how many were labeled as
    /// such. A miss is cheaper than a false positive — the artifact is still
    /// in the manifest, merely sorted lower — so this floor sits below
    /// precision deliberately.
    pub const PHOTOGRAPH_RECALL: f32 = 0.90;

    /// Accuracy on the high-resolution-asset slice.
    ///
    /// Tracked separately because it is the slice resolution alone cannot
    /// decide: a 2560x1440 vector wallpaper is larger than most photographs,
    /// so a classifier that has quietly learned "big means photo" passes
    /// overall and collapses here.
    pub const HIGH_RES_ASSET_ACCURACY: f32 = 0.95;
}

#[test]
fn eval_corpus_meets_the_recorded_precision_and_recall() {
    let mut triage = Triage::new();

    let mut results = Vec::new();
    let mut overall = Confusion::default();

    for slice in Slice::ALL {
        let images: Vec<_> = (0..EVAL_PER_SLICE)
            .map(|index| sample(slice, eval_seed(index)).image)
            .collect();
        let scores = triage.score_batch(&images).expect("deciding cannot fail");

        let mut correct = 0;
        let mut ambiguous = 0;
        for score in &scores {
            let label = score.map_or(TriageLabel::Ambiguous, |score| score.label);
            let truth = slice.truth();
            if label == truth {
                correct += 1;
            }
            if label == TriageLabel::Ambiguous {
                ambiguous += 1;
            }
            match (truth, label) {
                (TriageLabel::Photograph, TriageLabel::Photograph) => overall.true_positive += 1,
                (TriageLabel::Photograph, _) => overall.false_negative += 1,
                (_, TriageLabel::Photograph) => overall.false_positive += 1,
                _ => {}
            }
        }
        results.push(SliceResult {
            slice,
            correct,
            ambiguous,
            total: images.len(),
        });
    }

    let mut table = String::from("\ntriage eval — accuracy by slice\n");
    for result in &results {
        let _ = writeln!(
            table,
            "  {:<18} {:>5.1}%   ({}/{} correct, {} ambiguous)",
            result.slice.name(),
            result.accuracy() * 100.0,
            result.correct,
            result.total,
            result.ambiguous
        );
    }
    let _ = writeln!(
        table,
        "\n  photograph precision {:.3}   recall {:.3}",
        overall.precision(),
        overall.recall()
    );
    println!("{table}");

    assert!(
        overall.precision() >= floor::PHOTOGRAPH_PRECISION,
        "photograph precision {:.3} is below the {:.2} this harness records\n{table}",
        overall.precision(),
        floor::PHOTOGRAPH_PRECISION
    );
    assert!(
        overall.recall() >= floor::PHOTOGRAPH_RECALL,
        "photograph recall {:.3} is below the {:.2} this harness records\n{table}",
        overall.recall(),
        floor::PHOTOGRAPH_RECALL
    );

    let high_res = results
        .iter()
        .find(|result| result.slice == Slice::HighResAsset)
        .expect("the high-resolution-asset slice is part of the corpus");
    assert!(
        high_res.accuracy() >= floor::HIGH_RES_ASSET_ACCURACY,
        "high-resolution-asset accuracy {:.3} is below the {:.2} this harness records — \
         a classifier that fails here has learned resolution, not content\n{table}",
        high_res.accuracy(),
        floor::HIGH_RES_ASSET_ACCURACY
    );
}

#[test]
fn the_manifest_can_name_what_labelled_it() {
    // A label is only reproducible against the procedure that produced it, and
    // with no model file there is no hash to pin — so what a scan records is
    // the version of the rules (A-MODEL-PINNED).
    let triage = Triage::new();
    let identity = triage
        .model()
        .expect("a real classifier names its procedure");
    assert_eq!(identity.version, argos_classify::RULES_VERSION);
}
