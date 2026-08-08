//! Trains the triage CNN on the synthetic labeled corpus and writes the
//! pinned artifact.
//!
//! Training draws from `argos_classify::fixture` with seeds disjoint from the
//! eval harness's fixed set, so the recorded precision/recall measure
//! generalization, not memorization. The output is `triage-v1.safetensors`
//! plus its SHA-256, which are copied into `crates/argos_classify/model/` and
//! `MODEL_SHA256_HEX` together.

use argos_classify::fixture::{Slice, sample};
use argos_classify::net;
use train_triage::candle_net::CandleNet;
use candle_core::{DType, Device, Tensor};
use candle_nn::{Optimizer as _, VarBuilder, VarMap};
use sha2::{Digest as _, Sha256};

/// Training seeds start here; the eval harness uses `1..=EVAL_PER_SLICE`,
/// far below.
const TRAIN_SEED_BASE: u64 = 100_000;

/// Samples drawn per slice for training.
const TRAIN_PER_SLICE: u64 = 150;

/// Samples drawn per slice for validation during training.
const VAL_PER_SLICE: u64 = 30;

const EPOCHS: usize = 40;
const BATCH: usize = 64;
const LEARNING_RATE: f64 = 6e-4;

const INPUT_LEN: usize = net::INPUT_CHANNELS * net::INPUT_EDGE * net::INPUT_EDGE;

fn dataset(base: u64, per_slice: u64) -> (Vec<f32>, Vec<u32>) {
    let mut inputs = Vec::new();
    let mut labels = Vec::new();
    for slice in Slice::ALL {
        for index in 0..per_slice {
            let labeled = sample(slice, base + index * 7 + 1);
            inputs.extend_from_slice(&net::model_input(&labeled.image));
            labels.push(match labeled.slice.truth() {
                argos_core::classify::TriageLabel::Photograph => 0,
                _ => 1,
            });
        }
    }
    (inputs, labels)
}

fn accuracy(net: &CandleNet, inputs: &[f32], labels: &[u32], device: &Device) -> anyhow_free::Result<f32> {
    let n = labels.len();
    let mut correct = 0_usize;
    for start in (0..n).step_by(BATCH) {
        let end = (start + BATCH).min(n);
        let batch = Tensor::from_slice(
            &inputs[start * INPUT_LEN..end * INPUT_LEN],
            (
                end - start,
                net::INPUT_CHANNELS,
                net::INPUT_EDGE,
                net::INPUT_EDGE,
            ),
            device,
        )?;
        let probs = net.photograph_probabilities(&batch)?;
        for (p, label) in probs.iter().zip(&labels[start..end]) {
            let predicted = u32::from(*p < 0.5);
            if predicted == *label {
                correct += 1;
            }
        }
    }
    Ok(correct as f32 / n as f32)
}

mod anyhow_free {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}

fn main() -> anyhow_free::Result<()> {
    let device = Device::Cpu;
    println!("generating corpus...");
    let (train_x, train_y) = dataset(TRAIN_SEED_BASE, TRAIN_PER_SLICE);
    let (val_x, val_y) = dataset(TRAIN_SEED_BASE + 50_000, VAL_PER_SLICE);
    let n = train_y.len();
    println!("train {n} samples, val {} samples", val_y.len());

    let varmap = VarMap::new();
    let vars = VarBuilder::from_varmap(&varmap, DType::F32, &device);
    let net = CandleNet::build(&vars)?;
    let mut optimizer = candle_nn::AdamW::new(
        varmap.all_vars(),
        candle_nn::ParamsAdamW {
            lr: LEARNING_RATE,
            ..Default::default()
        },
    )?;

    // Deterministic shuffle: xorshift over indices.
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = 0x5EED_5EED_5EED_5EEDu64;
    let out = "triage-v1.safetensors";
    // Validation accuracy oscillates across epochs, so the last epoch is not
    // the best model — keep whichever weights scored highest and ship those.
    let mut best = 0.0_f32;
    let mut best_epoch = 0;
    for epoch in 0..EPOCHS {
        for i in (1..n).rev() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            order.swap(i, (rng % (i as u64 + 1)) as usize);
        }
        let mut loss_sum = 0.0_f32;
        let mut steps = 0;
        for chunk in order.chunks(BATCH) {
            let mut xs = Vec::with_capacity(chunk.len() * INPUT_LEN);
            let mut ys = Vec::with_capacity(chunk.len());
            for &i in chunk {
                xs.extend_from_slice(&train_x[i * INPUT_LEN..(i + 1) * INPUT_LEN]);
                ys.push(train_y[i]);
            }
            let batch = Tensor::from_vec(
                xs,
                (
                    chunk.len(),
                    net::INPUT_CHANNELS,
                    net::INPUT_EDGE,
                    net::INPUT_EDGE,
                ),
                &device,
            )?;
            let labels = Tensor::from_vec(ys, chunk.len(), &device)?;
            let logits = net.forward(&batch)?;
            let loss = candle_nn::loss::cross_entropy(&logits, &labels)?;
            optimizer.backward_step(&loss)?;
            loss_sum += loss.to_scalar::<f32>()?;
            steps += 1;
        }
        let val_acc = accuracy(&net, &val_x, &val_y, &device)?;
        let train_acc = accuracy(&net, &train_x, &train_y, &device)?;
        let kept = if val_acc > best {
            best = val_acc;
            best_epoch = epoch;
            varmap.save(out)?;
            " <- kept"
        } else {
            ""
        };
        println!(
            "epoch {epoch}: loss {:.4}, train acc {train_acc:.3}, val acc {val_acc:.3}{kept}",
            loss_sum / steps as f32,
        );
    }

    println!("best val acc {best:.3} at epoch {best_epoch}");
    let bytes = std::fs::read(out)?;
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    println!("wrote {out} ({} bytes)", bytes.len());
    println!("sha256 {hex}");
    Ok(())
}
