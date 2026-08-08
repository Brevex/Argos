//! Sanity check: can this training loop overfit a trivially separable task?
//!
//! Class 0 = per-pixel noise, class 1 = flat images. If loss does not
//! collapse toward zero here, the loop or the conv backward is broken and no
//! corpus change will fix it.

use argos_classify::net;
use candle_core::{DType, Device, Tensor};
use candle_nn::{Optimizer as _, VarBuilder, VarMap};

const INPUT_LEN: usize = net::INPUT_CHANNELS * net::INPUT_EDGE * net::INPUT_EDGE;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let device = Device::Cpu;
    let n = 64_usize;
    let mut xs = Vec::with_capacity(n * INPUT_LEN);
    let mut ys: Vec<u32> = Vec::with_capacity(n);
    let mut rng = 0x1234_5678_u64;
    for i in 0..n {
        let noisy = i % 2 == 0;
        ys.push(u32::from(!noisy));
        for _ in 0..INPUT_LEN {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let v = if noisy {
                (rng % 256) as f32 / 255.0
            } else {
                0.5
            };
            xs.push(v);
        }
    }
    let batch = Tensor::from_vec(
        xs,
        (n, net::INPUT_CHANNELS, net::INPUT_EDGE, net::INPUT_EDGE),
        &device,
    )?;
    let labels = Tensor::from_vec(ys, n, &device)?;

    let varmap = VarMap::new();
    let vars = VarBuilder::from_varmap(&varmap, DType::F32, &device);
    let network = train_triage::candle_net::CandleNet::build(&vars)?;
    let mut optimizer = candle_nn::AdamW::new(
        varmap.all_vars(),
        candle_nn::ParamsAdamW {
            lr: 1e-3,
            ..Default::default()
        },
    )?;
    println!("vars: {}", varmap.all_vars().len());
    for step in 0..60 {
        let logits = network.forward(&batch)?;
        let loss = candle_nn::loss::cross_entropy(&logits, &labels)?;
        optimizer.backward_step(&loss)?;
        if step % 10 == 0 {
            println!("step {step}: loss {:.4}", loss.to_scalar::<f32>()?);
        }
    }
    Ok(())
}
