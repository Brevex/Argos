//! The rule-based pre-filter: cheap statistics that settle the obvious cases
//! so the model only sees ambiguous ones.
//!
//! Every rule here points one way — *synthetic asset* — and only on signals a
//! photograph essentially cannot produce: meaningful transparency, a palette a
//! sensor could not emit, or long runs of byte-identical pixels. Sensor noise
//! and JPEG quantization make real photographs fail all three by wide margins,
//! which is what makes the short-circuit safe. Nothing here is a verdict about
//! recovery: a rule labels an artifact, it never touches one
//! (A-TRIAGE-NOT-VERDICT).

use argos_core::classify::{PixelImage, ScoredBy, TriageLabel, TriageScore};

/// Fraction of non-opaque pixels above which an image is called an asset.
///
/// JPEG has no alpha channel at all, and photographs stored as PNG are opaque
/// scans or screenshots; transparency is an authoring feature. The 2% floor
/// tolerates a decoder emitting a stray translucent edge without letting a
/// mostly-transparent icon through. On the eval corpus every photograph
/// measures 0% and every alpha-using asset measures far above 2%.
pub const TRANSPARENT_ASSET_FRACTION: f32 = 0.02;

/// Distinct-color ceiling under which a large image may be called an asset.
///
/// A 256-entry palette is the historical PNG-8 authoring limit, and flat
/// design stays well under it even at wallpaper resolutions, while a
/// photograph's noise floor alone produces thousands of distinct colors.
///
/// Never sufficient on its own — see [`PALETTE_LUMA_ASSET_MAX`].
pub const PALETTE_ASSET_MAX: u32 = 256;

/// Distinct-luminance ceiling the palette rule additionally requires.
///
/// The colour count is quantized to five bits per channel, so a *greyscale*
/// image can never exceed 32 distinct colours however rich it is — which
/// would make every black-and-white photograph fail the colour test by
/// construction, and a monochrome photograph is exactly the kind of evidence
/// that must not be short-circuited away from the model. Distinct 8-bit luma
/// levels are counted unquantized alongside it, and both have to be small
/// before the rule fires.
///
/// Measured over the eval corpus: the least varied photograph of any slice,
/// greyscale included, uses 71 distinct luma levels; the most varied
/// synthetic asset uses 47. The threshold sits in that gap, nearer the asset
/// side, because firing wrongly on a photograph costs the model its chance to
/// see it while firing wrongly on an asset costs one inference.
pub const PALETTE_LUMA_ASSET_MAX: u32 = 48;

/// Pixels an image needs before the palette rule may fire.
///
/// In a small image a small palette is not evidence — a 48x48 icon and a
/// 48x48 photo crop can both fit hundreds of colors — so the rule only speaks
/// when the image is large enough that a photograph would be forced into
/// thousands of distinct values.
pub const PALETTE_RULE_MIN_PIXELS: u64 = 128 * 128;

/// Fraction of horizontally byte-identical neighbour pixels above which an
/// image is called an asset.
///
/// Vector art and UI chrome are dominated by exact flat runs; photographs are
/// not, because sensor noise and JPEG ringing perturb even flat sky. Measured
/// on the eval corpus: photographs sit under 0.15 (JPEG-smoothed gradients
/// included), synthetic flat-design assets above 0.7. The threshold sits at
/// the midpoint with margin toward not firing, because a wrong "asset" label
/// on a photograph costs an examiner attention while a wrong pass merely
/// costs one inference.
pub const FLAT_RUN_ASSET_FRACTION: f32 = 0.55;

/// Sample budget for the single statistics pass.
///
/// Statistics converge long before this; the cap keeps the pass linear in
/// image size only up to a point, so a 64-megapixel frame does not pay 64
/// million hash insertions for a palette estimate (A-BOUNDED-ALLOC).
const MAX_SAMPLES: u64 = 1 << 20;

/// Cheap whole-image statistics, one pass, no allocation proportional to the
/// image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Features {
    /// Fraction of sampled pixels with alpha below opaque.
    pub transparent_fraction: f32,
    /// Distinct sampled RGB values, quantized to five bits per channel.
    pub distinct_colors: u32,
    /// Whether the distinct-color count hit its cap and stopped counting.
    pub palette_saturated: bool,
    /// Distinct sampled 8-bit luminance levels, unquantized. Separates a
    /// greyscale photograph, which fills most of them, from flat design,
    /// which uses a handful.
    pub distinct_luma: u32,
    /// Fraction of sampled pixels byte-identical to their left neighbour.
    pub flat_run_fraction: f32,
    /// Pixels actually sampled.
    pub samples: u64,
}

/// Computes [`Features`] by sampling at most [`MAX_SAMPLES`] pixels
/// stride-uniformly.
///
/// Measured at 1.00 ms for a 0.3-megapixel frame, against 2.61 ms for one
/// inference — so the rules pay for themselves whenever they settle an image
/// the model would otherwise have scored. Counting luminance levels alongside
/// colours costs about a third of that pass; it is what keeps greyscale
/// photographs out of the asset label, which is worth more than the time.
#[must_use]
pub fn features(image: &PixelImage) -> Features {
    let total = image.pixel_count();
    let stride = usize::try_from(total.div_ceil(MAX_SAMPLES))
        .unwrap_or(usize::MAX)
        .max(1);
    let rgba = image.rgba();
    let width = image.width() as usize;

    // Distinct colors via a fixed-size occupancy table over a 15-bit
    // quantization (5 bits per channel): 32768 slots, one bit each. Exact
    // enough to separate "hundreds" from "thousands", and its memory does not
    // depend on the image (A-BOUNDED-ALLOC). Quantization only ever merges
    // colors, so a count over the table never exceeds the true count and the
    // asset rule can only under-fire, never over-fire.
    let mut seen = [0_u64; 32768 / 64];
    let mut distinct = 0_u32;
    let mut luma_seen = [0_u64; 256 / 64];
    let mut distinct_luma = 0_u32;
    let mut transparent = 0_u64;
    let mut flat = 0_u64;
    let mut flat_eligible = 0_u64;
    let mut samples = 0_u64;

    let mut index = 0_usize;
    let pixel_count = usize::try_from(total).unwrap_or(usize::MAX);
    while index < pixel_count {
        let at = index * PixelImage::BYTES_PER_PIXEL;
        let px = &rgba[at..at + PixelImage::BYTES_PER_PIXEL];
        samples += 1;
        if px[3] != u8::MAX {
            transparent += 1;
        }
        let key = (usize::from(px[0] >> 3) << 10)
            | (usize::from(px[1] >> 3) << 5)
            | usize::from(px[2] >> 3);
        let (slot, bit) = (key / 64, key % 64);
        if seen[slot] & (1 << bit) == 0 {
            seen[slot] |= 1 << bit;
            distinct += 1;
        }
        // Integer BT.601 luma, kept at full 8-bit resolution.
        let luma = ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8)
            .min(255) as usize;
        let (luma_slot, luma_bit) = (luma / 64, luma % 64);
        if luma_seen[luma_slot] & (1 << luma_bit) == 0 {
            luma_seen[luma_slot] |= 1 << luma_bit;
            distinct_luma += 1;
        }
        // Compare with the pixel immediately to the left, when the sample is
        // not on the left edge.
        if !index.is_multiple_of(width) {
            flat_eligible += 1;
            let left = &rgba[at - PixelImage::BYTES_PER_PIXEL..at];
            if left == px {
                flat += 1;
            }
        }
        index += stride;
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "fractions over sample counts need no more than f32 precision"
    )]
    Features {
        transparent_fraction: if samples == 0 {
            0.0
        } else {
            transparent as f32 / samples as f32
        },
        distinct_colors: distinct,
        palette_saturated: distinct as usize >= 32768,
        distinct_luma,
        flat_run_fraction: if flat_eligible == 0 {
            0.0
        } else {
            flat as f32 / flat_eligible as f32
        },
        samples,
    }
}

/// Applies the rules to `features` for an image of `pixel_count` pixels.
///
/// `Some` is a confident *synthetic asset* verdict; `None` passes the image on
/// to inference. The rules never declare a photograph: a photograph verdict
/// without the model would rest on the absence of signals, and absence is not
/// evidence.
#[must_use]
pub fn screen(features: &Features, pixel_count: u64) -> Option<TriageScore> {
    let palette_poor = pixel_count >= PALETTE_RULE_MIN_PIXELS
        && !features.palette_saturated
        && features.distinct_colors <= PALETTE_ASSET_MAX
        && features.distinct_luma <= PALETTE_LUMA_ASSET_MAX;
    let asset = (features.transparent_fraction > TRANSPARENT_ASSET_FRACTION)
        || palette_poor
        || (features.flat_run_fraction > FLAT_RUN_ASSET_FRACTION);
    asset.then_some(TriageScore {
        photograph: 0.0,
        label: TriageLabel::SyntheticAsset,
        scored_by: ScoredBy::Rules,
    })
}
