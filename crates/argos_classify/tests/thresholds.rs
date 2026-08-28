//! Where the rule thresholds come from.
//!
//! Every constant in [`argos_classify::rules`] is a number on a scale, and
//! a number chosen by eye silently reshapes what an examiner looks at first.
//! This prints the distribution of each feature over a **validation** range of
//! the corpus generator, and the constants come from its percentiles.
//!
//! The range matters. Seeds here are far from the ones `eval.rs` draws, so a
//! threshold is never fitted to the samples that gate it — fitting a threshold
//! to the corpus that grades it leaves nothing grading anything (A-EVAL-GATED).
//!
//! Ignored by default: it measures rather than asserts, and its output is read
//! by a person changing a constant. Run it with
//! `cargo test -p argos_classify --features test-util --test thresholds -- --ignored --nocapture`.

use argos_classify::fixture::{Slice, sample};
use argos_classify::rules;
use argos_core::ports::TriageLabel;

/// Samples per slice. Enough for a stable first percentile.
const PER_SLICE: usize = 200;

/// Seed of the sample at `index`.
///
/// Disjoint from `eval.rs`, which draws `index * 7 + 1` for indices under
/// forty — every seed below three hundred. Nothing here is graded by what it
/// was derived from.
fn validation_seed(index: u64) -> u64 {
    500_000 + index * 13
}

/// The value at `percentile` of a sorted-in-place sample.
fn percentile(values: &mut [f32], percentile: f64) -> f32 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if values.is_empty() {
        return f32::NAN;
    }
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an index into a few hundred samples"
    )]
    let at = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[at]
}

#[test]
#[ignore = "prints the distributions a threshold is derived from; not an assertion"]
fn feature_distributions_over_the_validation_range() {
    println!("\nfeature distributions — validation range, {PER_SLICE} samples per slice\n");
    println!(
        "  {:<18} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "slice", "texture", "p05", "p50", "p95", "luma p05"
    );

    let mut photograph_texture = Vec::new();
    let mut asset_texture = Vec::new();
    let mut photograph_flat = Vec::new();
    let mut asset_flat = Vec::new();
    let mut photograph_luma = Vec::new();
    let mut asset_luma = Vec::new();

    for slice in Slice::ALL {
        let mut texture = Vec::with_capacity(PER_SLICE);
        let mut luma = Vec::with_capacity(PER_SLICE);
        let mut flat = Vec::with_capacity(PER_SLICE);
        for index in 0..PER_SLICE {
            let image = sample(slice, validation_seed(index as u64)).image;
            let features = rules::features(&image);
            texture.push(features.textured_fraction);
            flat.push(features.flat_run_fraction);
            #[expect(
                clippy::cast_precision_loss,
                reason = "a luminance count is at most 256 and exact in f32"
            )]
            luma.push(features.distinct_luma as f32);
        }
        if slice.truth() == TriageLabel::Photograph {
            photograph_texture.extend(texture.iter().copied());
            photograph_flat.extend(flat.iter().copied());
            photograph_luma.extend(luma.iter().copied());
        } else {
            asset_texture.extend(texture.iter().copied());
            asset_flat.extend(flat.iter().copied());
            asset_luma.extend(luma.iter().copied());
        }
        println!(
            "  {:<18} {:>8} {:>8.3} {:>8.3} {:>8.3} {:>8.0}",
            slice.name(),
            "",
            percentile(&mut texture, 0.05),
            percentile(&mut texture, 0.50),
            percentile(&mut texture, 0.95),
            percentile(&mut luma, 0.05),
        );
        println!(
            "  {:<18} {:>8} {:>8.3} {:>8.3} {:>8.3}",
            "",
            "flat",
            percentile(&mut flat, 0.05),
            percentile(&mut flat, 0.50),
            percentile(&mut flat, 0.95),
        );
    }

    // What a threshold is actually set from: the two ends that have to be
    // separated, not the middle of either distribution. A rule placed on a
    // median is a rule that fires wrongly on the tail.
    println!("\n  feature        photographs (min .. p01)   assets (p99 .. max)");
    let gap = |name: &str, photo: &mut Vec<f32>, asset: &mut Vec<f32>| {
        println!(
            "  {name:<14} {:>8.4} {:>8.4}   {:>8.4} {:>8.4}",
            percentile(photo, 0.00),
            percentile(photo, 0.01),
            percentile(asset, 0.99),
            percentile(asset, 1.00),
        );
    };
    gap("texture", &mut photograph_texture, &mut asset_texture);
    println!("\n  feature        photographs (p99 .. max)   assets (min .. p01)");
    let inverted = |name: &str, photo: &mut Vec<f32>, asset: &mut Vec<f32>| {
        println!(
            "  {name:<14} {:>8.4} {:>8.4}   {:>8.4} {:>8.4}",
            percentile(photo, 0.99),
            percentile(photo, 1.00),
            percentile(asset, 0.00),
            percentile(asset, 0.01),
        );
    };
    inverted("flat-run", &mut photograph_flat, &mut asset_flat);
    inverted("luma", &mut photograph_luma, &mut asset_luma);
}

/// The feature values two synthetic PNGs actually produce, and what they decide.
///
/// A spot check beside the distributions above: when a rule change moves a
/// label, this says which feature moved with it.
#[test]
#[ignore = "prints the features of two fixtures; not an assertion"]
fn feature_values_of_two_synthetic_pngs() {
    for (name, bytes) in [
        ("png(64,64)", argos_carve::fixture::png(64, 64)),
        ("png(200,150)", argos_carve::fixture::png(200, 150)),
    ] {
        let image =
            argos_carve::decode::decode_rgba(argos_core::Format::Png, &bytes).expect("decode");
        let features = rules::features(&image);
        let screened = rules::screen(&features, image.pixel_count());
        eprintln!(
            "{name:<14} texture={:.4} flat={:.4} luma={} colors={} alpha={:.3} -> {} ({})",
            features.textured_fraction,
            features.flat_run_fraction,
            features.distinct_luma,
            features.distinct_colors,
            features.transparent_fraction,
            screened.label,
            screened.decided_by
        );
    }
}
