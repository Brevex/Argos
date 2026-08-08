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
    /// What the operating system says about this medium that the user should
    /// know before trusting the result.
    pub warnings: Vec<String>,
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
    // A device-namespace path is a device by name; nothing can stat it.
    if looks_like_device(path) {
        return open_device(path, views);
    }
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;

    if is_block_device(&metadata) {
        return open_device(path, views);
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
        expects_content: argos_device::class::expects_deleted_content(
            image.geometry().class,
            argos_device::TrimState::Unknown,
        ),
        warnings: Vec::new(),
        path: path.to_path_buf(),
        kind: "image file",
        trailing,
    })
}

/// Opens a raw medium through the device HAL.
fn open_device(path: &Path, views: usize) -> anyhow::Result<Opened> {
    {
        let mut opened = Vec::with_capacity(views);
        let mut geometry = None;
        let mut trim = argos_device::TrimState::Unknown;
        for _ in 0..views.max(1) {
            let device = Device::open(path)
                .with_context(|| format!("cannot open device {} read-only", path.display()))?;
            trim = device.trim();
            let reader = BlockReader::new(device);
            geometry = Some(reader.geometry());
            opened.push(View::Device(Box::new(reader)));
        }
        let geometry = geometry.context("a scan needs at least one view")?;
        let len = geometry
            .capacity_bytes()
            .context("the device reports a geometry that overflows its address space")?;
        Ok(Opened {
            views: opened,
            len,
            expects_content: argos_device::class::expects_deleted_content(geometry.class, trim),
            warnings: warnings_for(path),
            path: path.to_path_buf(),
            kind: "block device",
            trailing: 0,
        })
    }
}

/// What the operating system says about this medium that a user should know
/// before trusting the result.
///
/// Two conditions, and both are about the medium changing while it is read.
/// Argos never writes to a source (A-READ-ONLY), but a mounted filesystem is
/// being written to by the OS underneath it — journal commits, atime updates,
/// background trim — so a manifest can end up describing a medium that no
/// longer exists. Scanning one partition of a disk is the other: it cannot see
/// the partition table, the gaps, or the residue a re-format left, so the
/// recovery is bounded in a way the user did not ask for.
fn warnings_for(path: &Path) -> Vec<String> {
    let text = path.to_string_lossy().into_owned();
    let mut warnings = Vec::new();

    let kind = argos_device::naming::linux_node_kind(&text)
        .or_else(|| argos_device::naming::macos_node_kind(&text))
        .or_else(|| argos_device::naming::windows_node_kind(&text));
    if kind == Some(argos_device::naming::NodeKind::Partition) {
        warnings.push(
            "this path is one partition, not the whole disk: the partition table, the space \
             between partitions and any residue of earlier filesystems lie outside it and will \
             not be scanned"
                .to_owned(),
        );
    }

    // Compared in canonical form: a scan of `/dev/rdisk0` is a scan of the
    // medium enumeration lists as `/dev/disk0`, and matching the strings as
    // written would warn about nothing on exactly the raw node the macOS HAL
    // tells callers to prefer.
    let wanted = argos_device::naming::canonical(&text);
    if let Some(device) = argos_device::inventory::list()
        .into_iter()
        .find(|device| argos_device::naming::canonical(&device.path.to_string_lossy()) == wanted)
        && device.is_mounted()
    {
        let where_at: Vec<String> = device
            .mounts
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        if device.has_writable_mount() {
            warnings.push(format!(
                "this medium is mounted writable ({}); the operating system may change it while \
                 the scan reads, so recovered bytes and the manifest can describe different \
                 moments. Unmount it if the evidence matters",
                where_at.join(", ")
            ));
        } else {
            warnings.push(format!(
                "this medium is mounted read-only ({})",
                where_at.join(", ")
            ));
        }
    }
    warnings
}

#[cfg(unix)]
fn is_block_device(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    // macOS raw disk nodes are character devices, not block devices; Linux
    // disks are block devices. Either is a medium rather than an image.
    let kind = metadata.file_type();
    kind.is_block_device() || kind.is_char_device()
}

#[cfg(not(unix))]
fn is_block_device(_metadata: &std::fs::Metadata) -> bool {
    // Windows device-namespace paths never reach here: `std::fs::metadata`
    // cannot stat them, so `open` routes them by name before it looks at the
    // filesystem. See `looks_like_device`.
    false
}

/// Whether `path` names a raw device by convention rather than by filesystem
/// type.
///
/// Windows device-namespace paths — `\\.\PhysicalDrive0` and the shadow-copy
/// namespace — are not files and cannot be stat'd, so they have to be
/// recognised by name before anything tries. On Unix this adds nothing the
/// file type does not already say, but keeping one predicate for all three
/// platforms keeps the routing in one place.
fn looks_like_device(path: &Path) -> bool {
    let text = path.to_string_lossy();
    argos_device::naming::windows_node_kind(&text).is_some()
        || argos_device::shadow::shadow_copy_index(&text).is_some()
}
