//! What the rule-based pre-filter may claim, and what it must stay silent
//! about.
//!
//! The pre-filter short-circuits inference, so a wrong "synthetic asset" here
//! is never corrected by the model. Every test below is about that asymmetry:
//! the rules must fire on assets a photograph cannot imitate, and must stay
//! silent on everything else.

use argos_classify::fixture::{Slice, sample};
use argos_classify::prefilter::{self, features, screen};
use argos_core::classify::{PixelImage, ScoredBy, TriageLabel};

/// The pre-filter's verdict for one image, or `None` when it defers.
fn verdict(image: &PixelImage) -> Option<TriageLabel> {
    screen(&features(image), image.pixel_count()).map(|score| score.label)
}

#[test]
fn no_photograph_in_the_corpus_is_short_circuited_as_an_asset() {
    // The rules run before the model and cannot be overruled by it, so a
    // photograph they claim is an asset is a photograph the model never sees.
    for slice in [
        Slice::Photograph,
        Slice::PhotographThumbnail,
        Slice::PhotographGreyscale,
    ] {
        for index in 0..40 {
            let labeled = sample(slice, index * 7 + 1);
            assert_eq!(
                verdict(&labeled.image),
                None,
                "the pre-filter claimed a {} was a synthetic asset; it must defer to the \
                 model on anything a photograph could produce",
                slice.name()
            );
        }
    }
}

#[test]
fn a_transparent_asset_is_settled_without_inference() {
    let mut fired = 0;
    for index in 0..40 {
        let labeled = sample(Slice::Icon, index * 7 + 1);
        if verdict(&labeled.image) == Some(TriageLabel::SyntheticAsset) {
            fired += 1;
        }
    }
    // Most icons in the corpus sit on a transparent canvas; the rest are the
    // opaque ones, which the model handles.
    assert!(
        fired >= 25,
        "the pre-filter settled only {fired} of 40 icons; transparency and flat runs \
         are the cheapest signals there are and should carry most of them"
    );
}

#[test]
fn the_rules_never_claim_a_photograph() {
    // Structural, not statistical: `screen` has exactly one verdict to give,
    // so no corpus change can turn it into a photograph oracle. A photograph
    // label requires the model.
    for slice in Slice::ALL {
        for index in 0..10 {
            let labeled = sample(slice, index * 13 + 3);
            let score = screen(&features(&labeled.image), labeled.image.pixel_count());
            if let Some(score) = score {
                assert_eq!(score.label, TriageLabel::SyntheticAsset);
                assert_eq!(score.scored_by, ScoredBy::Rules);
            }
        }
    }
}

#[test]
fn a_flat_fill_reads_as_flat_and_a_photograph_does_not() {
    let flat = PixelImage::new(64, 64, vec![200; 64 * 64 * 4]);
    let flat_features = features(&flat);
    assert!(
        flat_features.flat_run_fraction > prefilter::FLAT_RUN_ASSET_FRACTION,
        "a single-colour image must read as entirely flat, got {}",
        flat_features.flat_run_fraction
    );

    let photo = sample(Slice::Photograph, 11).image;
    let photo_features = features(&photo);
    assert!(
        photo_features.flat_run_fraction < prefilter::FLAT_RUN_ASSET_FRACTION,
        "a photograph must not read as flat, got {}",
        photo_features.flat_run_fraction
    );
    assert!(
        photo_features.distinct_colors > prefilter::PALETTE_ASSET_MAX,
        "sensor noise must put a photograph well past the palette ceiling, got {}",
        photo_features.distinct_colors
    );
}

#[test]
fn a_greyscale_photograph_is_not_mistaken_for_a_small_palette() {
    // The colour count is quantized to five bits per channel, so any grey
    // image has at most 32 distinct colours however rich it is. Counting
    // colours alone would therefore short-circuit every black-and-white
    // photograph as a synthetic asset — the model would never see one.
    for index in 0..20 {
        let image = sample(Slice::PhotographGreyscale, index * 7 + 1).image;
        let stats = features(&image);
        assert!(
            stats.distinct_colors <= prefilter::PALETTE_ASSET_MAX,
            "the premise of this test is that greyscale collapses the colour count"
        );
        assert!(
            stats.distinct_luma > prefilter::PALETTE_LUMA_ASSET_MAX,
            "a greyscale photograph must still be rich in luminance, got {}",
            stats.distinct_luma
        );
        assert_eq!(
            verdict(&image),
            None,
            "a greyscale photograph was short-circuited as a synthetic asset"
        );
    }
}

#[test]
fn flat_artwork_is_still_caught_when_it_is_grey() {
    // The luminance test must not disarm the palette rule generally: grey
    // flat design has few colours *and* few levels, and still reads as art.
    let mut canvas = vec![0_u8; 256 * 256 * 4];
    for (index, px) in canvas.chunks_exact_mut(4).enumerate() {
        // Four grey bands, hard edges, no grain.
        let level = [40_u8, 90, 160, 220][(index / (256 * 64)).min(3)];
        px.copy_from_slice(&[level, level, level, 255]);
    }
    let art = PixelImage::new(256, 256, canvas);
    let stats = features(&art);
    assert!(stats.distinct_luma <= prefilter::PALETTE_LUMA_ASSET_MAX);
    assert_eq!(verdict(&art), Some(TriageLabel::SyntheticAsset));
}

#[test]
fn statistics_are_bounded_regardless_of_image_size() {
    // The sampling cap means a large image costs no more than a modest one;
    // what matters is that the statistics still describe the picture.
    let big = sample(Slice::HighResAsset, 5).image;
    let stats = features(&big);
    assert!(
        stats.samples <= 1 << 20,
        "the pass sampled {} pixels, past its own budget",
        stats.samples
    );
    assert!(stats.samples > 0, "a non-empty image must be sampled");
}
