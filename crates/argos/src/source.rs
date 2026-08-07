//! Opening what the user named: a raw image file or a block device.
//!
//! Both are opened read-only and presented to the engine as the same thing —
//! a set of independent `Read + Seek` views — so the recovery code never
//! learns which one it is scanning.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context;
use argos_device::{BlockReader, Device, ImageSource};

/// One read-only view of the source, whichever kind it turned out to be.
#[derive(Debug)]
pub enum View {
    /// A raw image file, read directly.
    Image(File),
    /// A block device, byte-addressed through its sector geometry.
    Device(Box<BlockReader<Device>>),
}

impl Read for View {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Image(file) => file.read(buf),
            Self::Device(device) => device.read(buf),
        }
    }
}

impl Seek for View {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Image(file) => file.seek(pos),
            Self::Device(device) => device.seek(pos),
        }
    }
}

/// An opened source, ready to hand to the engine.
pub struct Opened {
    /// Independent views, one per worker.
    pub views: Vec<View>,
    /// Addressable length in bytes.
    pub len: u64,
    /// Whether a medium of this class is still expected to hold deleted
    /// content, or whether `TRIM` has probably already erased it.
    pub expects_content: bool,
    path: PathBuf,
    kind: &'static str,
    /// Bytes of a trailing partial sector an image file cannot address.
    trailing: u64,
}

impl Opened {
    /// One line naming the source, for the manifest and the console.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} ({}, {} bytes)",
            self.path.display(),
            self.kind,
            self.len
        );
        if self.trailing > 0 {
            use std::fmt::Write as _;
            let _ = write!(text, ", {} trailing bytes not addressable", self.trailing);
        }
        text
    }
}

/// Opens `path` read-only as `views` independent views.
///
/// A block device goes through the device HAL; anything else must be a regular
/// file holding a raw image.
///
/// # Errors
///
/// Fails when the path cannot be opened read-only, when its geometry cannot be
/// queried, or when it is neither a regular file nor a block device.
pub fn open(path: &Path, views: usize) -> anyhow::Result<Opened> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;

    if is_block_device(&metadata) {
        let mut opened = Vec::with_capacity(views);
        let mut geometry = None;
        for _ in 0..views.max(1) {
            let device = Device::open(path)
                .with_context(|| format!("cannot open device {} read-only", path.display()))?;
            let reader = BlockReader::new(device);
            geometry = Some(reader.geometry());
            opened.push(View::Device(Box::new(reader)));
        }
        let geometry = geometry.context("a scan needs at least one view")?;
        let len = geometry
            .capacity_bytes()
            .context("the device reports a geometry that overflows its address space")?;
        return Ok(Opened {
            views: opened,
            len,
            expects_content: argos_engine::expects_recoverable_content(geometry.class),
            path: path.to_path_buf(),
            kind: "block device",
            trailing: 0,
        });
    }

    anyhow::ensure!(
        metadata.is_file(),
        "{} is neither a regular file nor a block device",
        path.display()
    );

    // The image adapter reports the geometry — including any trailing partial
    // sector — while the scan itself reads the file directly.
    let image = ImageSource::open(path)
        .with_context(|| format!("cannot open {} read-only", path.display()))?;
    let trailing = image.trailing_bytes();
    let len = metadata.len().saturating_sub(trailing);

    let mut opened = Vec::with_capacity(views);
    for _ in 0..views.max(1) {
        opened.push(View::Image(
            File::options()
                .read(true)
                .open(path)
                .with_context(|| format!("cannot open {} read-only", path.display()))?,
        ));
    }
    Ok(Opened {
        views: opened,
        len,
        expects_content: argos_engine::expects_recoverable_content(image.geometry().class),
        path: path.to_path_buf(),
        kind: "image file",
        trailing,
    })
}

#[cfg(unix)]
fn is_block_device(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    metadata.file_type().is_block_device()
}

#[cfg(not(unix))]
fn is_block_device(_metadata: &std::fs::Metadata) -> bool {
    // Raw-device paths on other platforms arrive through their own HAL, which
    // does not exist yet; until then everything here is an image file.
    false
}
