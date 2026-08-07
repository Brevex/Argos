//! Builds a multi-gibibyte fixture disk for throughput measurement.
//!
//! ```text
//! cargo run --release -p argos_carve --features test-util \
//!     --example gen_bench_disk -- /tmp/disk.img 2
//! ```
//!
//! The filler is pseudo-random so the surface is incompressible and the
//! signature detector sees a realistic byte distribution — including the false
//! `FF D8 FF` hits a real medium produces, which is what exercises the
//! validation stage. Images are planted at regular intervals so a scan of the
//! result must cover the whole surface to find them all.

use std::io::{BufWriter, Write};

/// Bytes written per filler round.
const ROUND_BYTES: usize = 1024 * 1024;

/// Filler rounds between planted images.
const ROUNDS_PER_IMAGE: usize = 32;

/// Buffer the image file is written through.
const WRITE_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Entropy-coded payload of the planted JPEG, so it is a realistic size.
const JPEG_ENTROPY_BYTES: usize = 200_000;

/// Edge length of the planted PNG.
const PNG_EDGE: u32 = 256;

/// `xorshift64` seed. Any non-zero value works; fixed so runs are comparable.
const SEED: u64 = 0x2545_F491_4F6C_DD1D;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(gib)) = (args.next(), args.next()) else {
        eprintln!("usage: gen_bench_disk <path> <gibibytes>");
        std::process::exit(2);
    };
    let gib: usize = gib.parse().unwrap_or(2);
    let total = gib * 1024 * 1024 * 1024;

    let jpeg = argos_carve::fixture::Jpeg::new()
        .with_entropy_bytes(JPEG_ENTROPY_BYTES)
        .build();
    let png = argos_carve::fixture::png(PNG_EDGE, PNG_EDGE);

    let mut out = BufWriter::with_capacity(WRITE_BUFFER_BYTES, std::fs::File::create(&path)?);
    let mut state = SEED;
    let mut filler = vec![0_u8; ROUND_BYTES];
    let mut written = 0_usize;
    let mut planted = 0_usize;

    for round in 1_usize.. {
        if written >= total {
            break;
        }
        for byte in &mut filler {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = u8::try_from(state >> 56).unwrap_or(0);
        }
        out.write_all(&filler)?;
        written += filler.len();

        if round.is_multiple_of(ROUNDS_PER_IMAGE) {
            let image = if planted.is_multiple_of(2) {
                &jpeg
            } else {
                &png
            };
            out.write_all(image)?;
            written += image.len();
            planted += 1;
        }
    }
    out.flush()?;
    eprintln!("wrote {written} bytes to {path} with {planted} images");
    Ok(())
}
