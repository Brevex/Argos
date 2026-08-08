//! The eval harness: the gate every model or threshold change has to pass.
//!
//! The corpus is fixed by its seeds and disjoint from the training tool's
//! range, so these numbers measure generalization rather than recall of the
//! training set. The harness measures the *shipped* classifier — rule-based
//! pre-filter and model together — because that is what an examiner's results
//! are ordered by (A-EVAL-GATED).
//!
//! Precision and recall are printed per slice and overall, and held to
//! documented floors. Changing a threshold or replacing the model means
//! re-running this and committing the new numbers with the change.
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
/// Stays under `100_000`, where `tools/train_triage` starts, so no sample here
/// was ever trained on.
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
/// Each sits a little under what `triage-cnn-v1` measures, so ordinary
/// retraining noise does not fail the build while a real regression does.
/// Measured for the pinned model: precision 1.000, recall 0.958, high-res
/// asset accuracy 1.000, model-only recall 0.958, model-only asset rejection
/// 0.981.
///
/// Two sets, because the classifier is two mechanisms. The pre-filter reads
/// alpha, palette and flat runs; the model reads texture, and never sees
/// alpha at all — `model_input` composites it away. So the rules settle most
/// synthetic assets before inference, and the shipped pipeline's numbers say
/// almost nothing about the model on that class. The model-only floors below
/// are what make a model regression visible.
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

    /// Photograph recall of the model alone, with the pre-filter bypassed.
    ///
    /// Guards the half of the classifier the shipped numbers cannot see. A
    /// model that stopped recognizing photographs would still score well
    /// above through the rules, because the rules never claim a photograph —
    /// they would simply defer, and every photograph would come back
    /// `Ambiguous`. This floor is what fails in that case.
    pub const MODEL_ONLY_PHOTOGRAPH_RECALL: f32 = 0.90;

    /// Fraction of synthetic assets the model alone keeps out of the
    /// photograph label.
    ///
    /// Not a precision floor: with the pre-filter bypassed the model sees
    /// icons and sprites whose defining signal is transparency, which its
    /// input does not carry, so some of them are genuinely undecidable for
    /// it. What must hold is that it does not confidently call them
    /// photographs.
    pub const MODEL_ONLY_ASSET_REJECTION: f32 = 0.90;
}

#[test]
fn eval_corpus_meets_the_recorded_precision_and_recall() {
    let mut triage = match Triage::new() {
        Ok(triage) => triage,
        Err(err) => {
            panic!("the pinned model must load for the eval harness to gate anything: {err}")
        }
    };

    let mut results = Vec::new();
    let mut overall = Confusion::default();

    for slice in Slice::ALL {
        let images: Vec<_> = (0..EVAL_PER_SLICE)
            .map(|index| sample(slice, eval_seed(index)).image)
            .collect();
        let scores = triage.score_batch(&images).expect("the model must score");

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
fn the_model_alone_still_separates_the_classes() {
    let mut triage = Triage::new().expect("the pinned model must load");

    let mut photograph_hits = 0;
    let mut photographs = 0;
    let mut asset_rejections = 0;
    let mut assets = 0;
    let mut table = String::from("\ntriage eval — model alone, pre-filter bypassed\n");

    for slice in Slice::ALL {
        let images: Vec<_> = (0..EVAL_PER_SLICE)
            .map(|index| sample(slice, eval_seed(index)).image)
            .collect();
        let scores = triage.score_batch_model_only(&images);
        let called_photograph = scores
            .iter()
            .filter(|score| score.label == TriageLabel::Photograph)
            .count();

        if slice.truth() == TriageLabel::Photograph {
            photographs += images.len();
            photograph_hits += called_photograph;
        } else {
            assets += images.len();
            asset_rejections += images.len() - called_photograph;
        }
        let _ = writeln!(
            table,
            "  {:<18} {:>3}/{} called photograph",
            slice.name(),
            called_photograph,
            images.len()
        );
    }

    let recall = ratio(photograph_hits, photographs);
    let rejection = ratio(asset_rejections, assets);
    let _ = writeln!(
        table,
        "\n  photograph recall {recall:.3}   asset rejection {rejection:.3}"
    );
    println!("{table}");

    assert!(
        recall >= floor::MODEL_ONLY_PHOTOGRAPH_RECALL,
        "the model alone recalls {recall:.3} of photographs, below the {:.2} this harness \
         records\n{table}",
        floor::MODEL_ONLY_PHOTOGRAPH_RECALL
    );
    assert!(
        rejection >= floor::MODEL_ONLY_ASSET_REJECTION,
        "the model alone keeps only {rejection:.3} of synthetic assets out of the photograph \
         label, below the {:.2} this harness records\n{table}",
        floor::MODEL_ONLY_ASSET_REJECTION
    );
}

#[test]
fn the_model_is_pinned_to_its_recorded_hash() {
    let triage = Triage::new().expect("the pinned model must verify");
    let identity = triage.model().expect("a real classifier names its model");
    assert_eq!(identity.version, argos_classify::MODEL_VERSION);
    assert_eq!(
        identity.sha256.to_string(),
        argos_classify::MODEL_SHA256_HEX,
        "the model that scored differs from the one the source tree pins"
    );
}
