//! Derives the shipped label thresholds from the validation seed range.
//!
//! Thresholds must not be fitted to the eval harness's corpus, or the harness
//! stops being an independent gate (A-EVAL-GATED). So this runs the *shipped*
//! pipeline — pre-filter and pinned model together — over the validation
//! seeds the trainer used, prints the photograph-probability distribution per
//! class, and reports the threshold pair that separates them.

use argos_classify::Triage;
use argos_classify::fixture::{Slice, sample};
use argos_core::classify::TriageLabel;

/// Same range the trainer validates on: disjoint from both training and eval.
const VAL_SEED_BASE: u64 = 150_000;
const VAL_PER_SLICE: u64 = 30;

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    let at = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[at]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut triage = Triage::new()?;
    let mut photographs = Vec::new();
    let mut assets = Vec::new();

    // Model only: the pre-filter settles most assets, so the shipped
    // pipeline's asset probabilities are all exactly zero and say nothing
    // about where the model itself puts them. Thresholds have to separate the
    // classes the model produces, not the ones the rules already decided.
    for slice in Slice::ALL {
        let images: Vec<_> = (0..VAL_PER_SLICE)
            .map(|index| sample(slice, VAL_SEED_BASE + index * 7 + 1).image)
            .collect();
        let scores = triage.score_batch_model_only(&images);
        let into = if slice.truth() == TriageLabel::Photograph {
            &mut photographs
        } else {
            &mut assets
        };
        for score in scores {
            into.push(score.photograph);
        }
    }

    photographs.sort_by(f32::total_cmp);
    assets.sort_by(f32::total_cmp);

    println!("photograph probabilities ({} samples)", photographs.len());
    for fraction in [0.0, 0.05, 0.10, 0.25, 0.50, 1.0] {
        println!("  p{:>3.0}: {:.4}", fraction * 100.0, percentile(&photographs, fraction));
    }
    println!("asset probabilities ({} samples)", assets.len());
    for fraction in [0.0, 0.50, 0.75, 0.90, 0.95, 1.0] {
        println!("  p{:>3.0}: {:.4}", fraction * 100.0, percentile(&assets, fraction));
    }

    // The separating band: everything above the highest asset is safely a
    // photograph, everything below the lowest photograph safely an asset.
    let highest_asset = assets.last().copied().unwrap_or(0.0);
    let lowest_photo = photographs.first().copied().unwrap_or(1.0);
    println!("\nhighest asset {highest_asset:.4}, lowest photograph {lowest_photo:.4}");
    if lowest_photo > highest_asset {
        println!("the classes are separable on this set; any threshold in the band works");
    } else {
        println!("the classes overlap; a threshold pair trades recall for precision");
    }

    // Suggest a pair: photograph threshold above the 95th percentile of
    // assets, asset threshold below the 5th percentile of photographs.
    println!(
        "\nsuggested: PHOTOGRAPH_MIN {:.2}, ASSET_MAX {:.2}",
        percentile(&assets, 0.95).max(0.05),
        percentile(&photographs, 0.05).min(0.95)
    );
    Ok(())
}
