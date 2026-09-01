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

use argos_core::Format;
use argos_core::ports::PixelImage;
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
pub(crate) const MAX_PIXELS: usize = 512 * 1024 * 1024;

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
    pub(crate) fn seam_ratio(&self, row: u32) -> Option<f32> {
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

/// Largest pixel count [`decode_rgba`] will materialize.
///
/// RGBA is four bytes per pixel, so this bounds the plane at 256 MiB — one
/// image at a time, transient, and above every consumer camera sensor shipped
/// (the largest are ~61 megapixels). A frame claiming more is not scored
/// rather than allowed to size an allocation (A-BOUNDED-ALLOC).
pub const MAX_RGBA_PIXELS: u64 = 64 * 1024 * 1024;

/// Longest edge either decoder is allowed to report.
///
/// A second, per-axis bound below the area bound, because a decoder applies
/// its own limits while parsing the header and this is the one it understands.
/// 32768 is twice the widest consumer sensor; a frame past it on either axis
/// is not a photograph anyone took.
const MAX_FRAME_EDGE: usize = 32768;

/// Decodes a validated artifact's bytes to RGBA pixels for triage.
///
/// `None` when the bytes do not decode, when a dimension is zero, or when the
/// frame exceeds [`MAX_RGBA_PIXELS`] or `bytes` exceeds [`MAX_DECODE_BYTES`] —
/// an artifact that cannot be decoded within bounds is left unscored, never
/// dropped.
///
/// The pixel ceiling is enforced against the frame header **before** the
/// decoder is allowed to run, because a decoder sizes its output buffer from
/// that header the moment it has parsed it. Checking the returned buffer
/// instead would check a number that only exists once the allocation it was
/// meant to prevent has already been made: a one-kilobyte PNG declaring
/// 65535x65535 RGBA-16 asks for about 34 GB, and a refused allocation aborts
/// the process rather than failing the artifact (A-BOUNDED-ALLOC,
/// A-UNTRUSTED-ONDISK).
#[must_use]
pub fn decode_rgba(format: Format, bytes: &[u8]) -> Option<PixelImage> {
    if bytes.len() > MAX_DECODE_BYTES {
        return None;
    }
    let (width, height, rgba) = match format {
        Format::Jpeg => jpeg_rgba(bytes)?,
        Format::Png => png_rgba(bytes)?,
    };
    let pixels = affordable(width, height)?;
    let expected = usize::try_from(pixels.checked_mul(PixelImage::BYTES_PER_PIXEL as u64)?).ok()?;
    // An output that does not match its own header is a decoder disagreement
    // over hostile input; refuse it rather than construct a lying image.
    if rgba.len() != expected {
        return None;
    }
    Some(PixelImage::new(width, height, rgba))
}

/// The pixel count of a `width` by `height` frame, when it is one this module
/// will materialize.
///
/// `None` for an empty frame, one whose dimensions overflow, or one past
/// [`MAX_RGBA_PIXELS`].
fn affordable(width: u32, height: u32) -> Option<u64> {
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    (pixels > 0 && pixels <= MAX_RGBA_PIXELS).then_some(pixels)
}

/// Decodes a JPEG to interleaved RGBA.
fn jpeg_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    // Not strict: triage scores images that already passed structural
    // validation, so a tolerant decode that yields pixels is the useful one —
    // this is scoring, not the reassembly oracle above.
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_FRAME_EDGE)
        .set_max_height(MAX_FRAME_EDGE);
    let mut decoder = JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    // Headers first: this parses the frame header without sizing anything
    // from it, so the area check below happens before the decoder allocates.
    decoder.decode_headers().ok()?;
    let (width, height) = decoder.dimensions()?;
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    affordable(width, height)?;

    let rgba = decoder.decode().ok()?;
    Some((width, height, rgba))
}

/// Decodes a PNG and expands whatever colorspace it stored to RGBA.
fn png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    use zune_png::zune_core::bit_depth::BitDepth;
    use zune_png::zune_core::colorspace::ColorSpace as PngColor;
    use zune_png::zune_core::options::DecoderOptions as PngOptions;

    let options = PngOptions::default()
        .set_max_width(MAX_FRAME_EDGE)
        .set_max_height(MAX_FRAME_EDGE);
    let mut decoder = zune_png::PngDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    // Same reasoning as the JPEG path: `IHDR` is parsed here, and the area it
    // declares is checked before `decode_raw` sizes a buffer from it.
    decoder.decode_headers().ok()?;
    let (width, height) = decoder.dimensions()?;
    let frame_width = u32::try_from(width).ok()?;
    let frame_height = u32::try_from(height).ok()?;
    affordable(frame_width, frame_height)?;

    let samples = decoder.decode_raw().ok()?;
    let depth = decoder.depth()?;
    let color = decoder.colorspace()?;

    // 16-bit samples arrive as big-endian byte pairs; keep the high byte.
    // Triage needs tone, not tonal depth.
    let samples: Vec<u8> = match depth {
        BitDepth::Sixteen => samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| pair[0])
            .collect(),
        _ => samples,
    };

    let pixels = width.checked_mul(height)?;
    let channels = match color {
        PngColor::Luma => 1,
        PngColor::LumaA => 2,
        PngColor::RGB => 3,
        PngColor::RGBA => 4,
        _ => return None,
    };
    if samples.len() != pixels.checked_mul(channels)? {
        return None;
    }

    let mut rgba = Vec::with_capacity(pixels.checked_mul(PixelImage::BYTES_PER_PIXEL)?);
    match channels {
        1 => {
            for luma in &samples {
                rgba.extend_from_slice(&[*luma, *luma, *luma, u8::MAX]);
            }
        }
        2 => {
            for pair in samples.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pair[0], pair[0], pair[0], pair[1]]);
            }
        }
        3 => {
            for rgb in samples.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], u8::MAX]);
            }
        }
        _ => rgba.extend_from_slice(&samples),
    }
    Some((frame_width, frame_height, rgba))
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
