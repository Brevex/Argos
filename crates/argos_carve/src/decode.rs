//! Full pixel decoding, as the oracle a reassembly search needs.
//!
//! Structural validation asks "could these bytes be a JPEG?". For contiguous
//! carving that is enough, because a false positive would need a whole file's
//! worth of valid structure to appear by chance behind a real `SOI`.
//! Reassembly is different: it tests thousands of hypotheses, so an oracle that
//! is wrong one time in a thousand is wrong on every scan.
//!
//! And the JPEG marker grammar *is* that permissive. Random bytes spliced into
//! an entropy-coded scan regularly produce a byte pair that reads as a
//! length-prefixed segment, which skips an arbitrary span and lets the scan
//! wander on until it meets an `FF D9` — a complete, entirely fabricated
//! image. Only entropy-decoding the scan rejects that: the Huffman tables, the
//! coefficient counts and the frame dimensions all have to agree.
//!
//! So reassembled JPEGs are accepted on a real decode. PNG needs no equivalent
//! here: its per-chunk CRC32 already makes a chance assembly impossible, and
//! the structural validator verifies every one.

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

/// Largest image this module will decode.
///
/// Decoding needs the compressed bytes and the pixel plane in memory at once,
/// so the bound is what keeps a hypothesis from turning into an allocation the
/// medium chose (A-BOUNDED-ALLOC). Far above any photograph; an assembly
/// larger than this is not verifiable here and is therefore not claimed.
pub const MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;

/// Largest pixel count a decoded image may have, so a crafted frame header
/// cannot demand an enormous plane.
///
/// 512 megapixels is roughly twenty times the largest consumer sensor.
pub const MAX_PIXELS: usize = 512 * 1024 * 1024;

/// Rows a frame needs before a seam ratio means anything.
///
/// The ratio is against the median row difference, so a frame with only a
/// handful of rows has no reliable median to compare with.
const MIN_ROWS_FOR_SEAM: usize = 16;

/// A decoded image, reduced to what reassembly needs to judge it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Luma plane, one byte per pixel, row-major.
    luma: Vec<u8>,
}

impl Decoded {
    /// How much the row at `row` stands out from the picture around it.
    ///
    /// This is the stitch-row measure the reassembly spec calls for, and it is
    /// what a whole-frame average cannot do. Two photographs from one camera
    /// share Huffman tables, so a splice from one scan into the other decodes
    /// cleanly — the entropy decoder cannot tell them apart. What gives it away
    /// is the picture: the DC predictor carries across the splice, so the rows
    /// after it sit at the wrong brightness and the seam shows as one row far
    /// sharper than its neighbours.
    ///
    /// The result is the seam's row difference divided by the frame's median
    /// row difference, so it is scale-free: a busy photograph and a smooth one
    /// both score near `1.0` when there is no seam. `None` when the row is not
    /// in the frame or the frame is too small to have a median.
    #[must_use]
    pub fn seam_ratio(&self, row: u32) -> Option<f32> {
        let width = self.width as usize;
        let height = self.height as usize;
        if width == 0 || height < MIN_ROWS_FOR_SEAM || row == 0 || row as usize >= height {
            return None;
        }
        let mut differences: Vec<f32> = (1..height)
            .filter_map(|index| self.row_difference(index))
            .collect();
        let seam = self.row_difference(row as usize)?;
        if differences.len() < MIN_ROWS_FOR_SEAM {
            return None;
        }
        differences.sort_by(f32::total_cmp);
        let median = differences[differences.len() / 2];
        if median <= f32::EPSILON {
            // A frame with no variation at all says nothing either way.
            return None;
        }
        Some(seam / median)
    }

    /// Mean absolute luminance difference between row `row` and the one above.
    fn row_difference(&self, row: usize) -> Option<f32> {
        let width = self.width as usize;
        let above = self.luma.get((row - 1) * width..row * width)?;
        let here = self.luma.get(row * width..(row + 1) * width)?;
        let total: u64 = above
            .iter()
            .zip(here)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a mean of byte differences needs no more than f32 precision"
        )]
        let mean = total as f32 / width as f32;
        Some(mean)
    }

    /// Mean absolute luminance difference between vertically adjacent pixels,
    /// scaled to 0.0..=1.0.
    ///
    /// This is the pixel-boundary smoothness term the reassembly graph is
    /// weighted by: a correct assembly is a photograph, where rows resemble
    /// the row above; a wrong one has a seam where the splice landed, and its
    /// remaining rows are decoded from mismatched coefficients, which raises
    /// this sharply. Lower is smoother, so lower is better.
    #[must_use]
    pub fn roughness(&self) -> f32 {
        let width = self.width as usize;
        if width == 0 || self.height < 2 {
            return 1.0;
        }
        let mut total = 0_u64;
        let mut counted = 0_u64;
        for row in 1..self.height as usize {
            let (Some(above), Some(here)) = (
                self.luma.get((row - 1) * width..row * width),
                self.luma.get(row * width..(row + 1) * width),
            ) else {
                break;
            };
            for (a, b) in above.iter().zip(here) {
                total += u64::from(a.abs_diff(*b));
                counted += 1;
            }
        }
        if counted == 0 {
            return 1.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a mean of byte differences needs no more than f32 precision"
        )]
        let mean = total as f32 / counted as f32;
        (mean / 255.0).clamp(0.0, 1.0)
    }
}

/// Decodes `bytes` as a JPEG, to its luma plane.
///
/// `None` when the bytes are not a decodable image — which, for a reassembly
/// hypothesis, is the answer that matters. Nothing here trusts a dimension or
/// a length read from the stream: the decoder is given explicit bounds and its
/// output is checked against them.
#[must_use]
pub fn decode_jpeg_luma(bytes: &[u8]) -> Option<Decoded> {
    if bytes.len() > MAX_DECODE_BYTES {
        return None;
    }
    // Strict mode is the whole point. Left lenient, the decoder pads a
    // truncated or spliced scan with flat grey and returns success — and flat
    // grey is *smooth*, so it would sail through the roughness check as well.
    // Strict makes a stream that does not match its own frame an error, which
    // is what turns "decoded" into evidence.
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::Luma)
        .set_strict_mode(true)
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    let luma = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;

    let pixels = width.checked_mul(height)?;
    if pixels == 0 || pixels > MAX_PIXELS || luma.len() < pixels {
        return None;
    }
    Some(Decoded {
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        luma,
    })
}
