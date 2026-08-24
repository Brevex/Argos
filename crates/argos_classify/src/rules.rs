//! The rules that decide photograph or synthetic asset.
//!
//! Deterministic image statistics, one sampled pass, no model. What separates
//! the two classes is physical rather than semantic: a sensor and a JPEG
//! quantizer leave a high-frequency floor over every part of a frame, and
//! drawn art does not, because a designer's fill is exactly flat and a
//! designer's gradient is exactly smooth. That difference survives resolution,
//! subject and colour, which is why it does not need to be learned.
//!
//! Two directions, and they are not symmetric:
//!
//! - **Asset** rules fire on signals a photograph essentially cannot produce:
//!   meaningful transparency, a palette a sensor could not emit, long runs of
//!   byte-identical neighbours.
//! - **Photograph** rules need *positive* evidence — that high-frequency floor
//!   over a rich set of luminance levels. A photograph verdict resting on the
//!   absence of asset signals would be a verdict resting on absence, and
//!   absence is not evidence.
//!
//! Whatever satisfies neither is [`TriageLabel::Ambiguous`], which is reported
//! and never treated as an asset. For a tool whose output an examiner acts on,
//! an honest "unclear" ranks above a confident guess — and when a run is asked
//! to leave assets unwritten, `Ambiguous` is written.
//!
//! Nothing here is a verdict about recovery on its own: a rule labels an
//! artifact (A-TRIAGE-NOT-VERDICT). The one exception a run may opt into —
//! not writing what these rules call an asset — is the caller's, is off by
//! default, and still records every artifact it omits.

use argos_core::classify::{Decision, PixelImage, TriageLabel, TriageScore};

/// Fraction of non-opaque pixels above which an image is called an asset.
///
/// JPEG has no alpha channel at all, and photographs stored as PNG are opaque
/// scans or screenshots; transparency is an authoring feature. The 2% floor
/// tolerates a decoder emitting a stray translucent edge without letting a
/// mostly-transparent icon through. On the eval corpus every photograph
/// measures 0% and every alpha-using asset measures far above 2%.
pub(crate) const TRANSPARENT_ASSET_FRACTION: f32 = 0.02;

/// Distinct-color ceiling under which a large image may be called an asset.
///
/// A 256-entry palette is the historical PNG-8 authoring limit, and flat
/// design stays well under it even at wallpaper resolutions, while a
/// photograph's noise floor alone produces thousands of distinct colors.
///
/// Never sufficient on its own — see [`PALETTE_LUMA_ASSET_MAX`].
pub(crate) const PALETTE_ASSET_MAX: u32 = 256;

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
/// side, because the two errors do not cost the same: labelling a photograph
/// an asset pushes it down the ordering an examiner reads, while labelling an
/// asset a photograph only leaves it in the pile.
pub(crate) const PALETTE_LUMA_ASSET_MAX: u32 = 48;

/// Pixels an image needs before the palette rule may fire.
///
/// In a small image a small palette is not evidence — a 48x48 icon and a
/// 48x48 photo crop can both fit hundreds of colors — so the rule only speaks
/// when the image is large enough that a photograph would be forced into
/// thousands of distinct values.
pub(crate) const PALETTE_RULE_MIN_PIXELS: u64 = 128 * 128;

/// Fraction of horizontally byte-identical neighbour pixels above which an
/// image is called an asset.
///
/// Flat fills and drawn edges are made of exact repeats; sensor noise and JPEG
/// ringing perturb even a photograph's flat sky. Measured over the validation
/// range (`tests/thresholds.rs`, 1400 samples): the most repetitive photograph
/// of any slice reaches 0.144, the least repetitive synthetic asset 0.522.
///
/// The threshold sits in that gap and deliberately nearer the asset side. It
/// is the asset label that a run may act on by leaving an artifact unwritten,
/// so calling a photograph an asset costs evidence while missing an asset
/// costs a file on disk. When the two errors are that unequal, the threshold
/// belongs on the expensive one's side.
pub const FLAT_RUN_ASSET_FRACTION: f32 = 0.40;

/// Fraction of textured pixels a photograph verdict requires.
///
/// The positive evidence, and the only rule here that points at *photograph*.
/// Over the generated validation corpus the least textured photograph of any
/// slice measures 0.651; drawn art sits at 0.002 to 0.09 except for dithered
/// sprites, which reach 0.94 — and those are settled as assets by their flat
/// runs before this is consulted.
///
/// **This threshold is not calibrated against real media, and against real
/// media it is on the wrong side.** Measured over images recovered from a 1 TB
/// disk of ten years' use: a 1920x2560 camera frame scores 0.442, a 768x576
/// one 0.411, a 256x192 cache entry 0.292 — so the rule calls camera
/// originals `Ambiguous`, and in the same run 611 of 3,000 sampled images of
/// 300 pixels or less were called `Photograph`. The corpus in
/// `crate::fixture` draws its photographs; drawn noise is not sensor noise,
/// and the gap the generated numbers describe does not exist in the world
/// (`A-EVAL-GATED`).
///
/// Until it is recalibrated against a labelled corpus of real recoveries, no
/// decision may rest on the label this produces — see
/// `docs/defects/03-triage-miscalibrated.md`. Nothing in the engine does: what
/// is written is decided by
/// [`DEFAULT_MIN_LONG_SIDE`](../../argos_engine/config/constant.DEFAULT_MIN_LONG_SIDE.html),
/// and this only orders and labels (`A-TRIAGE-NOT-VERDICT`).
pub const PHOTOGRAPH_MIN_TEXTURE: f32 = 0.60;

/// Fraction of byte-identical neighbours a photograph verdict tolerates.
///
/// The same measurement as [`FLAT_RUN_ASSET_FRACTION`] read from the other
/// end: above the most repetitive photograph (0.144), far below the least
/// repetitive asset (0.522).
pub const PHOTOGRAPH_MAX_FLAT_RUN: f32 = 0.20;

/// Distinct luminance levels a photograph verdict requires.
///
/// A sensor fills most of the eight-bit scale even in a monochrome frame; flat
/// design uses a handful. Over the validation range the first percentile of
/// photographs is 66 and the most varied asset measured 47. The threshold sits
/// between them.
pub(crate) const PHOTOGRAPH_MIN_LUMA: u32 = 56;

/// Sample budget for the single statistics pass.
///
/// Statistics converge long before this; the cap keeps the pass linear in
/// image size only up to a point, so a 64-megapixel frame does not pay 64
/// million hash insertions for a palette estimate (A-BOUNDED-ALLOC).
const MAX_SAMPLES: u64 = 1 << 20;

/// Smallest horizontal second difference in luma counted as texture.
///
/// One level of the eight-bit scale is the quantizer's own step and says
/// nothing. Three is above it and still far below anything an eye would call
/// detail, so what this counts is the floor a sensor and a JPEG quantizer
/// leave behind rather than the picture drawn on top of it.
const TEXTURE_FLOOR: u32 = 3;

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
    /// Fraction of sampled pixels whose horizontal second difference in luma
    /// reaches [`TEXTURE_FLOOR`].
    ///
    /// The one feature here that points at *photograph*. Every other rule
    /// works by absence — no transparency, no small palette — and absence is
    /// not evidence. A sensor and a JPEG quantizer leave a high-frequency
    /// floor over the whole frame, including flat sky; drawn art does not,
    /// because a gradient a designer laid down is exactly smooth and a fill is
    /// exactly flat.
    pub textured_fraction: f32,
    /// Pixels actually sampled.
    pub samples: u64,
}

/// Computes [`Features`] by sampling at most [`MAX_SAMPLES`] pixels
/// stride-uniformly.
///
/// Measured at 1.00 ms for a 0.3-megapixel frame, which is what makes running
/// this over every artifact of a whole-disk recovery proportionate
/// (`M-HOTPATH`). Counting luminance levels alongside colours costs about a
/// third of that pass; it is what keeps greyscale
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
    let mut textured = 0_u64;
    let mut textured_eligible = 0_u64;
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
        let luma = luma_of(px) as usize;
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
            // With a right neighbour as well, the second difference across the
            // three is the local high-frequency content: zero on a flat fill
            // *and* on a perfectly linear gradient, non-zero wherever a sensor
            // or a quantizer has been.
            if !(index + 1).is_multiple_of(width) && index + 1 < pixel_count {
                textured_eligible += 1;
                let right =
                    &rgba[at + PixelImage::BYTES_PER_PIXEL..at + 2 * PixelImage::BYTES_PER_PIXEL];
                let second_difference = 2 * i32::from(luma_of(px))
                    - i32::from(luma_of(left))
                    - i32::from(luma_of(right));
                if second_difference.unsigned_abs() >= TEXTURE_FLOOR {
                    textured += 1;
                }
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
        textured_fraction: if textured_eligible == 0 {
            0.0
        } else {
            textured as f32 / textured_eligible as f32
        },
        samples,
    }
}

/// Integer BT.601 luma of one RGBA pixel, at full eight-bit resolution.
fn luma_of(px: &[u8]) -> u8 {
    // The weights sum to 256, so the shift already lands inside a byte. The
    // clamp is there so the conversion is total rather than resting on that
    // arithmetic staying true through an edit.
    let luma = (77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8;
    u8::try_from(luma.min(255)).unwrap_or(u8::MAX)
}

/// Applies the rules to `features` for an image of `pixel_count` pixels.
///
/// Asset rules are consulted first and photograph rules only after none of
/// them fired. The order is the point: a dithered sprite can carry as much
/// high-frequency content as a photograph, and what separates it is the flat
/// runs around the dithering — so the flat-run rule has to be asked before the
/// texture rule is.
#[must_use]
pub fn screen(features: &Features, pixel_count: u64) -> TriageScore {
    let label = |label, decided_by| TriageScore { label, decided_by };

    if features.transparent_fraction > TRANSPARENT_ASSET_FRACTION {
        return label(TriageLabel::SyntheticAsset, Decision::Transparency);
    }
    let palette_poor = pixel_count >= PALETTE_RULE_MIN_PIXELS
        && !features.palette_saturated
        && features.distinct_colors <= PALETTE_ASSET_MAX
        && features.distinct_luma <= PALETTE_LUMA_ASSET_MAX;
    if palette_poor {
        return label(TriageLabel::SyntheticAsset, Decision::Palette);
    }
    if features.flat_run_fraction > FLAT_RUN_ASSET_FRACTION {
        return label(TriageLabel::SyntheticAsset, Decision::FlatFill);
    }

    let photographic = features.textured_fraction >= PHOTOGRAPH_MIN_TEXTURE
        && features.flat_run_fraction <= PHOTOGRAPH_MAX_FLAT_RUN
        && features.distinct_luma >= PHOTOGRAPH_MIN_LUMA;
    if photographic {
        return label(TriageLabel::Photograph, Decision::SensorTexture);
    }

    label(TriageLabel::Ambiguous, Decision::Inconclusive)
}
