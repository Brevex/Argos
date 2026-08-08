//! ML triage of recovered images: photograph vs synthetic asset, plus
//! perceptual-hash dedup.
//!
//! [`Triage`] is the [`Classifier`] adapter: a rule-based pre-filter settles
//! the obvious synthetic assets, and a small pinned CNN scores the rest.
//! Inference is plain Rust arithmetic over weights compiled in from the
//! source tree and verified against a SHA-256 constant — no runtime, no
//! network, nothing this crate does not own (A-INFERENCE-PURE-RUST,
//! A-MODEL-PINNED). See [`net`] for why the forward pass is written out
//! rather than delegated, and what keeps it honest.
//!
//! Everything this crate produces is an annotation on an already-persisted
//! artifact; there is no API to filter, skip or delete one
//! (A-TRIAGE-NOT-VERDICT).

pub mod net;
pub mod phash;
pub mod prefilter;

mod error;
mod model;
mod weights;

#[cfg(feature = "test-util")]
pub mod fixture;

use std::convert::Infallible;

use argos_core::classify::{
    Classifier, ModelIdentity, PixelImage, ScoredBy, TriageLabel, TriageScore,
};

pub use error::TriageError;
pub use model::{MODEL_SHA256_HEX, MODEL_VERSION};
pub use weights::WeightError;

/// Photograph probability at or above which the label is `Photograph`.
///
/// Set below the 5th percentile of the model's photograph scores on the
/// training tool's *validation* range (0.594), so at least nineteen in twenty
/// photographs that reach the model are labeled as such. Derived there, not
/// on the eval corpus, which would stop the eval harness from gating anything
/// (A-EVAL-GATED). Re-derive with `cargo run --bin thresholds` in
/// `tools/train_triage` whenever the model changes.
pub const PHOTOGRAPH_MIN_PROBABILITY: f32 = 0.55;

/// Photograph probability at or below which the label is `SyntheticAsset`.
///
/// Set just above the 90th percentile of the model's synthetic-asset scores
/// on the same validation range (0.091), so nine in ten assets that survive
/// the pre-filter still land on the right label. The band between this and
/// [`PHOTOGRAPH_MIN_PROBABILITY`] is reported as
/// [`TriageLabel::Ambiguous`]: for a tool whose output an examiner acts on,
/// an honest "unclear" ranks better than a confident guess, and the artifact
/// is fully reported either way.
pub const ASSET_MAX_PROBABILITY: f32 = 0.10;

/// The ML triage classifier: pre-filter first, pinned CNN for the rest.
#[derive(Debug)]
pub struct Triage {
    net: net::Net,
    identity: ModelIdentity,
    /// Reused convolution buffers, so scoring a batch allocates once rather
    /// than once per image (`M-MEM-REUSE`).
    scratch: net::Activations,
}

impl Triage {
    /// Verifies the pinned model and builds the classifier over it.
    ///
    /// # Errors
    ///
    /// Fails when the compiled-in weights do not hash to
    /// [`MODEL_SHA256_HEX`] or do not load as the expected network. The
    /// caller is expected to proceed without triage and report why — never to
    /// abort the scan (A-MODEL-PINNED).
    pub fn new() -> Result<Self, TriageError> {
        let (net, sha256) = model::load_pinned()?;
        Ok(Self {
            net,
            identity: ModelIdentity {
                version: MODEL_VERSION,
                sha256,
            },
            scratch: net::Activations::new(),
        })
    }

    /// Builds a classifier over `bytes` instead of the compiled-in weights,
    /// applying the same pin verification.
    ///
    /// The only way to reach the verification-failed path from a test: the
    /// shipped model is `include_bytes!`d and hash-checked, so nothing else
    /// can make [`Triage::new`] fail without editing the source tree. That
    /// path is a contract — a model that does not verify disables triage and
    /// is reported, and never fails a scan (A-MODEL-PINNED) — and a contract
    /// with no test is one that stops holding quietly.
    ///
    /// # Errors
    ///
    /// Fails when `bytes` does not hash to [`MODEL_SHA256_HEX`] or does not
    /// load as the expected network.
    #[cfg(feature = "test-util")]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TriageError> {
        let (net, sha256) = model::verify_and_load(bytes)?;
        Ok(Self {
            net,
            identity: ModelIdentity {
                version: MODEL_VERSION,
                sha256,
            },
            scratch: net::Activations::new(),
        })
    }

    /// Scores a batch with the model alone, bypassing the pre-filter.
    ///
    /// The pre-filter settles most synthetic assets before inference, which
    /// means the shipped pipeline's numbers say almost nothing about the
    /// model's own behaviour on that class. The eval harness needs both, or a
    /// model regression would hide behind the rules (A-EVAL-GATED). Gated
    /// behind `test-util` because a production caller wanting inference
    /// without the pre-filter is a caller paying for work the rules already
    /// did (`M-TEST-UTIL`).
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn score_batch_model_only(&mut self, images: &[PixelImage]) -> Vec<TriageScore> {
        images
            .iter()
            .map(|image| {
                let photograph = self.infer_one(image);
                TriageScore {
                    photograph,
                    label: Self::label_of(photograph),
                    scored_by: ScoredBy::Model,
                }
            })
            .collect()
    }

    /// Runs the model over one image, returning its photograph probability.
    fn infer_one(&mut self, image: &PixelImage) -> f32 {
        let input = net::model_input(image);
        self.net.photograph_probability(&input, &mut self.scratch)
    }

    /// Maps a photograph probability to a label under the documented
    /// thresholds.
    #[must_use]
    pub fn label_of(photograph: f32) -> TriageLabel {
        if photograph >= PHOTOGRAPH_MIN_PROBABILITY {
            TriageLabel::Photograph
        } else if photograph <= ASSET_MAX_PROBABILITY {
            TriageLabel::SyntheticAsset
        } else {
            TriageLabel::Ambiguous
        }
    }
}

impl Classifier for Triage {
    /// Scoring cannot fail once the model has loaded: the forward pass is
    /// arithmetic over a fixed-size input, with no fallible step in it. Every
    /// way triage can go wrong is a setup failure, reported by
    /// [`Triage::new`].
    type Error = Infallible;

    fn model(&self) -> Option<ModelIdentity> {
        Some(self.identity)
    }

    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        let mut scores: Vec<Option<TriageScore>> = vec![None; images.len()];

        // The pre-filter settles the obvious cases so only ambiguous images
        // pay for inference.
        let mut pending = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let features = prefilter::features(image);
            match prefilter::screen(&features, image.pixel_count()) {
                Some(score) => scores[index] = Some(score),
                None => pending.push(index),
            }
        }
        for index in pending {
            let photograph = self.infer_one(&images[index]);
            scores[index] = Some(TriageScore {
                photograph,
                label: Self::label_of(photograph),
                scored_by: ScoredBy::Model,
            });
        }
        Ok(scores)
    }
}
