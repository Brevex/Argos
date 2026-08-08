//! The training-side description of the triage network.
//!
//! `argos_classify::net` writes the forward pass out by hand so the shipped
//! crate carries no inference runtime. Training still needs autodiff, so the
//! same architecture is described here in `candle` — and the two descriptions
//! are checked against each other by the `crosscheck` binary, which is what
//! makes having two of them safe.
//!
//! Anything changed here must be changed there, and `crosscheck` must pass
//! before the resulting weights are pinned.

use argos_classify::net::{CLASSES, INPUT_CHANNELS, KERNEL, WIDTHS};
use candle_core::{D, Tensor};
use candle_nn::{Conv2d, Conv2dConfig, Linear, Module as _, VarBuilder};

/// The network, as candle sees it.
#[derive(Clone, Debug)]
pub struct CandleNet {
    conv1: Conv2d,
    conv2: Conv2d,
    conv3: Conv2d,
    head: Linear,
}

impl CandleNet {
    /// Builds the network from `vars`, fresh variables or loaded weights.
    pub fn build(vars: &VarBuilder<'_>) -> Result<Self, candle_core::Error> {
        let pad = Conv2dConfig {
            padding: 1,
            ..Conv2dConfig::default()
        };
        Ok(Self {
            conv1: candle_nn::conv2d(INPUT_CHANNELS, WIDTHS[0], KERNEL, pad, vars.pp("conv1"))?,
            conv2: candle_nn::conv2d(WIDTHS[0], WIDTHS[1], KERNEL, pad, vars.pp("conv2"))?,
            conv3: candle_nn::conv2d(WIDTHS[1], WIDTHS[2], KERNEL, pad, vars.pp("conv3"))?,
            head: candle_nn::linear(WIDTHS[2], CLASSES, vars.pp("head"))?,
        })
    }

    /// Class logits for a batch shaped `[n, 3, 64, 64]`, returned as `[n, 2]`.
    pub fn forward(&self, batch: &Tensor) -> Result<Tensor, candle_core::Error> {
        let x = self.conv1.forward(batch)?.relu()?.max_pool2d(2)?;
        let x = self.conv2.forward(&x)?.relu()?.max_pool2d(2)?;
        let x = self.conv3.forward(&x)?.relu()?.max_pool2d(2)?;
        // Global average pool over the spatial dimensions.
        let x = x.mean(D::Minus1)?.mean(D::Minus1)?;
        self.head.forward(&x)
    }

    /// Probability that each image in the batch is a photograph.
    pub fn photograph_probabilities(&self, batch: &Tensor) -> Result<Vec<f32>, candle_core::Error> {
        let probabilities = candle_nn::ops::softmax(&self.forward(batch)?, D::Minus1)?;
        probabilities
            .narrow(D::Minus1, argos_classify::net::PHOTOGRAPH_CLASS, 1)?
            .flatten_all()?
            .to_vec1()
    }
}
