//! Copying a medium into a raw image before anything else reads it.
//!
//! A scan reads the whole surface, and a scan that is run again reads it again.
//! On a medium that is failing, every pass is a pass the medium may not
//! survive, and the sectors lost are lost for good. Acquiring once and working
//! from the image afterwards turns an experiment that costs the evidence into
//! one that costs a file read.
//!
//! Two guards stand between this and the evidence. The source is opened
//! read-only through the same HAL a scan uses, and the destination is refused
//! unless it is an ordinary file on some other disk — a destination that is
//! itself a device node would be written over, which is the one thing this
//! program must never do (`A-READ-ONLY`).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, bail};
use argos_device::acquire::{self, Progress};
use argos_device::{Device, ImageSource};

/// Acquires `source` into a new raw image at `to`.
///
/// # Errors
///
/// Fails when the destination would write onto the source, when it is not an
/// ordinary file, when it already exists, when the source cannot be opened
/// read-only, or when the image cannot be written.
pub fn run(source: &Path, to: &Path, notice: &dyn Notice) -> anyhow::Result<()> {
    crate::destination::refuse_writing_onto_source(source, to)?;
    refuse_device_destination(to)?;

    // Created exclusively: an acquisition that silently overwrote an earlier
    // image would destroy the only copy of a medium that may no longer be
    // readable.
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(to)
        .with_context(|| {
            format!(
                "cannot create the image {} (it must not already exist)",
                to.display()
            )
        })?;
    let mut dest = BufWriter::new(file);

    let mut report = |progress: Progress| notice.progress(progress);
    let report = match open_source(source)? {
        Source::Device(mut device) => {
            acquire::run(&mut device, &mut dest, acquire::Options::new(), &mut report)
        }
        Source::Image(mut image) => {
            acquire::run(&mut image, &mut dest, acquire::Options::new(), &mut report)
        }
    }
    .with_context(|| format!("cannot write the image {}", to.display()))?;

    notice.finished(&report);
    Ok(())
}

/// A source opened for acquisition, whichever kind it turned out to be.
enum Source {
    Device(Device),
    Image(ImageSource),
}

/// Opens `source` read-only as something with a sector geometry.
///
/// Acquiring an image file is not a mistake: a copy made from a failing medium
/// by another tool can itself have holes, and a second pass over it costs
/// nothing and maps them the same way.
fn open_source(source: &Path) -> anyhow::Result<Source> {
    let metadata = std::fs::metadata(source);
    let is_device = metadata
        .as_ref()
        .map(is_device_node)
        .ok()
        // A device-namespace path cannot be stat'd; it is a device by name.
        .unwrap_or(true);
    if is_device {
        return Device::open(source)
            .map(Source::Device)
            .with_context(|| format!("cannot open device {} read-only", source.display()));
    }
    ImageSource::open(source)
        .map(Source::Image)
        .with_context(|| format!("cannot open {} read-only", source.display()))
}

/// Refuses a destination that is a device node.
///
/// `refuse_writing_onto_source` compares device identity, so it already stops
/// an image being written onto the disk being read. It does not stop
/// `--to /dev/sdb`, which would destroy a different disk just as completely.
/// An acquisition writes a file, so anything that is not a file is refused.
fn refuse_device_destination(to: &Path) -> anyhow::Result<()> {
    let Ok(metadata) = std::fs::metadata(to) else {
        // It does not exist yet, which is what `create_new` requires anyway.
        // A device node always exists, so this branch cannot be one.
        return Ok(());
    };
    if !metadata.is_file() {
        bail!(
            "refusing to acquire into {}: an image is written to an ordinary file, and writing \
             to a device would overwrite it",
            to.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn is_device_node(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    let kind = metadata.file_type();
    kind.is_block_device() || kind.is_char_device()
}

#[cfg(not(unix))]
fn is_device_node(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// What an acquisition tells the person waiting for it.
///
/// A pass over a terabyte takes hours; the CLI renders this to the terminal,
/// and a test collects it instead.
pub trait Notice {
    /// One progress report from a pass.
    fn progress(&self, progress: Progress);
    /// What the acquisition recovered, and exactly what it did not.
    fn finished(&self, report: &acquire::Report);
}
