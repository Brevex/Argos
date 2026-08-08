//! Synthetic image and disk builders (`test-util` only).
//!
//! Builders produce structurally valid JPEG/PNG bytes plus deliberately
//! corrupt variants, and compose them into filler "disks" for end-to-end
//! carving tests. Content is entirely synthetic — never real photographs.

use miniz_oxide::deflate::compress_to_vec_zlib;

/// A synthetic baseline JPEG, configurable enough to exercise every branch of
/// the validator: restart markers, EXIF thumbnails, progressive-style extra
/// segments.
#[derive(Clone, Debug, Default)]
pub struct Jpeg {
    restart_interval: Option<u16>,
    thumbnail: Option<Vec<u8>>,
    entropy_bytes: usize,
}

impl Jpeg {
    /// A minimal valid JPEG: JFIF header, tables, one scan, EOI.
    #[must_use]
    pub fn new() -> Self {
        Self {
            restart_interval: None,
            thumbnail: None,
            entropy_bytes: 2048,
        }
    }

    /// Emits a `DRI` segment and cyclic `RST0..RST7` markers in the scan.
    #[must_use]
    pub fn with_restart_interval(mut self, interval: u16) -> Self {
        self.restart_interval = Some(interval);
        self
    }

    /// Embeds `thumbnail` (itself JPEG bytes) in an EXIF `APP1` segment.
    #[must_use]
    pub fn with_exif_thumbnail(mut self, thumbnail: Vec<u8>) -> Self {
        self.thumbnail = Some(thumbnail);
        self
    }

    /// Sets the entropy-coded payload size in bytes.
    #[must_use]
    pub fn with_entropy_bytes(mut self, entropy_bytes: usize) -> Self {
        self.entropy_bytes = entropy_bytes;
        self
    }

    /// Builds the JPEG byte stream.
    #[must_use]
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8]; // SOI

        // APP0 / JFIF 1.1, no thumbnail.
        segment(&mut out, 0xE0, &{
            let mut app0 = b"JFIF\0".to_vec();
            app0.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
            app0
        });

        if let Some(thumb) = &self.thumbnail {
            segment(&mut out, 0xE1, &exif_app1_payload(thumb));
        }

        // DQT: table 0, 64 unit coefficients.
        segment(&mut out, 0xDB, &{
            let mut dqt = vec![0x00];
            dqt.extend_from_slice(&[1_u8; 64]);
            dqt
        });

        // SOF0: 8-bit precision, 8x8, one component.
        segment(
            &mut out,
            0xC0,
            &[0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00],
        );

        // DHT: DC table 0 with a single one-bit code.
        segment(&mut out, 0xC4, &{
            let mut dht = vec![0x00, 0x01];
            dht.extend_from_slice(&[0_u8; 15]);
            dht.push(0x00);
            dht
        });

        if let Some(interval) = self.restart_interval {
            segment(&mut out, 0xDD, &interval.to_be_bytes());
        }

        // SOS: one component, spectral selection 0..63.
        segment(&mut out, 0xDA, &[0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);

        self.push_entropy(&mut out);
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        out
    }

    /// Entropy payload: high-entropy bytes the way a real compressed scan is,
    /// with genuine stuffed `0xFF 0x00` pairs and cyclic restart markers when
    /// an interval is set.
    ///
    /// The payload has to be *statistically* realistic, not merely valid: a
    /// low-entropy filler would classify as something other than a JPEG scan
    /// and would let a wrong splice glide through the entropy scan, so tests
    /// built on it would flatter both the classifier and reassembly.
    fn push_entropy(&self, out: &mut Vec<u8>) {
        let mut rst = 0_u8;
        let mut noise = Noise::new(0x9E37_79B9_7F4A_7C15);
        for i in 0..self.entropy_bytes {
            if i > 0 && i % 97 == 0 {
                out.extend_from_slice(&[0xFF, 0x00]);
            } else if self.restart_interval.is_some() && i > 0 && i % 331 == 0 {
                out.extend_from_slice(&[0xFF, 0xD0 + rst]);
                rst = (rst + 1) % 8;
            } else {
                // Any byte but 0xFF: a literal 0xFF must be stuffed, and the
                // stuffing above is where that is exercised deliberately.
                let byte = noise.next_byte();
                out.push(if byte == 0xFF { 0xFE } else { byte });
            }
        }
    }
}

/// A structurally valid truecolour PNG of `width` x `height` with a patterned
/// payload and a real zlib-compressed `IDAT`.
///
/// # Panics
///
/// Panics if either dimension is zero — a fixture bug, not a medium condition.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "fixture byte patterns intentionally wrap modulo small primes"
)]
pub fn png(width: u32, height: u32) -> Vec<u8> {
    assert!(
        width > 0 && height > 0,
        "png fixture dimensions must be non-zero, got {width}x{height}"
    );
    let mut out = crate::PNG_SIGNATURE.to_vec();

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // 8-bit truecolour, deflate, adaptive filtering, no interlace.
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &ihdr);

    let row_bytes = width as usize * 3;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter: none
        for x in 0..row_bytes {
            raw.push(((x * 7 + y * 13) % 251) as u8);
        }
    }
    chunk(&mut out, *b"IDAT", &compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// Wraps already-filtered raw scanlines into a truecolour PNG.
///
/// # Panics
///
/// Panics if either dimension is zero.
#[must_use]
pub fn png_from_raw(width: u32, height: u32, raw: &[u8]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a png needs both dimensions");
    let mut out = crate::PNG_SIGNATURE.to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &ihdr);
    chunk(&mut out, *b"IDAT", &compress_to_vec_zlib(raw, 6));
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// `bytes` cut off after `keep` bytes — truncation at an arbitrary boundary.
///
/// # Panics
///
/// Panics if `keep` exceeds the input length, with both values.
#[must_use]
pub fn truncated(bytes: &[u8], keep: usize) -> Vec<u8> {
    assert!(
        keep <= bytes.len(),
        "cannot keep {keep} bytes of a {}-byte fixture",
        bytes.len()
    );
    bytes[..keep].to_vec()
}

/// `bytes` with a big-endian `u16` written at `at` — for overflowing 16-bit
/// length/count fields.
///
/// # Panics
///
/// Panics if the write runs past the end, with the values.
#[must_use]
pub fn with_u16_be(bytes: &[u8], at: usize, value: u16) -> Vec<u8> {
    assert!(
        at + 2 <= bytes.len(),
        "cannot write u16 at {at} of a {}-byte fixture",
        bytes.len()
    );
    let mut out = bytes.to_vec();
    out[at..at + 2].copy_from_slice(&value.to_be_bytes());
    out
}

/// `bytes` with a big-endian `u32` written at `at` — for overflowing 32-bit
/// length fields.
///
/// # Panics
///
/// Panics if the write runs past the end, with the values.
#[must_use]
pub fn with_u32_be(bytes: &[u8], at: usize, value: u32) -> Vec<u8> {
    assert!(
        at + 4 <= bytes.len(),
        "cannot write u32 at {at} of a {}-byte fixture",
        bytes.len()
    );
    let mut out = bytes.to_vec();
    out[at..at + 4].copy_from_slice(&value.to_be_bytes());
    out
}

/// A buffer of `len` zero bytes — the zero-filled structure variant.
#[must_use]
pub fn zero_filled(len: usize) -> Vec<u8> {
    vec![0_u8; len]
}

/// An EXIF `APP1` payload (after the JPEG segment length) embedding
/// `thumbnail`; `cyclic` makes IFD0's next-IFD pointer point back at IFD0,
/// the crafted chain a bounded walker must terminate on.
#[must_use]
pub fn exif_payload(thumbnail: &[u8], cyclic: bool) -> Vec<u8> {
    let mut payload = exif_app1_payload(thumbnail);
    if cyclic {
        // The next-IFD pointer sits after IFD0's single 12-byte entry:
        // TIFF offset 8 (IFD0) + 2 (count) + 12 (entry) = 22; +6 EXIF header.
        const NEXT_IFD_PTR: usize = 6 + 22;
        payload[NEXT_IFD_PTR..NEXT_IFD_PTR + 4].copy_from_slice(&8_u32.to_le_bytes());
    }
    payload
}

/// `bytes` with the byte at `at` bit-flipped — a single-byte corruption.
///
/// # Panics
///
/// Panics if `at` is out of range, with both values.
#[must_use]
pub fn with_flipped_byte(bytes: &[u8], at: usize) -> Vec<u8> {
    assert!(
        at < bytes.len(),
        "cannot flip byte {at} of a {}-byte fixture",
        bytes.len()
    );
    let mut out = bytes.to_vec();
    out[at] ^= 0xFF;
    out
}

/// A synthetic medium: patterned filler guaranteed free of image signatures,
/// with images placed at chosen offsets.
#[derive(Clone, Debug)]
pub struct Disk {
    data: Vec<u8>,
}

impl Disk {
    /// A disk of `len` filler bytes containing no `0xFF` and no PNG
    /// signature lead byte, so only placed images produce signature hits.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "fixture byte patterns intentionally wrap modulo small primes"
    )]
    pub fn filled(len: usize) -> Self {
        let data = (0..len)
            .map(|i| {
                let byte = ((i * 37 + 11) % 251) as u8;
                if byte == 0xFF || byte == 0x89 {
                    0x00
                } else {
                    byte
                }
            })
            .collect();
        Self { data }
    }

    /// Places `bytes` at `offset`.
    ///
    /// # Panics
    ///
    /// Panics if the placement runs past the end of the disk, with the values.
    #[must_use]
    pub fn with(mut self, offset: usize, bytes: &[u8]) -> Self {
        assert!(
            offset + bytes.len() <= self.data.len(),
            "placement at {offset}+{} runs past the {}-byte disk",
            bytes.len(),
            self.data.len()
        );
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        self
    }

    /// A disk of `len` pseudo-random bytes: what free space on a used medium
    /// actually looks like.
    ///
    /// Unlike [`Disk::filled`], this contains `0xFF` bytes followed by
    /// arbitrary ones, so it breaks a JPEG entropy scan the way real
    /// unrelated data does. That is what makes a fragmented fixture actually
    /// fragmented: filler with no `0xFF` would let the scan glide across the
    /// gap and the image would carve as if it were contiguous.
    ///
    /// File *signatures* are scrubbed so only deliberately placed images
    /// produce header hits; footers are left alone, because false footers are
    /// a real condition reassembly has to cope with.
    #[must_use]
    pub fn noisy(len: usize, seed: u64) -> Self {
        let mut noise = Noise::new(seed);
        let mut data: Vec<u8> = (0..len).map(|_| noise.next_byte()).collect();
        for index in 0..data.len().saturating_sub(1) {
            // Break any accidental JPEG SOI, keeping the byte high-entropy.
            if data[index] == 0xFF && data[index + 1] == 0xD8 {
                data[index + 1] = 0x55;
            }
            // Break any accidental PNG signature lead.
            if data[index] == 0x89 && data.get(index + 1) == Some(&b'P') {
                data[index + 1] = 0x51;
            }
        }
        Self { data }
    }

    /// The composed disk image.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// A small deterministic `xorshift64*` source, so fixture bytes look like
/// compressed data while staying identical between runs.
#[derive(Clone, Debug)]
pub struct Noise {
    state: u64,
}

impl Noise {
    /// A generator seeded with `seed`; any non-zero seed works.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// The next pseudo-random byte.
    pub fn next_byte(&mut self) -> u8 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        u8::try_from(self.state >> 56).unwrap_or(0)
    }
}

/// Appends a JPEG marker segment: `FF <code> <len> <payload>`.
fn segment(out: &mut Vec<u8>, code: u8, payload: &[u8]) {
    out.extend_from_slice(&[0xFF, code]);
    let len = u16::try_from(payload.len() + 2)
        .unwrap_or_else(|_| panic!("segment payload of {} bytes exceeds u16", payload.len()));
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
}

/// Appends a PNG chunk with its CRC.
fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len())
        .unwrap_or_else(|_| panic!("chunk data of {} bytes exceeds u32", data.len()));
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// An EXIF `APP1` payload (little-endian TIFF, IFD0 → IFD1) embedding
/// `thumbnail` via the `JPEGInterchangeFormat` tags.
fn exif_app1_payload(thumbnail: &[u8]) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II");
    tiff.extend_from_slice(&42_u16.to_le_bytes());
    tiff.extend_from_slice(&8_u32.to_le_bytes()); // IFD0 offset

    // IFD0: one entry (orientation), then the pointer to IFD1 at offset 26.
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    ifd_entry(&mut tiff, 0x0112, 3, 1);
    tiff.extend_from_slice(&26_u32.to_le_bytes());

    // IFD1 at 26: thumbnail offset/length entries; data starts at 56.
    let thumb_offset = 56_u32;
    tiff.extend_from_slice(&2_u16.to_le_bytes());
    ifd_entry(&mut tiff, 0x0201, 4, thumb_offset);
    ifd_entry(
        &mut tiff,
        0x0202,
        4,
        u32::try_from(thumbnail.len())
            .unwrap_or_else(|_| panic!("thumbnail of {} bytes exceeds u32", thumbnail.len())),
    );
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    debug_assert_eq!(tiff.len(), thumb_offset as usize);
    tiff.extend_from_slice(thumbnail);

    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    payload
}

/// Appends one 12-byte IFD entry with an inline value.
fn ifd_entry(tiff: &mut Vec<u8>, tag: u16, kind: u16, value: u32) {
    tiff.extend_from_slice(&tag.to_le_bytes());
    tiff.extend_from_slice(&kind.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&value.to_le_bytes());
}

// --- fragmented layouts ----------------------------------------------------

/// A fragmented image planted on a disk, with the ground truth to check
/// against.
#[derive(Clone, Debug)]
pub struct Fragmented {
    /// The whole image, as it was before being split.
    pub image: Vec<u8>,
    /// Where each fragment landed, in file order — the answer a reassembly
    /// has to arrive at.
    pub extents: Vec<argos_core::geometry::ByteRange>,
    /// The composed disk.
    pub disk: Vec<u8>,
}

impl Fragmented {
    /// The disk as a seekable source.
    #[must_use]
    pub fn source(&self) -> std::io::Cursor<&[u8]> {
        std::io::Cursor::new(&self.disk)
    }

    /// Concatenates the planted fragments, so a test can assert that its
    /// recovered extents describe these exact bytes.
    #[must_use]
    pub fn planted_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.image.len());
        for extent in &self.extents {
            let start = usize::try_from(extent.start.get()).unwrap_or(usize::MAX);
            let len = usize::try_from(extent.len).unwrap_or(0);
            out.extend_from_slice(&self.disk[start..start + len]);
        }
        out
    }
}

/// Splits `image` into fragments starting at `starts` and plants them on a
/// disk of `len` bytes.
///
/// Every fragment but the last is a whole number of `block` bytes, which is
/// how a filesystem allocates, and therefore what a reassembly search assumes.
/// `starts` is in **file order**: the second entry holds the bytes that follow
/// the first, wherever it sits on the medium, so a test can deliberately place
/// a later fragment at a lower offset.
///
/// # Panics
///
/// Panics if the fragments do not fit, overlap, or if `starts` is empty.
#[must_use]
pub fn fragmented(len: usize, image: &[u8], starts: &[usize], block: usize) -> Fragmented {
    use argos_core::geometry::{ByteOffset, ByteRange};

    assert!(!starts.is_empty(), "a fragmented layout needs a start");
    assert!(block > 0, "the block size must be positive");
    for &start in starts {
        assert!(
            start % block == 0,
            "fragment start {start} is not a multiple of the {block}-byte block"
        );
    }

    // Every fragment but the last is a whole number of blocks; the remainder
    // is the tail.
    let pieces = starts.len();
    let per_piece = (image.len() / pieces / block).max(1) * block;
    let mut extents = Vec::with_capacity(pieces);
    let mut disk = Disk::noisy(len, 0x5DEE_CE66_D000_0001);
    let mut at = 0_usize;
    for (index, &start) in starts.iter().enumerate() {
        let take = if index + 1 == pieces {
            image.len() - at
        } else {
            per_piece.min(image.len() - at)
        };
        assert!(take > 0, "fragment {index} would be empty");
        disk = disk.with(start, &image[at..at + take]);
        extents.push(ByteRange::new(ByteOffset::new(start as u64), take as u64));
        at += take;
    }
    assert_eq!(at, image.len(), "the fragments must cover the whole image");

    Fragmented {
        image: image.to_vec(),
        extents,
        disk: disk.into_bytes(),
    }
}

// --- decodable images ------------------------------------------------------

/// A real, decodable JPEG of `width` x `height` with photograph-like content.
///
/// Unlike [`Jpeg`], which builds a structurally valid stream to exercise the
/// state machine, this is an actual encoded image: it survives a full pixel
/// decode. Reassembly is accepted on a real decode, so any test that measures
/// reassembly has to work on images that can be decoded.
///
/// The content is smooth gradients with gentle texture — what a photograph
/// looks like to the vertical-difference measure reassembly ranks by — so a
/// correct assembly scores far below a spliced one.
///
/// # Panics
///
/// Panics if either dimension is zero, or if encoding fails, which would be a
/// fixture bug rather than a medium condition.
#[must_use]
pub fn photo_jpeg(width: u16, height: u16, seed: u64) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a photo needs both dimensions");
    let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height));
    let mut noise = Noise::new(seed);
    // The subject has to depend on the seed, not just the grain. Two fixtures
    // that differ only in texture are effectively the same photograph, and a
    // splice between them has no seam to find — which would let a reassembly
    // test pass while proving nothing about telling two files apart.
    let subject = Noise::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut subject = subject;
    let slope_x = u32::from(subject.next_byte()) % 200 + 25;
    let slope_y = u32::from(subject.next_byte()) % 200 + 25;
    let offset = u32::from(subject.next_byte());
    let bands = u32::from(subject.next_byte()) % 5 + 1;
    for y in 0..u32::from(height) {
        for x in 0..u32::from(width) {
            // Smooth gradients so adjacent rows stay similar — the property a
            // seam breaks — but with a subject the seed chooses.
            let base = (x * slope_x / u32::from(width)
                + y * slope_y / u32::from(height)
                + offset
                + (x * bands / u32::from(width)) * 40)
                % 256;
            let texture = u32::from(noise.next_byte()) / 24;
            pixels.push(u8::try_from((base + texture) % 256).unwrap_or(0));
        }
    }

    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 85);
    encoder
        .encode(&pixels, width, height, jpeg_encoder::ColorType::Luma)
        .expect("encoding a fixture photo must succeed");
    out
}

/// A real, decodable colour JPEG with chroma subsampling.
///
/// Three components with 2x2 luma sampling is what a camera writes, and it is
/// the interleaved-MCU geometry a single-component fixture never exercises.
///
/// # Panics
///
/// Panics if either dimension is zero, or if encoding fails.
#[must_use]
pub fn photo_jpeg_rgb(width: u16, height: u16, seed: u64) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a photo needs both dimensions");
    let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height) * 3);
    let mut noise = Noise::new(seed);
    for y in 0..u32::from(height) {
        for x in 0..u32::from(width) {
            let base = (x * 200 / u32::from(width) + y * 40 / u32::from(height)) % 256;
            let texture = u32::from(noise.next_byte()) / 24;
            let value = u8::try_from((base + texture) % 256).unwrap_or(0);
            pixels.extend_from_slice(&[value, value.wrapping_add(17), value.wrapping_sub(23)]);
        }
    }

    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 85);
    encoder
        .encode(&pixels, width, height, jpeg_encoder::ColorType::Rgb)
        .expect("encoding a fixture photo must succeed");
    out
}

/// A real PNG whose `IDAT` payload is incompressible, the way photographic
/// data is.
///
/// [`png`] builds a patterned image that compresses to almost nothing, which
/// makes its payload low-entropy and unrepresentative of what a block
/// classifier meets on a medium.
///
/// # Panics
///
/// Panics if either dimension is zero.
#[must_use]
pub fn noisy_png(width: u32, height: u32, seed: u64) -> Vec<u8> {
    assert!(width > 0 && height > 0, "a png needs both dimensions");
    let mut noise = Noise::new(seed);
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for _ in 0..height {
        // Filter type 0 (none), then the row's pixels.
        raw.extend_from_slice(&[0]);
        for _ in 0..width {
            raw.extend_from_slice(&[noise.next_byte(), noise.next_byte(), noise.next_byte()]);
        }
    }
    png_from_raw(width, height, &raw)
}

/// Absolute offset of the first `IDAT` chunk's payload in a PNG.
///
/// # Panics
///
/// Panics if `bytes` holds no `IDAT` chunk — a fixture bug.
#[must_use]
pub fn idat_payload_offset(bytes: &[u8]) -> usize {
    let mut at = 8; // past the signature
    while at + 8 <= bytes.len() {
        let len =
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let kind = &bytes[at + 4..at + 8];
        if kind == b"IDAT" {
            return at + 8;
        }
        at += 12 + len;
    }
    panic!("the fixture PNG has no IDAT chunk");
}
