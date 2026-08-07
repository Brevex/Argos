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

    let scan_dir = base.join("carver_scan");
    fs::create_dir_all(&scan_dir)?;
    let disk = argos_carve::fixture::Disk::filled(64 * 1024)
        .with(1_000, &plain)
        .with(30_000, &png(8, 8))
        .into_bytes();
    fs::write(scan_dir.join("disk"), &disk)?;

    println!("seed corpus written under {}", base.display());
    Ok(())
}
