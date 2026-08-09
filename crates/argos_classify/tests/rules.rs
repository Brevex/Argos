//! What the rules may claim, and where each direction has to stay silent.
//!
//! The two errors are not equally expensive. A run may be asked to leave
//! artifacts labelled `SyntheticAsset` unwritten, so a photograph called an
//! asset is a photograph missing from the output; an asset called a photograph
//! is a file on disk. Every test here is about that asymmetry — and about the
//! rule that keeps it honest: an image satisfying neither direction is
//! `Ambiguous`, which is reported and never treated as an asset.

use argos_classify::fixture::{Slice, sample};
use argos_classify::rules::{self, features, screen};
use argos_core::classify::{Decision, PixelImage, TriageLabel};

/// The label one image gets.
fn verdict(image: &PixelImage) -> TriageLabel {
    screen(&features(image), image.pixel_count()).label
}

#[test]
fn no_photograph_in_the_corpus_is_called_an_asset() {
    // The expensive direction. An artifact labelled asset is one a run may be
    // told not to write, so a wrong label here is evidence that never reaches
    // the output directory.
    for slice in [
        Slice::Photograph,
        Slice::PhotographThumbnail,
        Slice::PhotographGreyscale,
    ] {
        for index in 0..40 {
            let labeled = sample(slice, index * 7 + 1);
            assert_ne!(
                verdict(&labeled.image),
                TriageLabel::SyntheticAsset,
                "a {} was called a synthetic asset",
                slice.name()
            );
        }
    }
}

#[test]
fn no_asset_in_the_corpus_is_called_a_photograph() {
    for slice in [
        Slice::Icon,
        Slice::Sprite,
        Slice::UiChrome,
        Slice::HighResAsset,
    ] {
        for index in 0..40 {
            let labeled = sample(slice, index * 7 + 1);
            assert_ne!(
                verdict(&labeled.image),
                TriageLabel::Photograph,
                "a {} was called a photograph",
                slice.name()
            );
        }
    }
}

#[test]
fn a_photograph_label_always_rests_on_positive_evidence() {
    // Structural, not statistical. Every asset rule works by absence — no
    // transparency, no small palette — and a photograph verdict that also
    // rested on absence would rest on nothing. The only rule that may return
    // `Photograph` is the one that measured the sensor's own high-frequency
    // floor, and this asserts that no other path can.
    for slice in Slice::ALL {
        for index in 0..20 {
            let image = sample(slice, index * 13 + 3).image;
            let score = screen(&features(&image), image.pixel_count());
            if score.label == TriageLabel::Photograph {
                assert_eq!(score.decided_by, Decision::SensorTexture);
            }
            if score.label == TriageLabel::Ambiguous {
                assert_eq!(score.decided_by, Decision::Inconclusive);
            }
        }
    }
}

#[test]
fn an_image_with_no_texture_is_never_called_a_photograph() {
    // A flat fill has no high-frequency content at all, so it fails the one
    // rule that can claim a photograph however else it looks.
    let flat = PixelImage::new(64, 64, vec![200; 64 * 64 * 4]);
    let flat_features = features(&flat);
    assert!(
        flat_features.textured_fraction < f32::EPSILON,
        "a single-colour image has no high-frequency content at all, got {}",
        flat_features.textured_fraction
    );
    assert!(
        flat_features.flat_run_fraction > rules::FLAT_RUN_ASSET_FRACTION,
        "a single-colour image must read as entirely flat, got {}",
        flat_features.flat_run_fraction
    );
    assert_eq!(verdict(&flat), TriageLabel::SyntheticAsset);
}

#[test]
fn a_photograph_carries_the_high_frequency_floor_a_fill_does_not() {
    let photo = sample(Slice::Photograph, 11).image;
    let photo_features = features(&photo);
    assert!(
        photo_features.textured_fraction >= rules::PHOTOGRAPH_MIN_TEXTURE,
        "a photograph must carry sensor texture, got {}",
        photo_features.textured_fraction
    );
    assert!(
        photo_features.flat_run_fraction <= rules::PHOTOGRAPH_MAX_FLAT_RUN,
        "a photograph must not read as flat, got {}",
        photo_features.flat_run_fraction
    );
    assert_eq!(verdict(&photo), TriageLabel::Photograph);
}

#[test]
fn transparency_settles_an_image_on_its_own() {
    // JPEG has no alpha at all and a photograph stored as PNG is an opaque
    // scan or screenshot; transparency is an authoring feature. Half the
    // pixels here are transparent, which no sensor produces.
    let mut pixels = vec![255_u8; 64 * 64 * 4];
    for (index, chunk) in pixels.chunks_exact_mut(4).enumerate() {
        chunk[3] = if index % 2 == 0 { 0 } else { 255 };
    }
    let image = PixelImage::new(64, 64, pixels);

    let score = screen(&features(&image), image.pixel_count());

    assert_eq!(score.label, TriageLabel::SyntheticAsset);
    assert_eq!(score.decided_by, Decision::Transparency);
}

#[test]
fn an_image_that_satisfies_neither_direction_is_reported_unclear() {
    // A smooth gradient: not flat enough to be an asset by its runs, not
    // textured enough to be a photograph. An honest "unclear" is what such an
    // image gets, and what keeps it out of the label a run may act on.
    let (width, height) = (256_u32, 256_u32);
    let mut pixels = Vec::with_capacity((width * height) as usize * 4);
    for y in 0..height {
        for x in 0..width {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "both coordinates are below 256 by construction"
            )]
            let (r, g) = (x as u8, y as u8);
            pixels.extend_from_slice(&[r, g, 128, 255]);
        }
    }
    let image = PixelImage::new(width, height, pixels);

    let score = screen(&features(&image), image.pixel_count());

    assert_eq!(score.label, TriageLabel::Ambiguous);
    assert_eq!(score.decided_by, Decision::Inconclusive);
}
