//! The eval harness: the gate every rule or threshold change has to pass.
//!
//! The corpus is fixed by its seeds and disjoint from the range the thresholds
//! were derived over, so these numbers grade the rules against samples that did
//! not shape them. The harness measures the *shipped* classifier, because that
//! is what an examiner's results are ordered by (A-EVAL-GATED).
//!
//! Precision and recall are printed per slice and overall, and held to
//! documented floors. Changing a rule or a threshold means re-running this and
//! committing the new numbers with the change. The thresholds themselves are
//! derived in `tests/thresholds.rs`, over a disjoint range of the generator —
//! deriving them here would leave nothing grading anything.
//!
//! Nothing here decides what a scan reports. A classifier that scored every
//! sample wrongly would still recover every artifact; that property is proved
//! in `argos_engine`'s triage suite, not by these numbers.

use std::fmt::Write as _;

use argos_classify::Triage;
use argos_classify::fixture::{Slice, sample};
use argos_core::classify::{Classifier, TriageLabel};

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
