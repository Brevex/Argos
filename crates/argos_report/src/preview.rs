//! Previews: a small, derived rendering of a recovered artifact.
//!
//! A preview is not evidence. It carries no provenance of its own, it is
//! reproducible from the artifact at any time, and nothing in the manifest
//! depends on it. It exists so a viewer can show a thousand recoveries without
//! decoding a thousand full-resolution images, and so a reviewer can tell a
//! photograph from an icon at a glance.
//!
//! Because it is derived, everything here is lossy on purpose and none of it
//! feeds back: no measurement taken from a preview reaches a confidence tier,
//! a triage score or a manifest field other than the preview's own path.

use argos_core::classify::PixelImage;

/// Longest edge of a preview, in pixels.
///
/// Large enough to recognise a photograph in a gallery, small enough that a
/// directory of thousands costs less than one recovered image.
pub(crate) const MAX_EDGE: u32 = 256;

/// JPEG quality of a preview. Previews are looked at, never analysed, and past
/// this the extra bytes stop being visible at this size.
const QUALITY: u8 = 80;

/// Channels in the encoder's input.
const RGB_CHANNELS: usize = 3;

/// Full opacity, and the divisor of the alpha composite below.
const OPAQUE: u32 = 255;

/// A downscaled JPEG of `image`, or `None` when it has no pixels to show.
///
/// Transparency is composited over white, so an icon with an alpha channel
/// looks in a gallery the way it looks in a file manager rather than as a
/// black square.
pub(crate) fn encode(image: &PixelImage) -> Option<Vec<u8>> {
    let (width, height) = (image.width(), image.height());
    let (target_width, target_height) = fit(width, height)?;

    let mut rgb = vec![
        0_u8;
        usize::try_from(target_width)
            .ok()?
            .checked_mul(usize::try_from(target_height).ok()?)?
            .checked_mul(RGB_CHANNELS)?
    ];
    resample(image, target_width, target_height, &mut rgb);

    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, QUALITY)
        .encode(
            &rgb,
            u16::try_from(target_width).ok()?,
            u16::try_from(target_height).ok()?,
            jpeg_encoder::ColorType::Rgb,
        )
        .ok()?;
    Some(out)
}

/// The preview's dimensions: `width`×`height` scaled to fit [`MAX_EDGE`],
/// never enlarged, never collapsed to zero.
///
/// `None` for an image with no pixels — there is nothing to show, and the
/// division below would have no defined answer.
fn fit(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let longest = width.max(height);
    if longest <= MAX_EDGE {
        return Some((width, height));
    }
    // In `u64`: the products below exceed `u32` for any image wider than
    // 16 megapixels on one edge, and the medium decides these numbers
    // (A-UNTRUSTED-ONDISK).
    let scale = |edge: u32| -> u32 {
        let scaled = u64::from(edge) * u64::from(MAX_EDGE) / u64::from(longest);
        u32::try_from(scaled).unwrap_or(MAX_EDGE).max(1)
    };
    Some((scale(width), scale(height)))
}

/// Averages each target pixel over the source box it covers.
///
/// Box sampling rather than point sampling: a photograph point-sampled to a
/// sixteenth of its width aliases into something that can look like a
/// synthetic asset, and a preview that misrepresents what was recovered is
/// worse than no preview.
fn resample(image: &PixelImage, target_width: u32, target_height: u32, out: &mut [u8]) {
    let (width, height) = (image.width(), image.height());
    let pixels = image.rgba();
    let stride = usize::try_from(width).unwrap_or(0) * PixelImage::BYTES_PER_PIXEL;

    for ty in 0..target_height {
        let (top, bottom) = box_edges(ty, target_height, height);
        for tx in 0..target_width {
            let (left, right) = box_edges(tx, target_width, width);

            let (mut red, mut green, mut blue, mut count) = (0_u64, 0_u64, 0_u64, 0_u64);
            for y in top..bottom {
                let row = usize::try_from(y).unwrap_or(0) * stride;
                for x in left..right {
                    let at = row + usize::try_from(x).unwrap_or(0) * PixelImage::BYTES_PER_PIXEL;
                    let Some(pixel) = pixels.get(at..at + PixelImage::BYTES_PER_PIXEL) else {
                        continue;
                    };
                    let alpha = u32::from(pixel[3]);
                    red += u64::from(over_white(u32::from(pixel[0]), alpha));
                    green += u64::from(over_white(u32::from(pixel[1]), alpha));
                    blue += u64::from(over_white(u32::from(pixel[2]), alpha));
                    count += 1;
                }
            }

            let at = (usize::try_from(ty).unwrap_or(0)
                * usize::try_from(target_width).unwrap_or(0)
                + usize::try_from(tx).unwrap_or(0))
                * RGB_CHANNELS;
            let Some(target) = out.get_mut(at..at + RGB_CHANNELS) else {
                continue;
            };
            // A box with no samples cannot happen — `box_edges` never returns
            // an empty span — but a preview must not panic over a thumbnail.
            let mean = |sum: u64| u8::try_from(sum / count.max(1)).unwrap_or(u8::MAX);
            target[0] = mean(red);
            target[1] = mean(green);
            target[2] = mean(blue);
        }
    }
}

/// The half-open source span target index `at` of `target_len` covers in a
/// source of `source_len` pixels. Never empty, never past the source.
fn box_edges(at: u32, target_len: u32, source_len: u32) -> (u32, u32) {
    let span = |index: u64| -> u32 {
        let scaled = index * u64::from(source_len) / u64::from(target_len.max(1));
        u32::try_from(scaled).unwrap_or(source_len).min(source_len)
    };
    let start = span(u64::from(at));
    let end = span(u64::from(at) + 1)
        .max(start.saturating_add(1))
        .min(source_len);
    (start, end)
}

/// One channel composited over an opaque white background.
fn over_white(channel: u32, alpha: u32) -> u8 {
    let blended = (channel * alpha + OPAQUE * (OPAQUE - alpha)) / OPAQUE;
    u8::try_from(blended).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use argos_core::classify::PixelImage;

    use super::{MAX_EDGE, encode, fit};

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> PixelImage {
        let count = (width as usize) * (height as usize);
        PixelImage::new(width, height, rgba.repeat(count))
    }

    #[test]
    fn a_preview_never_enlarges_a_small_image() {
        // Upscaling a 32×32 icon to 256 would put more pixels in the preview
        // than the artifact has, which says something false about it.
        assert_eq!(fit(32, 32), Some((32, 32)));
        assert_eq!(fit(MAX_EDGE, MAX_EDGE), Some((MAX_EDGE, MAX_EDGE)));
    }

    #[test]
    fn a_large_image_keeps_its_aspect_ratio() {
        assert_eq!(fit(4000, 3000), Some((256, 192)));
        assert_eq!(fit(3000, 4000), Some((192, 256)));
        // An extreme panorama still has a visible edge rather than none.
        let (width, height) = fit(100_000, 10).expect("a panorama has pixels");
        assert_eq!(width, MAX_EDGE);
        assert!(height >= 1, "an edge must not round away to nothing");
    }

    #[test]
    fn an_empty_image_has_no_preview() {
        assert_eq!(fit(0, 100), None);
        assert_eq!(fit(100, 0), None);
        assert!(encode(&solid(0, 0, [0; 4])).is_none());
    }

    #[test]
    fn a_preview_is_a_decodable_jpeg() {
        let bytes = encode(&solid(300, 200, [200, 40, 40, 255])).expect("a solid image encodes");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG starts with SOI");
        assert_eq!(
            &bytes[bytes.len() - 2..],
            &[0xFF, 0xD9],
            "and ends with EOI"
        );
    }

    #[test]
    fn transparency_is_composited_over_white_not_over_black() {
        // A fully transparent asset must not preview as a black rectangle:
        // that is what an overwritten region looks like, and confusing the two
        // in a gallery is exactly the wrong mistake for this tool to make.
        let clear = encode(&solid(64, 64, [0, 0, 0, 0])).expect("a clear image encodes");
        let black = encode(&solid(64, 64, [0, 0, 0, 255])).expect("a black image encodes");
        assert_ne!(clear, black);
    }
}
