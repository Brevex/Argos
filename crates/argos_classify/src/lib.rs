//! Triage of recovered images: photograph vs synthetic asset, plus
//! perceptual-hash dedup.
//!
//! [`Triage`] is the [`Classifier`] adapter, and it is rules and arithmetic —
//! no model, no weights, no inference. What separates a photograph from an
//! icon is physical: a sensor and a JPEG quantizer leave a high-frequency
//! floor over a frame that drawn art does not have, and drawn art carries
//! transparency, small palettes and long runs of identical pixels that a
//! sensor cannot produce. Measuring those directly is cheaper than learning
//! them, reproducible without a pinned artifact, and explainable to whoever
//! has to act on the result — a label here comes with the property that
//! decided it (see [`rules`]).
//!
//! Everything this crate produces is an annotation on an already-persisted
//! artifact. A run may be asked to leave artifacts labelled `SyntheticAsset`
//! unwritten, but that is a decision the caller makes and states; nothing in
//! this crate filters, skips or deletes anything (A-TRIAGE-NOT-VERDICT).

pub mod phash;
pub mod rules;

#[cfg(feature = "test-util")]
pub mod fixture;

use std::convert::Infallible;

use argos_core::classify::{Classifier, ModelIdentity, PixelImage, TriageScore};

/// Version of the decision procedure, recorded in every manifest it labels.
///
/// Bumped whenever a rule or a threshold changes, because a label is only
/// reproducible against the procedure that produced it (A-MODEL-PINNED). It
/// replaces a model hash: there is no file to pin when the decision lives in
/// the source tree the binary was built from.
pub const RULES_VERSION: &str = "rules-v1";

/// Deterministic triage over decoded images.
///
/// Holds no state between images and cannot fail, which is why it is cheap
/// enough to run over every artifact of a whole-disk recovery: a scan that
/// labelled twenty-three thousand images used to spend a minute of inference
/// doing it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Triage;

impl Triage {
    /// A classifier over the compiled-in rules.
    ///
    /// Infallible, and its signature says so. There is nothing to load and
    /// nothing to verify: a `Result` here would be a failure path no caller
    /// could ever reach and no test could ever cover.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The label the rules give one image.
    #[must_use]
    pub fn decide(image: &PixelImage) -> TriageScore {
        rules::screen(&rules::features(image), image.pixel_count())
    }
}

impl Classifier for Triage {
    /// Deciding cannot fail: every rule is arithmetic over statistics of an
    /// image already decoded, with no fallible step in it.
    type Error = Infallible;

    fn model(&self) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            version: RULES_VERSION,
        })
    }

    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        Ok(images
            .iter()
            .map(|image| Some(Self::decide(image)))
            .collect())
    }
}
