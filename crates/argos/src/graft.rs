//! The `graft` command: pixels from fragments whose header is gone.
//!
//! The sweep itself is [`argos_engine::graft`]; this opens the medium, writes
//! what decoded, and keeps the output apart from any scan's.

use std::path::Path;

use anyhow::Context as _;
use argos_engine::graft::JpegReference as Reference;

use crate::source;

/// Reads `path` as the header its lost siblings were written with.
pub fn reference_from(path: &Path) -> anyhow::Result<Reference> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read the reference {}", path.display()))?;
    Reference::read(&bytes)
        .map_err(|error| anyhow::anyhow!("{} {error}", path.display()))
        .context(
            "a reference must be a whole baseline or extended sequential JPEG — one of the \
             photographs this medium already gave back, from the same camera as what is missing",
        )
}

/// Sweeps `range` of `src`, writing every graft that decoded into `out`.
pub fn run(
    src: &Path,
    out: &Path,
    reference: &Reference,
    range: std::ops::Range<u64>,
) -> anyhow::Result<(usize, usize)> {
    crate::destination::refuse_writing_onto_source(src, out)?;
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;

    let mut opened = source::open(src, 1)?;
    for warning in &opened.warnings {
        eprintln!("warning: {warning}");
    }
    let end = range.end.min(opened.len);
    let mut view = opened
        .views
        .pop()
        .context("opening the medium yielded no view")?;

    let mut written = 0_usize;
    let mut failed = Ok(());
    let entered = argos_engine::graft::sweep(&mut view, range.start..end, reference, |grafted| {
        if failed.is_err() {
            return;
        }
        let name = format!("grafted-{:016x}.jpg", grafted.at.get());
        match std::fs::write(out.join(&name), &grafted.bytes) {
            Ok(()) => {
                written += 1;
                println!(
                    "  {name}  {}x{} entered at medium offset {}",
                    grafted.dimensions.0,
                    grafted.dimensions.1,
                    grafted.at.get()
                );
            }
            Err(error) => failed = Err(anyhow::anyhow!("cannot write {name}: {error}")),
        }
    });
    failed?;
    Ok((entered, written))
}
