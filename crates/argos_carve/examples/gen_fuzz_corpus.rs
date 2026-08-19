//! Writes fixture-built seed inputs into the fuzz corpus directories.
//!
//! Run from the repository root: `cargo run -p argos_carve --example gen_fuzz_corpus`.

use std::fs;
use std::path::Path;

use argos_carve::fixture::{Jpeg, png, truncated, with_flipped_byte};

fn main() -> std::io::Result<()> {
    let base = Path::new("crates/argos_carve/fuzz/corpus");

    let jpeg_dir = base.join("jpeg_validate");
    fs::create_dir_all(&jpeg_dir)?;
    let plain = Jpeg::new().build();
    let thumb = Jpeg::new().with_entropy_bytes(128).build();
    fs::write(jpeg_dir.join("plain"), &plain)?;
    fs::write(
        jpeg_dir.join("restarts"),
        Jpeg::new().with_restart_interval(4).build(),
    )?;
    fs::write(
        jpeg_dir.join("exif"),
        Jpeg::new().with_exif_thumbnail(thumb.clone()).build(),
    )?;
    fs::write(
        jpeg_dir.join("truncated"),
        truncated(&plain, plain.len() / 2),
    )?;
    fs::write(jpeg_dir.join("flipped"), with_flipped_byte(&plain, 40))?;

    let png_dir = base.join("png_validate");
    fs::create_dir_all(&png_dir)?;
    let picture = png(24, 16);
    fs::write(png_dir.join("plain"), &picture)?;
    fs::write(
        png_dir.join("truncated"),
        truncated(&picture, picture.len() / 2),
    )?;
    fs::write(png_dir.join("flipped"), with_flipped_byte(&picture, 40))?;

    let exif_dir = base.join("exif_thumbnail");
    fs::create_dir_all(&exif_dir)?;
    // The fuzz target's input is the TIFF bytes after the "Exif\0\0" prefix.
    let payload = argos_carve::fixture::exif_payload(&thumb, false);
    fs::write(exif_dir.join("tiff"), &payload[6..])?;
    let cyclic = argos_carve::fixture::exif_payload(&thumb, true);
    fs::write(exif_dir.join("cyclic"), &cyclic[6..])?;

    // Description reading, which has two entry points and therefore two kinds
    // of seed: a whole image, where the `APP1` has to be found among marker
    // segments whose lengths come off the medium, and a bare TIFF, where every
    // value is reached through an offset the file states.
    let metadata_dir = base.join("exif_metadata");
    fs::create_dir_all(&metadata_dir)?;
    let camera = Jpeg::new()
        .with_capture("Make", "Model", "2016:07:04 11:22:33", (4128, 3096))
        .build();
    let tiff =
        argos_carve::fixture::exif_capture("Make", "Model", "2016:07:04 11:22:33", (4128, 3096));
    fs::write(metadata_dir.join("camera"), &camera)?;
    fs::write(metadata_dir.join("tiff"), &tiff)?;
    fs::write(
        metadata_dir.join("tiff-truncated"),
        truncated(&tiff, tiff.len() / 2),
    )?;
    // Byte 12 is inside IFD0's first entry, so flipping it moves an offset or
    // a count — the field a walker must not follow blindly.
    fs::write(
        metadata_dir.join("tiff-flipped-offset"),
        with_flipped_byte(&tiff, 12),
    )?;

    // The reassembly oracle. Seeding it with real encoded images matters more
    // than anywhere else here: starting from noise, a fuzzer would spend its
    // budget failing at the frame header and never reach the entropy decoder
    // at all.
    let entropy_dir = base.join("jpeg_entropy_scan");
    fs::create_dir_all(&entropy_dir)?;
    let photo = argos_carve::fixture::photo_jpeg(64, 48, 0x51ED_2A11_0000_0001);
    let colour = argos_carve::fixture::photo_jpeg_rgb(48, 32, 0x0F1E_2D3C_4B5A_6978);
    fs::write(entropy_dir.join("photo"), &photo)?;
    fs::write(entropy_dir.join("colour"), &colour)?;
    fs::write(
        entropy_dir.join("truncated"),
        truncated(&photo, photo.len() / 2),
    )?;
    fs::write(
        entropy_dir.join("flipped-scan"),
        with_flipped_byte(&photo, photo.len() * 3 / 4),
    )?;
    fs::write(entropy_dir.join("restarts"), &plain)?;

    let classify_dir = base.join("block_classify");
    fs::create_dir_all(&classify_dir)?;
    fs::write(classify_dir.join("entropy"), &photo)?;
    fs::write(
        classify_dir.join("deflate"),
        argos_carve::fixture::noisy_png(32, 32, 0xF00D_BEEF),
    )?;
    fs::write(classify_dir.join("zeros"), vec![0_u8; 4096])?;
    fs::write(
        classify_dir.join("noise"),
        argos_carve::fixture::Disk::noisy(4096, 0x2468_ACE0_1357_9BDF).into_bytes(),
    )?;

    // This target reads its header and break offsets from the first five
    // bytes, so a seed is a small control prefix followed by a medium.
    let bifragment_dir = base.join("reassemble_bifragment");
    fs::create_dir_all(&bifragment_dir)?;
    let mut seed = vec![0_u8, 0, 0x40, 0, 0];
    seed.extend_from_slice(&photo);
    fs::write(bifragment_dir.join("photo"), &seed)?;
    // Large enough that both fragments get a whole block.
    let block = argos_carve::classify::BLOCK_BYTES;
    let splittable = argos_carve::fixture::photo_jpeg(320, 240, 0x2468_ACE0_1357_9BDF);
    let mut split = vec![0_u8, 0, 0x40, 0, 0];
    split.extend_from_slice(
        &argos_carve::fixture::fragmented(32 * block, &splittable, &[block, 8 * block], block).disk,
    );
    fs::write(bifragment_dir.join("fragmented"), &split)?;

    let scan_dir = base.join("carver_scan");
    fs::create_dir_all(&scan_dir)?;
    let disk = argos_carve::fixture::Disk::filled(64 * 1024)
        .with(1_000, &plain)
        .with(30_000, &png(8, 8))
        .into_bytes();
    fs::write(scan_dir.join("disk"), &disk)?;

    decode_rgba_seeds(base, &plain, &picture)?;

    println!("seed corpus written under {}", base.display());
    Ok(())
}

/// Seeds for the pixel-decoding target.
///
/// `decode_rgba` takes a format selector byte in front of the image, so a
/// seed has to carry one. Both formats and both failure shapes the decoder
/// has to survive — a truncated stream and a header claiming more pixels than
/// any machine has — are represented.
fn decode_rgba_seeds(base: &Path, jpeg: &[u8], png_image: &[u8]) -> std::io::Result<()> {
    let rgba_dir = base.join("decode_rgba");
    fs::create_dir_all(&rgba_dir)?;
    let prefixed = |selector: u8, image: &[u8]| {
        let mut out = Vec::with_capacity(image.len() + 1);
        out.push(selector);
        out.extend_from_slice(image);
        out
    };
    fs::write(rgba_dir.join("jpeg"), prefixed(0, jpeg))?;
    fs::write(rgba_dir.join("png"), prefixed(1, png_image))?;
    fs::write(
        rgba_dir.join("jpeg_truncated"),
        prefixed(0, &truncated(jpeg, jpeg.len() / 3)),
    )?;
    fs::write(
        rgba_dir.join("png_flipped"),
        prefixed(1, &with_flipped_byte(png_image, 20)),
    )?;
    // A PNG header declaring an impossible frame: the shape that would size a
    // multi-gigabyte allocation if the pixel ceiling were checked after the
    // decode rather than before it.
    let mut huge = png_image.to_vec();
    huge[16..20].copy_from_slice(&0xFFFF_u32.to_be_bytes());
    huge[20..24].copy_from_slice(&0xFFFF_u32.to_be_bytes());
    fs::write(rgba_dir.join("png_impossible_frame"), prefixed(1, &huge))?;
    Ok(())
}
