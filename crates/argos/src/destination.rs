//! Refusing an output location that would write onto the evidence.
//!
//! A-READ-ONLY requires destination paths to be validated against the source
//! *device*, not merely the source path. The dangerous case is not
//! `--out` inside the image file's directory; it is `argos scan /dev/sda --out
//! /mnt/sda2/recovered`, where the output lands on a partition of the very
//! disk being read and overwrites the unallocated space still to be carved.
//! Nothing about the two paths reveals that — only their device identity does.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Refuses an output location that could write onto the source.
///
/// Two independent checks: the output tree must not contain the source path,
/// and — when the source is a block device — the output must not live on that
/// device or on any partition of it.
///
/// # Errors
///
/// Fails when either check trips, or when a path cannot be resolved.
pub fn refuse_writing_onto_source(source: &Path, out: &Path) -> anyhow::Result<()> {
    let resolved_source = std::fs::canonicalize(source)
        .with_context(|| format!("cannot resolve source path {}", source.display()))?;
    // `out` may not exist yet; resolve its deepest existing ancestor.
    let resolved_out = deepest_existing_ancestor(out)?;

    if resolved_source.starts_with(&resolved_out) {
        bail!(
            "refusing to write output under {}: it would contain the source {}",
            out.display(),
            source.display()
        );
    }
    refuse_same_device(&resolved_source, &resolved_out, source, out)
}

#[cfg(target_os = "linux")]
fn refuse_same_device(
    resolved_source: &Path,
    resolved_out: &Path,
    source: &Path,
    out: &Path,
) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let source_meta = std::fs::metadata(resolved_source)
        .with_context(|| format!("cannot inspect {}", source.display()))?;
    if !is_block_device(&source_meta) {
        // An image file is just a file; the path check above already covers it.
        return Ok(());
    }
    // `rdev` is the device the node *is*; `dev` is the device a file lives on.
    let source_disk = whole_disk_of(source_meta.rdev());
    let out_meta = std::fs::metadata(resolved_out)
        .with_context(|| format!("cannot inspect {}", out.display()))?;
    let out_disk = whole_disk_of(out_meta.dev());

    if let (Some(source_disk), Some(out_disk)) = (source_disk, out_disk)
        && source_disk == out_disk
    {
        bail!(
            "refusing to write output to {}: it is on {}, the same physical disk as the source \
             {} — writing there would destroy the deleted data being recovered",
            out.display(),
            source_disk,
            source.display()
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "one signature on every platform keeps the caller free of cfg"
)]
fn refuse_same_device(
    _resolved_source: &Path,
    _resolved_out: &Path,
    _source: &Path,
    _out: &Path,
) -> anyhow::Result<()> {
    // Device scanning only exists on Linux so far; when a HAL lands for another
    // platform, its device-identity check lands with it.
    Ok(())
}

/// Kernel name of the whole disk a device number belongs to.
///
/// A partition's `sysfs` entry has a `partition` file and sits inside its
/// disk's directory, so `sda2` resolves to `sda` — which is what makes a
/// partition of the source disk recognisable as the source disk.
#[cfg(target_os = "linux")]
fn whole_disk_of(dev: u64) -> Option<String> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "major and minor are 32-bit fields of a device number by definition"
    )]
    let (major, minor) = ((dev >> 8) as u32 & 0xFFF, (dev & 0xFF) as u32);
    let entry = PathBuf::from(format!("/sys/dev/block/{major}:{minor}"));
    let resolved = std::fs::canonicalize(&entry).ok()?;
    let disk = if entry.join("partition").exists() {
        resolved.parent()?.to_path_buf()
    } else {
        resolved
    };
    Some(disk.file_name()?.to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
fn is_block_device(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    metadata.file_type().is_block_device()
}

/// Canonicalizes the deepest ancestor of `path` that already exists, with the
/// missing suffix appended.
fn deepest_existing_ancestor(path: &Path) -> anyhow::Result<PathBuf> {
    let mut existing = path;
    let mut suffix = Vec::new();
    loop {
        if existing.exists() {
            let mut resolved = std::fs::canonicalize(existing)
                .with_context(|| format!("cannot resolve output path {}", existing.display()))?;
            for part in suffix.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                suffix.push(name.to_owned());
                existing = parent;
            }
            _ => bail!("output path {} has no existing ancestor", path.display()),
        }
    }
}
