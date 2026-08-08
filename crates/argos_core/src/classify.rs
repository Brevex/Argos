//! The triage port: scoring recovered images as photograph vs synthetic asset.
//!
//! Triage exists to save the examiner's time, never to decide what the
//! examiner sees. A [`Classifier`] hands back an opinion *about* an artifact;
//! it has no way to remove one, because it is only ever consulted after the
//! artifact is persisted, hashed and recorded, and its whole output is an
//! annotation (A-TRIAGE-NOT-VERDICT). The port has no filtering method to
//! misuse.

use std::convert::Infallible;
use std::error::Error;
use std::fmt;

use crate::artifact::Digest;

/// A decoded image, RGBA8 row-major, as handed to a [`Classifier`].
///
/// `Debug` prints dimensions only: decoded pixels are recovered content and
/// must never reach a log or a panic message (A-NO-CONTENT-IN-LOGS).
#[derive(Clone, PartialEq, Eq)]
pub struct PixelImage {
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

impl PixelImage {
    /// Bytes per pixel: red, green, blue, alpha.
    pub const BYTES_PER_PIXEL: usize = 4;

    /// An image of `width` by `height` pixels over `pixels`.
    ///
    /// # Panics
    ///
    /// Panics when `pixels` is not exactly `width * height * 4` bytes — the
    /// buffer and the dimensions come from the same decoder, so a mismatch is
    /// a bug in the adapter that produced them, not a property of the medium.
    #[must_use]
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(Self::BYTES_PER_PIXEL));
        assert_eq!(
            Some(pixels.len()),
            expected,
            "pixel buffer of {} bytes does not match {width}x{height} RGBA",
            pixels.len(),
        );
        Self {
            width,
            height,
            pixels: pixels.into_boxed_slice(),
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Number of pixels.
    #[must_use]
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The RGBA8 pixel data, row-major, four bytes per pixel.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.pixels
    }
}

impl fmt::Debug for PixelImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PixelImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &"<redacted>")
            .finish()
    }
}

/// What triage concluded an image most likely is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TriageLabel {
    /// A user photograph — what a recovery of personal images is looking for.
    Photograph,
    /// A synthetic asset: icon, sprite, UI chrome, web-cache graphic.
    SyntheticAsset,
    /// Neither signal was strong enough to say. Presented, never hidden.
    Ambiguous,
}

impl fmt::Display for TriageLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Photograph => "photograph",
            Self::SyntheticAsset => "synthetic-asset",
            Self::Ambiguous => "ambiguous",
        };
        f.write_str(name)
    }
}

/// Which part of a classifier produced a score.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScoredBy {
    /// The rule-based pre-filter: dimensions, alpha usage, palette statistics.
    Rules,
    /// Model inference.
    Model,
}

impl fmt::Display for ScoredBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Rules => "rules",
            Self::Model => "model",
        };
        f.write_str(name)
    }
}

/// A classifier's opinion of one image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriageScore {
    /// Probability in `0.0..=1.0` that the image is a user photograph.
    pub photograph: f32,
    /// The label the probability maps to under the classifier's documented
    /// thresholds.
    pub label: TriageLabel,
    /// What produced the score.
    pub scored_by: ScoredBy,
}

/// Identity of the model behind a classifier, for the manifest.
///
/// A scan records exactly which model scored it, so a result can be reproduced
/// with the same model and nothing else (A-MODEL-PINNED).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelIdentity {
    /// Human-readable model version.
    pub version: &'static str,
    /// SHA-256 of the model file.
    pub sha256: Digest,
}

/// Scores decoded images: photograph vs synthetic asset.
///
/// The engine consults a classifier only after artifacts are persisted and
/// hashed; everything a classifier returns is an annotation on an existing
/// record. `None` means "no opinion" and leaves the artifact unscored — it is
/// never a reason to drop one (A-TRIAGE-NOT-VERDICT).
pub trait Classifier {
    /// What scoring fails with.
    type Error: Error + Send + Sync + 'static;

    /// Identity of the model behind this classifier, when there is one.
    fn model(&self) -> Option<ModelIdentity>;

    /// Scores a batch of images, one answer per image in order.
    ///
    /// Batching exists because inference amortizes over it; a caller with one
    /// image uses [`Classifier::score`].
    ///
    /// # Errors
    ///
    /// Fails when the classifier itself breaks — a model tensor mismatch, not
    /// a property of any image. Per-image "no opinion" is `Ok(None)`.
    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error>;

    /// Scores one image.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Classifier::score_batch`].
    fn score(&mut self, image: &PixelImage) -> Result<Option<TriageScore>, Self::Error> {
        Ok(self
            .score_batch(std::slice::from_ref(image))?
            .into_iter()
            .next()
            .flatten())
    }
}

/// The null classifier: no model, no opinion, every artifact left as it is.
///
/// This is the adapter behind a scan whose triage is disabled — by the user,
/// or because a pinned model failed verification. The scan proceeds and
/// reports everything, unscored (A-MODEL-PINNED).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcceptAll;

impl AcceptAll {
    /// The null classifier.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Classifier for AcceptAll {
    type Error = Infallible;

    fn model(&self) -> Option<ModelIdentity> {
        None
    }

    fn score_batch(
        &mut self,
        images: &[PixelImage],
    ) -> Result<Vec<Option<TriageScore>>, Self::Error> {
        Ok(vec![None; images.len()])
    }
}
