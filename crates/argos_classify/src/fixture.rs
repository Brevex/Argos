//! Synthetic labeled corpus: photographs vs assets, generated, never real.
//!
//! Every image here is drawn from a seed — no photograph of a real scene ever
//! enters the repository. The photograph class models what a sensor produces
//! (smooth shading, hue variation, per-pixel noise); the asset classes model
//! what authoring tools produce (flat fills, hard edges, transparency,
//! noiseless gradients). The high-resolution-asset slice exists because
//! resolution alone cannot separate the classes, and a corpus without it
//! would let a model learn exactly that shortcut.
//!
//! The training tool and the eval harness both draw from this generator with
//! disjoint seed ranges; the eval set is fixed by its seeds (A-EVAL-GATED).

use argos_core::ports::{PixelImage, TriageLabel};

/// Deterministic xorshift64 generator, so a corpus is its seeds.
#[derive(Clone, Debug)]
pub struct Noise(u64);

impl Noise {
    /// A generator over `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    /// Next raw value.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform index in `0..bound` (`bound` of zero yields zero).
    pub fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).unwrap_or(u64::MAX);
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }

    /// Uniform value in `-amplitude..=amplitude`.
    pub(crate) fn jitter(&mut self, amplitude: i64) -> i64 {
        if amplitude <= 0 {
            return 0;
        }
        let span = amplitude.unsigned_abs().saturating_mul(2).saturating_add(1);
        i64::try_from(self.next_u64() % span).unwrap_or(0) - amplitude
    }
}

/// The slices the corpus is measured over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slice {
    /// A camera-class photograph.
    Photograph,
    /// A photograph at thumbnail resolution — small is not synthetic.
    PhotographThumbnail,
    /// A monochrome photograph. Its own slice because greyscale collapses
    /// every colour statistic a rule could use, so it is the shape most
    /// likely to be mistaken for flat artwork — and a black-and-white
    /// photograph is evidence like any other.
    PhotographGreyscale,
    /// A small icon: flat shapes, transparency.
    Icon,
    /// Pixel-art sprite: few colors, transparent background.
    Sprite,
    /// Application or web UI chrome: bars, cards, separators, text-like rows.
    UiChrome,
    /// A high-resolution vector-style asset — the slice resolution alone
    /// cannot decide.
    HighResAsset,
}

impl Slice {
    /// Every slice, in a fixed order.
    pub const ALL: [Self; 7] = [
        Self::Photograph,
        Self::PhotographThumbnail,
        Self::PhotographGreyscale,
        Self::Icon,
        Self::Sprite,
        Self::UiChrome,
        Self::HighResAsset,
    ];

    /// Ground-truth label of the slice.
    #[must_use]
    pub fn truth(self) -> TriageLabel {
        match self {
            Self::Photograph | Self::PhotographThumbnail | Self::PhotographGreyscale => {
                TriageLabel::Photograph
            }
            Self::Icon | Self::Sprite | Self::UiChrome | Self::HighResAsset => {
                TriageLabel::SyntheticAsset
            }
        }
    }

    /// Position of the slice in [`Slice::ALL`], used to decorrelate seeds
    /// between slices.
    #[must_use]
    pub fn index(self) -> u8 {
        Self::ALL
            .iter()
            .position(|other| *other == self)
            .and_then(|at| u8::try_from(at).ok())
            .unwrap_or(0)
    }

    /// Short name for tables.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Photograph => "photograph",
            Self::PhotographThumbnail => "photo-thumbnail",
            Self::PhotographGreyscale => "photo-greyscale",
            Self::Icon => "icon",
            Self::Sprite => "sprite",
            Self::UiChrome => "ui-chrome",
            Self::HighResAsset => "high-res asset",
        }
    }
}

/// One labeled sample.
#[derive(Clone, Debug)]
pub struct Labeled {
    /// The image.
    pub image: PixelImage,
    /// The slice it was drawn from; its truth is [`Slice::truth`].
    pub slice: Slice,
}

/// Draws the sample of `slice` at `seed`.
#[must_use]
pub fn sample(slice: Slice, seed: u64) -> Labeled {
    let mut noise = Noise::new(seed ^ u64::from(slice.index()).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let image = match slice {
        Slice::Photograph => {
            let (w, h) = [(640, 480), (800, 600), (1024, 768)][noise.below(3)];
            photograph(w, h, &mut noise)
        }
        Slice::PhotographThumbnail => photograph(160, 120, &mut noise),
        Slice::PhotographGreyscale => {
            let (w, h) = [(640, 480), (800, 600)][noise.below(2)];
            desaturated(&photograph(w, h, &mut noise))
        }
        Slice::Icon => icon(&mut noise),
        Slice::Sprite => sprite(&mut noise),
        Slice::UiChrome => ui_chrome(&mut noise),
        Slice::HighResAsset => high_res_asset(&mut noise),
    };
    Labeled { image, slice }
}

/// A mutable RGBA canvas the generators draw on.
struct Canvas {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize, fill: [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            rgba.extend_from_slice(&fill);
        }
        Self {
            width,
            height,
            rgba,
        }
    }

    fn put(&mut self, x: usize, y: usize, color: [u8; 4]) {
        if x < self.width && y < self.height {
            let at = (y * self.width + x) * 4;
            self.rgba[at..at + 4].copy_from_slice(&color);
        }
    }

    fn fill_rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, color: [u8; 4]) {
        for y in y0..(y0 + h).min(self.height) {
            for x in x0..(x0 + w).min(self.width) {
                self.put(x, y, color);
            }
        }
    }

    fn fill_circle(&mut self, cx: usize, cy: usize, radius: usize, color: [u8; 4]) {
        let radius_squared = radius.saturating_mul(radius);
        for y in cy.saturating_sub(radius)..(cy + radius + 1).min(self.height) {
            let dy = y.abs_diff(cy);
            for x in cx.saturating_sub(radius)..(cx + radius + 1).min(self.width) {
                let dx = x.abs_diff(cx);
                if dx * dx + dy * dy <= radius_squared {
                    self.put(x, y, color);
                }
            }
        }
    }

    fn into_image(self) -> PixelImage {
        PixelImage::new(
            u32::try_from(self.width).unwrap_or(0),
            u32::try_from(self.height).unwrap_or(0),
            self.rgba,
        )
    }
}

fn clamp_channel(value: i64) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

/// What a sensor produces: smooth shading, hue variation, objects, and noise
/// on every pixel.
fn photograph(width: usize, height: usize, noise: &mut Noise) -> PixelImage {
    let span_x = i64::try_from(width).unwrap_or(1).max(1);
    let span_y = i64::try_from(height).unwrap_or(1).max(1);

    // Scene parameters: per-channel gradients and band structure.
    let base: [i64; 3] = std::array::from_fn(|_| 60 + i64::try_from(noise.below(120)).unwrap_or(0));
    let slope_x: [i64; 3] = std::array::from_fn(|_| noise.jitter(60));
    let slope_y: [i64; 3] = std::array::from_fn(|_| noise.jitter(60));
    let bands = 2 + i64::try_from(noise.below(4)).unwrap_or(0);
    let band_amp = 15 + i64::try_from(noise.below(25)).unwrap_or(0);

    // A few soft blobs standing in for subjects.
    let short = span_x.min(span_y);
    let mut blobs = Vec::new();
    for _ in 0..2 + noise.below(3) {
        let radius = short / 8 + i64::try_from(noise.below(width.min(height) / 4)).unwrap_or(0);
        blobs.push((
            i64::try_from(noise.below(width)).unwrap_or(0),
            i64::try_from(noise.below(height)).unwrap_or(0),
            radius.max(1),
            [noise.jitter(70), noise.jitter(70), noise.jitter(70)],
        ));
    }

    let half_x = span_x / 2;
    let half_y = span_y / 2;
    let corner = half_x * half_x + half_y * half_y;
    let mut rgba = Vec::with_capacity(width * height * PixelImage::BYTES_PER_PIXEL);
    for fy in 0..span_y {
        for fx in 0..span_x {
            // Low-frequency band shading shared by the channels.
            let phase = (fx * bands * 314 / span_x + fy * 157 / span_y) % 628;
            let band = band_amp * sin_milli(phase) / 1000;
            // Vignette: darker toward the corners, as lenses are.
            let dx = fx - half_x;
            let dy = fy - half_y;
            let vignette = -40 * (dx * dx + dy * dy) / corner.max(1);

            let mut px = [0_u8; 4];
            px[3] = u8::MAX;
            for (channel, value) in px[..3].iter_mut().enumerate() {
                let mut level = base[channel]
                    + slope_x[channel] * fx / span_x
                    + slope_y[channel] * fy / span_y
                    + band
                    + vignette;
                for (bx, by, radius, tint) in &blobs {
                    let (dx, dy) = (fx - bx, fy - by);
                    let distance = dx * dx + dy * dy;
                    let reach = radius * radius;
                    if distance <= reach {
                        // Soft falloff toward the blob edge.
                        level += tint[channel] * (reach - distance) / reach;
                    }
                }
                // Sensor noise: independent per pixel and channel.
                level += noise.jitter(10);
                *value = clamp_channel(level);
            }
            rgba.extend_from_slice(&px);
        }
    }
    PixelImage::new(
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
        rgba,
    )
}

/// Collapses an image to grey, keeping its luminance and its grain.
///
/// This is what a monochrome camera mode or a desaturating edit produces: the
/// picture is intact, but every colour statistic a rule might use is gone.
fn desaturated(image: &PixelImage) -> PixelImage {
    let mut rgba = image.rgba().to_vec();
    for px in rgba.as_chunks_mut::<{ PixelImage::BYTES_PER_PIXEL }>().0 {
        let luma = u8::try_from(
            ((77 * u32::from(px[0]) + 150 * u32::from(px[1]) + 29 * u32::from(px[2])) >> 8)
                .min(255),
        )
        .unwrap_or(0);
        px[0] = luma;
        px[1] = luma;
        px[2] = luma;
    }
    PixelImage::new(image.width(), image.height(), rgba)
}

/// Integer sine: `sin(phase/100)` scaled by 1000, phase in `0..628`.
fn sin_milli(phase: i64) -> i64 {
    // Bhaskara I approximation, exact enough for shading.
    let phase = phase.rem_euclid(628);
    let (phase, sign) = if phase < 314 {
        (phase, 1)
    } else {
        (phase - 314, -1)
    };
    sign * 4 * phase * (314 - phase) * 1000 / (49348 * 5 - phase * (314 - phase) * 4)
}

/// A flat-shape icon, usually on a transparent canvas.
fn icon(noise: &mut Noise) -> PixelImage {
    let edge = [16_usize, 24, 32, 48, 64][noise.below(5)];
    let transparent = noise.below(4) != 0;
    let background = if transparent {
        [0, 0, 0, 0]
    } else {
        flat_color(noise)
    };
    let mut canvas = Canvas::new(edge, edge, background);
    for _ in 0..=noise.below(3) {
        let color = flat_color(noise);
        if noise.below(2) == 0 {
            let radius = edge / 4 + noise.below(edge / 4);
            let spread = edge / 4;
            canvas.fill_circle(
                edge / 2 + noise.below(spread),
                edge / 2 + noise.below(spread),
                radius.max(1),
                color,
            );
        } else {
            let w = edge / 3 + noise.below(edge / 3);
            let h = edge / 3 + noise.below(edge / 3);
            canvas.fill_rect(
                noise.below(edge - w + 1),
                noise.below(edge - h + 1),
                w,
                h,
                color,
            );
        }
    }
    canvas.into_image()
}

/// Pixel art: a coarse grid of cells from a small palette over transparency.
fn sprite(noise: &mut Noise) -> PixelImage {
    let edge = 16 + noise.below(33);
    let cell = 2 + noise.below(3);
    let mut palette = vec![[0, 0, 0, 0]];
    for _ in 0..3 + noise.below(5) {
        palette.push(flat_color(noise));
    }
    let mut canvas = Canvas::new(edge, edge, [0, 0, 0, 0]);
    for cy in 0..edge.div_ceil(cell) {
        for cx in 0..edge.div_ceil(cell) {
            let color = palette[noise.below(palette.len())];
            canvas.fill_rect(cx * cell, cy * cell, cell, cell, color);
        }
    }
    canvas.into_image()
}

/// Application chrome: flat panels, one gradient bar, separators and
/// text-like dashes.
fn ui_chrome(noise: &mut Noise) -> PixelImage {
    let (width, height) = [(360, 640), (800, 600), (1280, 720)][noise.below(3)];
    let background = flat_gray(noise, 230, 25);
    let mut canvas = Canvas::new(width, height, background);

    // Header bar with a vertical linear gradient.
    let bar_h = height / 12;
    let top = flat_color(noise);
    for y in 0..bar_h {
        let drop = i64::try_from(y * 40 / bar_h.max(1)).unwrap_or(0);
        let shade = |c: u8| clamp_channel(i64::from(c) - drop);
        let row = [shade(top[0]), shade(top[1]), shade(top[2]), 255];
        canvas.fill_rect(0, y, width, 1, row);
    }

    // Cards with 1px borders and text-like rows of dashes.
    for _ in 0..2 + noise.below(4) {
        let w = width / 3 + noise.below(width / 3);
        let h = height / 8 + noise.below(height / 6);
        let x0 = noise.below(width - w);
        let y0 = bar_h + noise.below(height - bar_h - h);
        let border = flat_gray(noise, 120, 60);
        canvas.fill_rect(x0, y0, w, h, border);
        canvas.fill_rect(x0 + 1, y0 + 1, w - 2, h - 2, flat_gray(noise, 245, 10));
        let ink = flat_gray(noise, 60, 40);
        let mut y = y0 + 8;
        while y + 4 < y0 + h {
            let dash_w = w / 2 + noise.below(w / 3);
            canvas.fill_rect(x0 + 8, y, dash_w.min(w - 16), 3, ink);
            y += 9;
        }
    }

    // Thin separators.
    for _ in 0..2 + noise.below(3) {
        let y = noise.below(height);
        canvas.fill_rect(0, y, width, 1, flat_gray(noise, 180, 40));
    }
    canvas.into_image()
}

/// A wallpaper-class vector asset: smooth noiseless gradient plus large flat
/// shapes. Large on purpose — this is the slice resolution cannot decide.
fn high_res_asset(noise: &mut Noise) -> PixelImage {
    let (width, height) = [(1920, 1080), (2560, 1440)][noise.below(2)];
    let from = flat_color(noise);
    let to = flat_color(noise);
    let radial = noise.below(2) == 0;
    let mut canvas = Canvas::new(width, height, [0, 0, 0, 255]);

    let max_d2 = ((width * width + height * height) / 4).max(1);
    for y in 0..height {
        for x in 0..width {
            // Interpolation weight in 0..=1000, linear or radial.
            let weight = if radial {
                let dx = x.abs_diff(width / 2);
                let dy = y.abs_diff(height / 2);
                ((dx * dx + dy * dy) * 1000 / max_d2).min(1000)
            } else {
                (x * 500 / width.max(1)) + (y * 500 / height.max(1))
            };
            let t = i64::try_from(weight).unwrap_or(0);
            let mut px = [0_u8; 4];
            px[3] = 255;
            for channel in 0..3 {
                let a = i64::from(from[channel]);
                let b = i64::from(to[channel]);
                px[channel] = clamp_channel(a + (b - a) * t / 1000);
            }
            canvas.put(x, y, px);
        }
    }

    // Large flat geometry over the gradient.
    for _ in 0..3 + noise.below(4) {
        let color = flat_color(noise);
        if noise.below(2) == 0 {
            let radius = height / 8 + noise.below(height / 4);
            canvas.fill_circle(
                noise.below(width),
                noise.below(height),
                radius.max(1),
                color,
            );
        } else {
            let w = width / 6 + noise.below(width / 4);
            let h = height / 6 + noise.below(height / 4);
            canvas.fill_rect(noise.below(width), noise.below(height), w, h, color);
        }
    }
    canvas.into_image()
}

/// A saturated flat color.
fn flat_color(noise: &mut Noise) -> [u8; 4] {
    let mut channel = || u8::try_from(40 + noise.below(200)).unwrap_or(u8::MAX);
    [channel(), channel(), channel(), u8::MAX]
}

/// A flat gray around `center` with `spread` of variation.
fn flat_gray(noise: &mut Noise, center: i64, spread: i64) -> [u8; 4] {
    let level = clamp_channel(center + noise.jitter(spread));
    [level, level, level, 255]
}
