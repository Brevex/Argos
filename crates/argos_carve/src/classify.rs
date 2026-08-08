//! Block classification: what a block of the medium probably is.
//!
//! Reassembly is a search over which blocks join which file, and the search
//! space is the whole surface. Classification is what makes it tractable: a
//! cheap feature vector per block partitions the medium into plausible image
//! blocks and everything else, so a graph walk considers thousands of
//! candidates instead of billions (Garfinkel, DFRWS 2007; Fitzgerald et al.
//! on fragment classification).
//!
//! Nothing here decides a recovery. A class is a *hint* that gates which
//! blocks enter a reassembly graph; every extent that survives the graph is
//! still validated by the real decoder before it can be reported.
//!
//! This runs over every byte of the medium, so it makes exactly one pass over
//! a block, allocates nothing, and uses integer arithmetic throughout.

/// Bytes per classified block.
///
/// 4 KiB is the smallest allocation unit any filesystem Argos handles uses, so
/// a fragment boundary always falls on a multiple of it; smaller blocks would
/// classify sub-fragment noise, larger ones would straddle boundaries.
pub const BLOCK_BYTES: usize = 4096;

/// Shortest block worth classifying. Below this the byte histogram is too
/// sparse for entropy to mean anything.
pub const MIN_BLOCK_BYTES: usize = 256;

/// Entropy at or above which a block is treated as compressed or encrypted.
///
/// Shannon entropy of 8-bit symbols runs 0..=8 bits. Compressed data measures
/// ~7.9; natural text and structured records stay below ~6. The gap is wide,
/// so 7.0 separates them without being sensitive to where exactly it sits.
pub const HIGH_ENTROPY_BITS: f32 = 7.0;

/// Entropy at or below which a block is treated as sparse or padding.
pub const LOW_ENTROPY_BITS: f32 = 2.0;

/// Minimum `0xFF 0x00` stuffing occurrences per block before the JPEG-stream
/// detector will claim a block.
///
/// A JPEG entropy stream must escape every `0xFF` it emits, and `0xFF` occurs
/// about once per 256 bytes in compressed data, so a 4 KiB block of real
/// entropy-coded scan holds ~16 stuffed pairs. Requiring several keeps random
/// high-entropy data from passing.
const MIN_STUFFING_PER_BLOCK: u32 = 4;

/// Fraction of `0xFF` bytes that must be legal (stuffing or a restart marker)
/// for a block to read as a JPEG entropy stream, in percent.
///
/// Inside a scan every `0xFF` is followed by `0x00` or an `RSTn`; anything
/// else ends the scan. A block with many `0xFF`s followed by arbitrary bytes
/// is not scan data.
const MIN_LEGAL_FF_PERCENT: u32 = 80;

/// zlib compression method nibble for deflate. Source: RFC 1950 §2.2.
const ZLIB_DEFLATE_METHOD: u8 = 8;

/// What a block of the medium looks like.
///
/// Ordering is by usefulness to reassembly, weakest first, so the most
/// promising class compares greatest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockClass {
    /// Zeros, padding or a long run of one byte: never file content.
    LowEntropy,
    /// Readable text or structured records: not image payload.
    TextOrSparse,
    /// Compressed or encrypted, but not recognisably image payload.
    #[default]
    HighEntropy,
    /// A zlib/deflate stream — PNG `IDAT` payload looks like this.
    Deflate,
    /// A JPEG entropy-coded scan.
    JpegStream,
}

impl BlockClass {
    /// Whether a block of this class can be part of a reassembled image.
    ///
    /// Deliberately permissive: excluding a block wrongly loses a recovery,
    /// while including one wrongly only costs search time, because the
    /// decoder still has to accept it.
    #[must_use]
    pub const fn can_hold_image_data(self) -> bool {
        matches!(self, Self::JpegStream | Self::Deflate | Self::HighEntropy)
    }
}

impl std::fmt::Display for BlockClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::LowEntropy => "low-entropy",
            Self::TextOrSparse => "text-or-sparse",
            Self::HighEntropy => "high-entropy",
            Self::Deflate => "deflate",
            Self::JpegStream => "jpeg-stream",
        };
        f.write_str(name)
    }
}

/// One block's classification and the measurements behind it.
///
/// The measurements are kept so a caller can rank blocks within a class rather
/// than only between classes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockProfile {
    /// What the block looks like.
    pub class: BlockClass,
    /// Confidence in the class, 0.0 to 1.0.
    pub score: f32,
    /// Shannon entropy of the block in bits per byte, 0.0 to 8.0.
    pub entropy: f32,
    /// `0xFF 0x00` stuffing pairs found.
    pub stuffing: u32,
    /// Restart markers found in cyclic order.
    pub restarts: u32,
}

impl Default for BlockProfile {
    fn default() -> Self {
        Self {
            class: BlockClass::LowEntropy,
            score: 0.0,
            entropy: 0.0,
            stuffing: 0,
            restarts: 0,
        }
    }
}

/// Classifies one block.
///
/// A block shorter than [`MIN_BLOCK_BYTES`] is reported as
/// [`BlockClass::LowEntropy`] with a zero score: there is not enough of it to
/// measure, and claiming otherwise would put noise into a reassembly graph.
#[must_use]
pub fn classify(block: &[u8]) -> BlockProfile {
    if block.len() < MIN_BLOCK_BYTES {
        return BlockProfile::default();
    }

    let mut histogram = [0_u32; 256];
    for &byte in block {
        histogram[usize::from(byte)] += 1;
    }
    let entropy = shannon_entropy(&histogram, block.len());
    let jpeg = jpeg_stream_features(block);

    // A JPEG scan is high-entropy data with a signature no other high-entropy
    // data has: every 0xFF escaped, and restart markers in cyclic order.
    if entropy >= HIGH_ENTROPY_BITS
        && jpeg.stuffing >= MIN_STUFFING_PER_BLOCK
        && jpeg.legal_percent() >= MIN_LEGAL_FF_PERCENT
    {
        return BlockProfile {
            class: BlockClass::JpegStream,
            score: confidence_from(jpeg.legal_percent()),
            entropy,
            stuffing: jpeg.stuffing,
            restarts: jpeg.restarts,
        };
    }

    if entropy >= HIGH_ENTROPY_BITS && starts_zlib_stream(block) {
        return BlockProfile {
            class: BlockClass::Deflate,
            score: 0.75,
            entropy,
            stuffing: jpeg.stuffing,
            restarts: jpeg.restarts,
        };
    }

    let class = if entropy <= LOW_ENTROPY_BITS {
        BlockClass::LowEntropy
    } else if entropy >= HIGH_ENTROPY_BITS {
        BlockClass::HighEntropy
    } else {
        BlockClass::TextOrSparse
    };
    BlockProfile {
        class,
        score: normalized_entropy(entropy),
        entropy,
        stuffing: jpeg.stuffing,
        restarts: jpeg.restarts,
    }
}

/// Shannon entropy of a byte histogram, in bits per byte.
fn shannon_entropy(histogram: &[u32; 256], len: usize) -> f32 {
    if len == 0 {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "entropy is a heuristic score; f32 precision is far beyond what it needs"
    )]
    let total = len as f32;
    let mut entropy = 0.0_f32;
    for &count in histogram {
        if count == 0 {
            continue;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a per-block symbol count is far below f32's exact-integer range"
        )]
        let probability = count as f32 / total;
        entropy -= probability * probability.log2();
    }
    entropy
}

/// Entropy scaled to 0.0..=1.0, for use as a score.
fn normalized_entropy(entropy: f32) -> f32 {
    (entropy / 8.0).clamp(0.0, 1.0)
}

/// A percentage turned into a 0.0..=1.0 score.
fn confidence_from(percent: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a percentage is exactly representable in f32"
    )]
    let value = percent as f32 / 100.0;
    value.clamp(0.0, 1.0)
}

/// What the JPEG-stream detector measured in a block.
struct JpegFeatures {
    /// `0xFF 0x00` pairs: the escape a scan must use for a literal `0xFF`.
    stuffing: u32,
    /// Restart markers appearing in cyclic `RST0..RST7` order.
    restarts: u32,
    /// `0xFF` bytes followed by something a scan does not allow.
    illegal: u32,
}

impl JpegFeatures {
    /// Percentage of `0xFF` bytes that a scan would accept.
    fn legal_percent(&self) -> u32 {
        let legal = self.stuffing.saturating_add(self.restarts);
        let total = legal.saturating_add(self.illegal);
        if total == 0 {
            return 0;
        }
        legal.saturating_mul(100) / total
    }
}

/// Measures the `0xFF` handling that distinguishes an entropy-coded scan from
/// any other high-entropy data (T.81 §B.2.1: stuffing and restart cadence).
fn jpeg_stream_features(block: &[u8]) -> JpegFeatures {
    let mut features = JpegFeatures {
        stuffing: 0,
        restarts: 0,
        illegal: 0,
    };
    // Restart markers cycle RST0..RST7; a block starts mid-cycle, so the first
    // one seen sets the phase and later ones must follow it.
    let mut expected: Option<u8> = None;

    let mut index = 0_usize;
    while index + 1 < block.len() {
        if block[index] != 0xFF {
            index += 1;
            continue;
        }
        let next = block[index + 1];
        match next {
            0x00 => {
                features.stuffing += 1;
                index += 2;
            }
            // RST0..RST7. Source: T.81 Table B.1.
            0xD0..=0xD7 => {
                let marker = next - 0xD0;
                match expected {
                    None => features.restarts += 1,
                    Some(want) if want == marker => features.restarts += 1,
                    Some(_) => features.illegal += 1,
                }
                expected = Some((marker + 1) % 8);
                index += 2;
            }
            // A fill byte before another marker is legal; keep scanning from it.
            0xFF => index += 1,
            _ => {
                features.illegal += 1;
                index += 2;
            }
        }
    }
    features
}

/// Whether a block opens with a plausible zlib header (RFC 1950 §2.2).
///
/// PNG `IDAT` payload is a zlib stream, so this recognises the first block of
/// an image's compressed data. Later blocks are raw deflate with no header and
/// fall through to the entropy classes, which is why
/// [`BlockClass::HighEntropy`] still enters reassembly.
fn starts_zlib_stream(block: &[u8]) -> bool {
    let (Some(&cmf), Some(&flg)) = (block.first(), block.get(1)) else {
        return false;
    };
    if cmf & 0x0F != ZLIB_DEFLATE_METHOD {
        return false;
    }
    // The window size must be one deflate defines, and the header's two bytes
    // form a multiple of 31. Source: RFC 1950 §2.2.
    if cmf >> 4 > 7 {
        return false;
    }
    (u16::from(cmf) * 256 + u16::from(flg)).is_multiple_of(31)
}
