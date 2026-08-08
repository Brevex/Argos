//! What the perceptual hash must collapse and what it must keep apart.
//!
//! Dedup groups near-duplicates so one inference speaks for a group. A hash
//! that collapses unrelated pictures would give them one label; a hash that
//! separates re-encodings would pay for the same inference twice. Both are
//! measured here on generated images.

use argos_classify::fixture::{Slice, sample};
use argos_classify::phash::{NEAR_DUPLICATE_DISTANCE, hamming, perceptual_hash};
use argos_core::classify::PixelImage;

/// The hash of an image the suite expects to be hashable.
fn hash_of(image: &PixelImage) -> u64 {
    perceptual_hash(image).expect("a textured image must have a hash")
}

/// Rescales an image by nearest-neighbour sampling — a resized copy, the way
/// a thumbnail cache stores one.
fn resized(image: &PixelImage, width: u32, height: u32) -> PixelImage {
    let (source_w, source_h) = (image.width() as usize, image.height() as usize);
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let source_y = y * source_h / height as usize;
        for x in 0..width as usize {
            let source_x = x * source_w / width as usize;
            let at = (source_y * source_w + source_x) * 4;
            out.extend_from_slice(&image.rgba()[at..at + 4]);
        }
    }
    PixelImage::new(width, height, out)
}

/// Adds a small amount of noise to every pixel — what re-encoding does.
fn jittered(image: &PixelImage, amplitude: u32) -> PixelImage {
    let mut rgba = image.rgba().to_vec();
    let mut state = 0x9E37_79B9_u32;
    let span = 2 * amplitude + 1;
    for (index, byte) in rgba.iter_mut().enumerate() {
        // Leave alpha alone: re-encoding perturbs colour, not opacity.
        if index % 4 == 3 {
            continue;
        }
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let delta = i64::from(state % span) - i64::from(amplitude);
        *byte = u8::try_from((i64::from(*byte) + delta).clamp(0, 255)).unwrap_or(0);
    }
    PixelImage::new(image.width(), image.height(), rgba)
}

#[test]
fn a_resized_copy_stays_within_the_near_duplicate_distance() {
    for index in 0..10 {
        let original = sample(Slice::Photograph, index * 7 + 1).image;
        let half = resized(&original, original.width() / 2, original.height() / 2);
        let distance = hamming(hash_of(&original), hash_of(&half));
        assert!(
            distance <= NEAR_DUPLICATE_DISTANCE,
            "a half-size copy landed {distance} bits away, past the {NEAR_DUPLICATE_DISTANCE} \
             this dedup treats as the same picture"
        );
    }
}

#[test]
fn a_re_encoded_copy_stays_within_the_near_duplicate_distance() {
    for index in 0..10 {
        let original = sample(Slice::Photograph, index * 11 + 3).image;
        let noisy = jittered(&original, 6);
        let distance = hamming(hash_of(&original), hash_of(&noisy));
        assert!(
            distance <= NEAR_DUPLICATE_DISTANCE,
            "a re-encoded copy landed {distance} bits away, past the {NEAR_DUPLICATE_DISTANCE} \
             this dedup treats as the same picture"
        );
    }
}

#[test]
fn unrelated_photographs_do_not_collapse() {
    let mut collisions = 0;
    let mut pairs = 0;
    for a in 0..12_u64 {
        for b in (a + 1)..12 {
            let left = hash_of(&sample(Slice::Photograph, a * 7 + 1).image);
            let right = hash_of(&sample(Slice::Photograph, b * 7 + 1).image);
            pairs += 1;
            if hamming(left, right) <= NEAR_DUPLICATE_DISTANCE {
                collisions += 1;
            }
        }
    }
    assert_eq!(
        collisions, 0,
        "{collisions} of {pairs} unrelated photograph pairs collapsed into one group; \
         a group shares one label, so a collision mislabels a real image"
    );
}

#[test]
fn a_photograph_and_an_asset_never_collapse() {
    for index in 0..10_u64 {
        let photo = hash_of(&sample(Slice::Photograph, index * 7 + 1).image);
        let asset = hash_of(&sample(Slice::HighResAsset, index * 7 + 1).image);
        assert!(
            hamming(photo, asset) > NEAR_DUPLICATE_DISTANCE,
            "a photograph and a synthetic asset collapsed into one group"
        );
    }
}

#[test]
fn an_image_with_no_structure_gets_no_hash() {
    // Bits come from comparing block means against their median, so every
    // flat image hashes to zero whatever colour it is. A blank scan page, a
    // solid black frame and a spacer graphic would all land in one group and
    // share one score — which would put a label on an artifact nothing ever
    // looked at. They must get no hash instead.
    for level in [0_u8, 64, 128, 255] {
        let flat = PixelImage::new(64, 64, vec![level; 64 * 64 * 4]);
        assert_eq!(
            perceptual_hash(&flat),
            None,
            "a solid image at level {level} was given a hash"
        );
    }

    // Different solid colours, which a hash would also collapse.
    let mut red = Vec::new();
    for _ in 0..64 * 64 {
        red.extend_from_slice(&[200, 20, 20, 255]);
    }
    assert_eq!(perceptual_hash(&PixelImage::new(64, 64, red)), None);
}

#[test]
fn a_faint_but_real_gradient_is_still_hashed() {
    // The flatness guard must not swallow low-contrast photographs; only
    // images with no spatial structure at all.
    let mut rgba = Vec::new();
    for y in 0..64_u32 {
        for _ in 0..64_u32 {
            let level = u8::try_from(100 + y / 4).unwrap_or(255);
            rgba.extend_from_slice(&[level, level, level, 255]);
        }
    }
    assert!(
        perceptual_hash(&PixelImage::new(64, 64, rgba)).is_some(),
        "a real gradient was mistaken for a structureless image"
    );
}
