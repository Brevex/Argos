//! Proves the two descriptions of the network still agree.
//!
//! `argos_classify::net` writes the forward pass out by hand so the shipped
//! crate needs no inference runtime; `candle_net` describes the same
//! architecture in `candle` for training. Two descriptions of one network can
//! drift, and a drift would show up as an accuracy the eval harness cannot
//! explain — or worse, not show up at all. This runs both over the pinned
//! weights and the same inputs, and fails if any probability differs by more
//! than float noise.
//!
//! Run it after every training run, before pinning the new hash.

use argos_classify::fixture::{Slice, sample};
use argos_classify::net::{self, Activations, Net};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use train_triage::candle_net::CandleNet;

/// Largest probability difference accepted between the two implementations.
///
/// They sum the same products in different orders, so they cannot be
/// bit-identical in f32; anything past this is a structural difference, not
/// rounding.
const TOLERANCE: f32 = 1e-4;

const INPUT_LEN: usize = net::INPUT_CHANNELS * net::INPUT_EDGE * net::INPUT_EDGE;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../../crates/argos_classify/model/triage-v1.safetensors".to_owned());
    let bytes = std::fs::read(&path)?;
    println!("checking {path} ({} bytes)", bytes.len());

    let hand = Net::load(&bytes)?;
    let device = Device::Cpu;
    let vars = VarBuilder::from_buffered_safetensors(bytes.clone(), DType::F32, &device)?;
    let candle = CandleNet::build(&vars)?;

    // A spread of inputs across every slice, so the comparison covers the
    // whole range of activations the network sees rather than one corner.
    let mut images = Vec::new();
    for slice in Slice::ALL {
        for index in 0..6_u64 {
            images.push(sample(slice, 700_000 + index * 13 + 1).image);
        }
    }

    let mut inputs = Vec::with_capacity(images.len() * INPUT_LEN);
    for image in &images {
        inputs.extend_from_slice(&net::model_input(image));
    }
    let batch = Tensor::from_vec(
        inputs.clone(),
        (
            images.len(),
            net::INPUT_CHANNELS,
            net::INPUT_EDGE,
            net::INPUT_EDGE,
        ),
        &device,
    )?;
    let reference = candle.photograph_probabilities(&batch)?;

    let mut scratch = Activations::new();
    let mut worst = 0.0_f32;
    let mut worst_at = 0;
    for (index, expected) in reference.iter().enumerate() {
        let input = &inputs[index * INPUT_LEN..(index + 1) * INPUT_LEN];
        let got = hand.photograph_probability(input, &mut scratch);
        let difference = (got - expected).abs();
        if difference > worst {
            worst = difference;
            worst_at = index;
        }
    }

    println!(
        "{} images, worst difference {worst:.3e} at index {worst_at}",
        reference.len()
    );
    if worst > TOLERANCE {
        return Err(format!(
            "the hand-written forward pass and the candle one disagree by {worst:.3e}, past the \
             {TOLERANCE:.0e} tolerance — the two descriptions of the network have drifted"
        )
        .into());
    }
    println!("the two descriptions agree");
    Ok(())
}
