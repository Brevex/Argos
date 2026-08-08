//! The triage network: a small CNN, photograph vs synthetic asset.
//!
//! Three 3x3 convolution blocks with 2x2 max-pooling, global average pooling,
//! and one linear head — a MobileNet-class budget scaled to the two-class
//! problem. There is no IDCT-grade signal here to need more: the classes
//! separate on texture statistics (sensor noise, gradient structure, flat
//! fills) that survive the 64x64 downsample.
//!
//! The forward pass is written out here rather than delegated to an inference
//! runtime. Every pure-Rust runtime that can train this network also drags a
//! large dependency tree into the process that reads evidence — `candle`
//! brings a tokenizer stack and several duplicated crate versions — and for
//! four layers of arithmetic that trade is not worth making. Training still
//! uses a runtime, in `tools/train_triage`, outside the workspace; this
//! module reads the weights that training produced.
//!
//! The obvious risk of writing it out is drift: two descriptions of one
//! network that stop agreeing. `tools/train_triage`'s `crosscheck` binary
//! runs both over the same weights and fails if their outputs diverge, which
//! is the guard that makes the split safe.

use argos_core::classify::PixelImage;

use crate::weights::{self, WeightError};

/// Edge of the square model input, in pixels.
pub const INPUT_EDGE: usize = 64;

/// Channels of the model input: red, green, blue.
pub const INPUT_CHANNELS: usize = 3;

/// Classes the head separates: photograph, synthetic asset — in that order.
pub const CLASSES: usize = 2;

/// Index of the photograph class in the head's output.
pub const PHOTOGRAPH_CLASS: usize = 0;

/// Channel widths of the three convolution blocks.
pub const WIDTHS: [usize; 3] = [16, 32, 64];

/// Edge of every convolution kernel.
pub const KERNEL: usize = 3;

/// One 3x3, stride-1, padding-1 convolution layer.
#[derive(Clone, Debug)]
struct Conv {
    /// `[out_channels][in_channels][3][3]`, flattened.
    weight: Vec<f32>,
    bias: Vec<f32>,
    in_channels: usize,
    out_channels: usize,
}

impl Conv {
    fn load(
        weights: &mut weights::Weights,
        name: &str,
        in_channels: usize,
        out_channels: usize,
    ) -> Result<Self, WeightError> {
        Ok(Self {
            weight: weights::take(
                weights,
                &format!("{name}.weight"),
                &[out_channels, in_channels, KERNEL, KERNEL],
            )?,
            bias: weights::take(weights, &format!("{name}.bias"), &[out_channels])?,
            in_channels,
            out_channels,
        })
    }

    /// Convolves `input`, shaped `[in_channels][edge][edge]`, into `out`,
    /// which is resized to `[out_channels][edge][edge]`.
    ///
    /// Padding is one and stride is one, so the spatial size is unchanged.
    /// The `ReLU` is folded in: every use of this layer is followed by one, and
    /// keeping them together saves a pass over the whole plane.
    fn forward_relu(&self, input: &[f32], edge: usize, out: &mut Vec<f32>) {
        let plane = edge * edge;
        out.clear();
        out.resize(self.out_channels * plane, 0.0);

        for out_channel in 0..self.out_channels {
            let bias = self.bias[out_channel];
            let out_plane = &mut out[out_channel * plane..(out_channel + 1) * plane];
            out_plane.fill(bias);

            for in_channel in 0..self.in_channels {
                let in_plane = &input[in_channel * plane..(in_channel + 1) * plane];
                let kernel = &self.weight
                    [((out_channel * self.in_channels) + in_channel) * KERNEL * KERNEL..]
                    [..KERNEL * KERNEL];

                for (ky, row) in kernel.chunks_exact(KERNEL).enumerate() {
                    // Kernel row `ky` reads input row `y + ky - 1`; rows
                    // outside the plane are the zero padding and contribute
                    // nothing.
                    for (kx, tap) in row.iter().enumerate() {
                        if *tap == 0.0 {
                            continue;
                        }
                        for y in 0..edge {
                            let Some(source_y) = (y + ky).checked_sub(1) else {
                                continue;
                            };
                            if source_y >= edge {
                                continue;
                            }
                            let out_row = &mut out_plane[y * edge..(y + 1) * edge];
                            let in_row = &in_plane[source_y * edge..(source_y + 1) * edge];
                            // Same reasoning along x, hoisted out of the
                            // inner loop so it stays a straight fused
                            // multiply-add over a contiguous span.
                            let (out_from, in_from, len) = match kx {
                                0 => (1, 0, edge - 1),
                                1 => (0, 0, edge),
                                _ => (0, 1, edge - 1),
                            };
                            for offset in 0..len {
                                out_row[out_from + offset] += tap * in_row[in_from + offset];
                            }
                        }
                    }
                }
            }
            for value in out_plane {
                *value = value.max(0.0);
            }
        }
    }
}

/// Halves both spatial dimensions by taking the maximum of each 2x2 cell.
fn max_pool2(input: &[f32], channels: usize, edge: usize, out: &mut Vec<f32>) -> usize {
    let half = edge / 2;
    out.clear();
    out.resize(channels * half * half, f32::MIN);
    for channel in 0..channels {
        let in_plane = &input[channel * edge * edge..(channel + 1) * edge * edge];
        let out_plane = &mut out[channel * half * half..(channel + 1) * half * half];
        for y in 0..half {
            for x in 0..half {
                let a = in_plane[2 * y * edge + 2 * x];
                let b = in_plane[2 * y * edge + 2 * x + 1];
                let c = in_plane[(2 * y + 1) * edge + 2 * x];
                let d = in_plane[(2 * y + 1) * edge + 2 * x + 1];
                out_plane[y * half + x] = a.max(b).max(c).max(d);
            }
        }
    }
    half
}

/// The network with its weights.
#[derive(Clone, Debug)]
pub struct Net {
    conv1: Conv,
    conv2: Conv,
    conv3: Conv,
    /// `[CLASSES][WIDTHS[2]]`, flattened.
    head_weight: Vec<f32>,
    head_bias: Vec<f32>,
}

/// Scratch buffers reused across images, so scoring a batch does not allocate
/// per image (`M-MEM-REUSE`).
#[derive(Debug, Default)]
pub struct Activations {
    a: Vec<f32>,
    b: Vec<f32>,
}

impl Activations {
    /// Empty buffers, grown on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Net {
    /// Builds the network from the tensors in a weight file.
    ///
    /// # Errors
    ///
    /// Fails when a tensor is missing or has an unexpected shape.
    pub fn load(bytes: &[u8]) -> Result<Self, WeightError> {
        let mut weights = weights::parse(bytes)?;
        Ok(Self {
            conv1: Conv::load(&mut weights, "conv1", INPUT_CHANNELS, WIDTHS[0])?,
            conv2: Conv::load(&mut weights, "conv2", WIDTHS[0], WIDTHS[1])?,
            conv3: Conv::load(&mut weights, "conv3", WIDTHS[1], WIDTHS[2])?,
            head_weight: weights::take(&mut weights, "head.weight", &[CLASSES, WIDTHS[2]])?,
            head_bias: weights::take(&mut weights, "head.bias", &[CLASSES])?,
        })
    }

    /// Class logits for one image's `model_input`.
    ///
    /// # Panics
    ///
    /// Panics when `input` is not `INPUT_CHANNELS * INPUT_EDGE * INPUT_EDGE`
    /// values — a caller bug, since the only producer is [`model_input`].
    #[must_use]
    pub fn logits(&self, input: &[f32], scratch: &mut Activations) -> [f32; CLASSES] {
        assert_eq!(
            input.len(),
            INPUT_CHANNELS * INPUT_EDGE * INPUT_EDGE,
            "model input must be {INPUT_CHANNELS}x{INPUT_EDGE}x{INPUT_EDGE} values"
        );

        self.conv1.forward_relu(input, INPUT_EDGE, &mut scratch.a);
        let edge = max_pool2(&scratch.a, WIDTHS[0], INPUT_EDGE, &mut scratch.b);
        self.conv2.forward_relu(&scratch.b, edge, &mut scratch.a);
        let edge = max_pool2(&scratch.a, WIDTHS[1], edge, &mut scratch.b);
        self.conv3.forward_relu(&scratch.b, edge, &mut scratch.a);
        let edge = max_pool2(&scratch.a, WIDTHS[2], edge, &mut scratch.b);

        // Global average pool over the spatial dimensions.
        let plane = edge * edge;
        #[expect(
            clippy::cast_precision_loss,
            reason = "the plane is 64 values; the count is exact in f32"
        )]
        let count = plane as f32;
        let mut pooled = [0.0_f32; WIDTHS[2]];
        for (channel, value) in pooled.iter_mut().enumerate() {
            let slice = &scratch.b[channel * plane..(channel + 1) * plane];
            *value = slice.iter().sum::<f32>() / count;
        }

        let mut logits = [0.0_f32; CLASSES];
        for (class, logit) in logits.iter_mut().enumerate() {
            let row = &self.head_weight[class * WIDTHS[2]..(class + 1) * WIDTHS[2]];
            *logit = self.head_bias[class]
                + row
                    .iter()
                    .zip(&pooled)
                    .map(|(weight, value)| weight * value)
                    .sum::<f32>();
        }
        logits
    }

    /// Probability that one image is a photograph.
    #[must_use]
    pub fn photograph_probability(&self, input: &[f32], scratch: &mut Activations) -> f32 {
        softmax(self.logits(input, scratch))[PHOTOGRAPH_CLASS]
    }
}

/// Numerically stable softmax over the class logits.
fn softmax(logits: [f32; CLASSES]) -> [f32; CLASSES] {
    let peak = logits.iter().copied().fold(f32::MIN, f32::max);
    let mut exponentials = logits;
    let mut total = 0.0_f32;
    for value in &mut exponentials {
        *value = (*value - peak).exp();
        total += *value;
    }
    if total == 0.0 {
        // Every logit underflowed to zero, which cannot happen after
        // subtracting the peak — the peak's own term is exactly 1.0.
        return [0.5; CLASSES];
    }
    for value in &mut exponentials {
        *value /= total;
    }
    exponentials
}

/// Smallest per-channel standard deviation treated as real variation.
///
/// Below this the plane is flat to within a quantization step, so it is left
/// at zero rather than amplified into noise by the division.
const MIN_DEVIATION: f32 = 1e-4;

/// Reduces an image to the model's input: 64x64 RGB, point-sampled, then
/// standardized per channel to zero mean and unit deviation.
///
/// Two choices here are load-bearing, and both were measured while deriving
/// the pinned model.
///
/// **Point sampling, not area averaging.** The classes separate on
/// pixel-level texture — sensor noise above all — and averaging a 16x12 cell
/// of a camera frame erases that noise entirely, leaving a smooth gradient
/// indistinguishable from a vector wallpaper. Point sampling keeps every
/// output pixel a real pixel with its noise intact, whatever the source
/// resolution. Training on area-averaged inputs plateaued at 0.74 validation
/// accuracy with photographs and high-resolution assets fused into one class.
///
/// **Standardization, not raw `0.0..=1.0`.** Sensor noise is roughly 4% of
/// full scale, sitting on a mean near 0.5; without a normalization layer in
/// the network, that offset dominates the first convolution and the texture
/// the classifier needs never survives it. Standardizing removes the offset
/// and equalizes contrast, so the convolutions read spatial structure —
/// high-frequency scatter versus a smooth ramp — rather than amplitude. Raw
/// inputs underfit at 0.71 training accuracy, barely above the corpus's 0.67
/// majority-class baseline.
///
/// Transparent pixels are composited over white, matching the perceptual
/// hash.
#[must_use]
pub fn model_input(image: &PixelImage) -> Vec<f32> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let rgba = image.rgba();
    let plane = INPUT_EDGE * INPUT_EDGE;
    let mut input = vec![0.0_f32; INPUT_CHANNELS * plane];

    for out_y in 0..INPUT_EDGE {
        // Centre of the cell, so a small image is sampled evenly rather than
        // biased toward its top-left.
        let y = ((2 * out_y + 1) * height / (2 * INPUT_EDGE)).min(height - 1);
        let row = &rgba[y * width * PixelImage::BYTES_PER_PIXEL..];
        for out_x in 0..INPUT_EDGE {
            let x = ((2 * out_x + 1) * width / (2 * INPUT_EDGE)).min(width - 1);
            let px = &row[x * PixelImage::BYTES_PER_PIXEL..][..PixelImage::BYTES_PER_PIXEL];
            let alpha = u32::from(px[3]);
            for channel in 0..INPUT_CHANNELS {
                let over = (u32::from(px[channel]) * alpha + 255 * (255 - alpha)) / 255;
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "an 8-bit channel value is exact in f32"
                )]
                let value = over as f32 / 255.0;
                input[channel * plane + out_y * INPUT_EDGE + out_x] = value;
            }
        }
    }

    for channel in 0..INPUT_CHANNELS {
        let samples = &mut input[channel * plane..(channel + 1) * plane];
        #[expect(
            clippy::cast_precision_loss,
            reason = "the sample count is a fixed 4096"
        )]
        let count = plane as f32;
        let mean = samples.iter().sum::<f32>() / count;
        let variance = samples
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f32>()
            / count;
        let deviation = variance.sqrt();
        if deviation < MIN_DEVIATION {
            samples.fill(0.0);
            continue;
        }
        for value in samples {
            *value = (*value - mean) / deviation;
        }
    }
    input
}
