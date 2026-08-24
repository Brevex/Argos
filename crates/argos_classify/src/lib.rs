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
//!
//! Dedup is the crate's other half: [`perceptual_hash`] reduces an image to a
//! 64-bit blockhash, and copies that were recompressed, resized or slightly
//! cropped land within [`NEAR_DUPLICATE_DISTANCE`] of each other. Collapsing
//! is an annotation too — a near-duplicate shares its group's score and is
//! never removed.

pub mod rank;
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

/// Grid edge of the hash: 8x8 blocks, one bit each.
const GRID: usize = 8;

/// Hamming distance at or under which two hashes are treated as the same
/// picture.
///
/// The blockhash literature operates at ~4% of the hash width for
/// re-encodings; 4% of 64 bits is 2.6. Rounding up to 3 keeps re-encoded and
/// resized copies together (the fixture suite checks this) while unrelated
/// images, which disagree on ~32 bits, stay far outside it. Collapsing is an
/// annotation either way: a near-duplicate shares its group's score, it is
/// never removed (A-TRIAGE-NOT-VERDICT).
pub const NEAR_DUPLICATE_DISTANCE: u32 = 3;

/// Samples each block is reduced from, along each axis.
///
/// A block mean converges long before every pixel in the block is read, so
/// the pass strides to at most this many rows and columns per block rather
/// than touching the whole image. It bounds the hash's cost by the hash's own
/// size instead of by the frame's: a 64-megapixel image costs what a
/// 256x256 one does. Sampling only ever perturbs a mean slightly, and the
/// dedup distance already tolerates far more than that.
const SAMPLES_PER_BLOCK: usize = 32;

/// Spread of block means below which an image has no structure to hash.
///
/// Bits are set by comparing each block mean against the median, so an image
/// whose blocks are all the same brightness produces a hash of zero whatever
/// colour it is: a blank scan page, a solid-black frame and a spacer graphic
/// would collapse into one group and share one score. That is not a
/// near-duplicate relationship — it is the hash having nothing to say — so
/// the spread is checked and a flat image gets no hash at all. Four levels
/// out of 256 is under the noise floor of any photograph.
const MIN_BLOCK_SPREAD: u64 = 4;

/// The 64-bit blockhash of an image.
///
/// `None` when the image has no spatial structure to hash (see
/// [`MIN_BLOCK_SPREAD`]); such an image is scored on its own rather than
/// grouped, because grouping it would attribute one image's score to another.
///
/// Transparent pixels are composited over white first, so a logo on a
/// transparent canvas hashes as it is displayed rather than as black.
#[must_use]
pub fn perceptual_hash(image: &PixelImage) -> Option<u64> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let rgba = image.rgba();
    let stride_y = height.div_ceil(GRID * SAMPLES_PER_BLOCK).max(1);
    let stride_x = width.div_ceil(GRID * SAMPLES_PER_BLOCK).max(1);

    let mut sums = [0_u64; GRID * GRID];
    let mut counts = [0_u64; GRID * GRID];
    let mut y = 0;
    while y < height {
        let cell_y = y * GRID / height;
        let row = &rgba[y * width * PixelImage::BYTES_PER_PIXEL..];
        let mut x = 0;
        while x < width {
            let cell_x = x * GRID / width;
            let px = &row[x * PixelImage::BYTES_PER_PIXEL..][..PixelImage::BYTES_PER_PIXEL];
            let luma = luma_over_white(px);
            let cell = cell_y * GRID + cell_x;
            sums[cell] += u64::from(luma);
            counts[cell] += 1;
            x += stride_x;
        }
        y += stride_y;
    }

    let mut means = [0_u64; GRID * GRID];
    for (mean, (sum, count)) in means.iter_mut().zip(sums.iter().zip(&counts)) {
        *mean = if *count == 0 { 0 } else { sum / count };
    }
    let mut sorted = means;
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    // A picture with no light and shade across it cannot be told from any
    // other such picture by this hash.
    let spread = sorted.last()?.saturating_sub(*sorted.first()?);
    if spread < MIN_BLOCK_SPREAD {
        return None;
    }

    let mut hash = 0_u64;
    for (bit, mean) in means.iter().enumerate() {
        if *mean > median {
            hash |= 1 << bit;
        }
    }
    Some(hash)
}

/// Number of differing bits between two hashes.
#[must_use]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// BT.601 luma of one RGBA pixel composited over a white background.
fn luma_over_white(px: &[u8]) -> u8 {
    let alpha = u32::from(px[3]);
    let over = |channel: u8| -> u32 { (u32::from(channel) * alpha + 255 * (255 - alpha)) / 255 };
    let (r, g, b) = (over(px[0]), over(px[1]), over(px[2]));
    // Integer BT.601: (77 R + 150 G + 29 B) / 256.
    ((77 * r + 150 * g + 29 * b) >> 8).min(255) as u8
}
