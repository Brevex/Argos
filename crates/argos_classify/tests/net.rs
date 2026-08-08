//! The forward pass, pinned to numbers.
//!
//! `argos_classify::net` writes the network out by hand so the evidence path
//! carries no inference runtime, and `tools/train_triage`'s `crosscheck`
//! binary is what proves that description still agrees with the `candle` one
//! used for training. But `tools/` is excluded from the workspace, so nothing
//! in `cargo test --workspace` would notice the hand-written pass changing
//! underneath the pinned weights.
//!
//! These are that guard. They are golden values, recorded from the pinned
//! model, and their whole job is to fail when the arithmetic moves: an edit to
//! the convolution, the pooling, the head or the input reduction changes them,
//! whatever it does to the accuracy numbers. Regenerating them is only correct
//! alongside a new pinned model and a passing `crosscheck`.

use argos_classify::Triage;
use argos_classify::fixture::{Slice, sample};
use argos_classify::net::{self, Activations, Net};

/// Tolerance on a golden probability. Floating-point summation order can move
/// the last bits; nothing else may.
const TOLERANCE: f32 = 1e-4;

/// Three samples per slice, and what `triage-cnn-v1` scores them.
const GOLDEN: [(Slice, [f32; 3]); 7] = [
    (Slice::Photograph, [0.955_448, 0.978_657, 0.964_672]),
    (
        Slice::PhotographThumbnail,
        [0.998_591, 0.994_141, 0.999_144],
    ),
    (
        Slice::PhotographGreyscale,
        [0.985_980, 0.998_760, 0.299_691],
    ),
    (Slice::Icon, [0.000_037, 0.000_562, 0.000_313]),
    (Slice::Sprite, [0.001_222, 0.013_359, 0.041_751]),
    (Slice::UiChrome, [0.000_015, 0.000_119, 0.000_834]),
    (Slice::HighResAsset, [0.202_061, 0.011_471, 0.180_726]),
];

/// Seed of the golden sample at index `i`. Its own range, so the golden
/// inputs are neither trained on nor part of the eval corpus.
fn golden_seed(index: u64) -> u64 {
    900_000 + index * 17 + 1
}

#[test]
fn the_forward_pass_still_produces_its_recorded_numbers() {
    let mut triage = Triage::new().expect("the pinned model must load");
    for (slice, expected) in GOLDEN {
        let images: Vec<_> = (0..3)
            .map(|index| sample(slice, golden_seed(index)).image)
            .collect();
        let scored = triage.score_batch_model_only(&images);
        for (got, want) in scored.iter().zip(expected) {
            assert!(
                (got.photograph - want).abs() < TOLERANCE,
                "{} scored {:.6}, recorded {want:.6} — the forward pass has changed under the \
                 pinned weights",
                slice.name(),
                got.photograph
            );
        }
    }
}

#[test]
fn the_input_reduction_is_standardized_per_channel() {
    // The network was trained on standardized inputs; feeding it raw ones
    // would silently halve its accuracy rather than fail. Each channel plane
    // must come out with zero mean and unit deviation.
    let image = sample(Slice::Photograph, golden_seed(0)).image;
    let input = net::model_input(&image);
    let plane = net::INPUT_EDGE * net::INPUT_EDGE;
    assert_eq!(input.len(), net::INPUT_CHANNELS * plane);
    #[expect(
        clippy::cast_precision_loss,
        reason = "the plane is a fixed 4096 values and exact in f32"
    )]
    let count = plane as f32;

    for channel in 0..net::INPUT_CHANNELS {
        let samples = &input[channel * plane..(channel + 1) * plane];
        let mean = samples.iter().sum::<f32>() / count;
        let variance = samples.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / count;
        assert!(
            mean.abs() < 1e-3,
            "channel {channel} mean is {mean}, not zero"
        );
        assert!(
            (variance.sqrt() - 1.0).abs() < 1e-3,
            "channel {channel} deviation is {}, not one",
            variance.sqrt()
        );
    }
}

#[test]
fn a_flat_image_reduces_to_zeros_rather_than_amplified_noise() {
    // Standardizing divides by the deviation, so a plane with no variation
    // would turn quantization dust into unit-scale noise if the guard were
    // missing.
    let flat = argos_core::classify::PixelImage::new(32, 32, vec![128; 32 * 32 * 4]);
    let input = net::model_input(&flat);
    assert!(
        input.iter().all(|value| *value == 0.0),
        "a single-colour image did not reduce to zeros"
    );
}

#[test]
fn the_weight_reader_refuses_a_file_it_cannot_trust() {
    // The pinned bytes are hash-verified before they reach the reader, so
    // this is about failing cleanly rather than about hostile input: a build
    // that shipped the wrong file should say so, not index out of bounds.
    assert!(Net::load(&[]).is_err(), "an empty file must be refused");
    assert!(
        Net::load(&[0; 4]).is_err(),
        "a file too short for its own length prefix must be refused"
    );

    // A length prefix that runs past the file.
    let mut truncated = u64::MAX.to_le_bytes().to_vec();
    truncated.extend_from_slice(b"{}");
    Net::load(&truncated).unwrap_err();

    // A well-formed prefix over a header that is not JSON.
    let mut not_json = 4_u64.to_le_bytes().to_vec();
    not_json.extend_from_slice(b"oops");
    Net::load(&not_json).unwrap_err();

    // Valid JSON, but no tensors: the network names what it needs, so this
    // must fail with a missing tensor rather than build something empty.
    let header = br#"{"__metadata__":{}}"#;
    let mut empty = u64::try_from(header.len())
        .expect("small")
        .to_le_bytes()
        .to_vec();
    empty.extend_from_slice(header);
    Net::load(&empty).unwrap_err();

    // A tensor whose declared byte range runs past the end of the file.
    let header = br#"{"conv1.bias":{"dtype":"F32","shape":[16],"data_offsets":[0,64]}}"#;
    let mut past_end = u64::try_from(header.len())
        .expect("small")
        .to_le_bytes()
        .to_vec();
    past_end.extend_from_slice(header);
    past_end.extend_from_slice(&[0; 8]);
    Net::load(&past_end).unwrap_err();

    // A tensor whose shape disagrees with the bytes behind it.
    let header = br#"{"conv1.bias":{"dtype":"F32","shape":[16],"data_offsets":[0,8]}}"#;
    let mut wrong_shape = u64::try_from(header.len())
        .expect("small")
        .to_le_bytes()
        .to_vec();
    wrong_shape.extend_from_slice(header);
    wrong_shape.extend_from_slice(&[0; 8]);
    Net::load(&wrong_shape).unwrap_err();
}

#[test]
fn a_network_loaded_twice_scores_identically() {
    // Loading is the only place the weights are interpreted, and the scratch
    // buffers are reused across images — a stale buffer would show up as the
    // second score of a batch differing from the first.
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/model/triage-v1.safetensors"
    ))
    .expect("the pinned model is in the source tree");
    let net = Net::load(&bytes).expect("the pinned model loads");

    let image = sample(Slice::Photograph, golden_seed(1)).image;
    let input = net::model_input(&image);
    let mut fresh = Activations::new();
    let first = net.photograph_probability(&input, &mut fresh);

    // Same buffers, after scoring something else entirely.
    let other = net::model_input(&sample(Slice::UiChrome, golden_seed(2)).image);
    let _ = net.photograph_probability(&other, &mut fresh);
    let again = net.photograph_probability(&input, &mut fresh);

    assert!(
        (first - again).abs() < f32::EPSILON,
        "reusing the scratch buffers changed the answer: {first} then {again}"
    );
}

#[test]
fn a_model_that_does_not_match_its_pinned_hash_is_refused() {
    // The contract this guards is A-MODEL-PINNED's: a model that fails
    // verification disables triage and is reported. What must not happen is
    // the tampered weights being used anyway.
    let mut bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/model/triage-v1.safetensors"
    ))
    .expect("the pinned model is in the source tree");
    Triage::from_bytes(&bytes).expect("the untouched file verifies");

    // Flip one bit of one weight. The file is still a structurally valid
    // safetensors that would load and score — only the hash gives it away.
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    let err = Triage::from_bytes(&bytes).expect_err("a tampered model must be refused");
    let message = err.to_string();
    assert!(
        message.contains("pin verification"),
        "the failure must name the reason, got: {message}"
    );
    assert!(
        message.contains(argos_classify::MODEL_SHA256_HEX),
        "the failure must state the hash that was expected, got: {message}"
    );
}
