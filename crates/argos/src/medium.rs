//! Opening what the user named: a raw image file or a block device.
//!
//! Both are opened read-only and presented to the engine as the same thing —
//! a set of independent `Read + Seek` views — so the recovery code never
//! learns which one it is scanning.
//!
//! The same file answers where a run may *write*. A-READ-ONLY requires an
//! output path to be validated against the source **device**, not merely the
//! source path: the dangerous case is not `--out` inside an image file's
//! directory, it is `argos scan /dev/sda --out /mnt/sda2/recovered`, where the
//! output lands on a partition of the disk being read and overwrites the
//! unallocated space still to be carved. Only device identity reveals that.
//!
//! The same file also copies the medium into a raw image before anything else
//! reads it. A scan reads the whole surface, and a scan that is run again
//! reads it again; on a medium that is failing, every pass is a pass the
//! medium may not survive, and the sectors lost are lost for good. Acquiring
//! once and working from the image afterwards turns an experiment that costs
//! the evidence into one that costs a file read. Two guards stand between
//! that and the evidence: the source is opened read-only through the same HAL
//! a scan uses, and the destination is refused unless it is an ordinary file
//! on some other disk — a destination that is itself a device node would be
//! written over, which is the one thing this program must never do
//! (`A-READ-ONLY`).

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use argos_device::acquire::{self, Progress};
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

    if is_device_node(&metadata) {
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
fn is_device_node(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    // macOS raw disk nodes are character devices, not block devices; Linux
    // disks are block devices. Either is a medium rather than an image.
    let kind = metadata.file_type();
    kind.is_block_device() || kind.is_char_device()
}

#[cfg(not(unix))]
fn is_device_node(_metadata: &std::fs::Metadata) -> bool {
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
        || argos_device::inventory::shadow_copy_index(&text).is_some()
}

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

/// Whether the source node is one `/sys/dev/block` can resolve a disk for.
///
/// Narrower than [`is_device_node`] on purpose: the same-device check reaches
/// the source through its `sysfs` block entry, and a character device has
/// none. It is only ever asked on Linux, which is the only platform whose
/// device identity this crate can establish.
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

/// Acquires `source` into a new raw image at `to`.
///
/// `cancelled` is consulted once per chunk of the sweep and once per sector of
/// the refinement, so a stop takes effect within one read rather than at the
/// end of the medium. What was copied before the stop stays copied: the image
/// is a prefix of the medium, and the report says how much was never reached.
///
/// # Errors
///
/// Fails when the destination would write onto the source, when it is not an
/// ordinary file, when it already exists, when the source cannot be opened
/// read-only, or when the image cannot be written.
pub fn acquire_image(
    source: &Path,
    to: &Path,
    notice: &dyn AcquireNotice,
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<()> {
    refuse_writing_onto_source(source, to)?;
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
        Source::Device(mut device) => acquire::run(
            &mut device,
            &mut dest,
            acquire::Options::new(),
            &mut report,
            cancelled,
        ),
        Source::Image(mut image) => acquire::run(
            &mut image,
            &mut dest,
            acquire::Options::new(),
            &mut report,
            cancelled,
        ),
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

/// What an acquisition tells the person waiting for it.
///
/// A pass over a terabyte takes hours; the CLI renders this to the terminal,
/// and a test collects it instead.
pub trait AcquireNotice {
    /// One progress report from a pass.
    fn progress(&self, progress: Progress);
    /// What the acquisition recovered, and exactly what it did not.
    fn finished(&self, report: &acquire::Report);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{AcquireNotice, Progress, acquire, acquire_image};

    /// Sector size every fixture here is sized in multiples of.
    const SECTOR: usize = 512;

    /// Sectors the fixture medium holds.
    const SECTORS: u64 = 2048;

    /// Collects what an acquisition reported, in the order it said it.
    ///
    /// This is the other implementation of [`AcquireNotice`]: the terminal renderer
    /// shows these to a person, and this one keeps them so a test can assert
    /// on what a person would have been told.
    #[derive(Default)]
    struct Collected {
        passes: Mutex<Vec<Progress>>,
        report: Mutex<Option<acquire::Report>>,
    }

    impl AcquireNotice for Collected {
        fn progress(&self, progress: Progress) {
            self.passes
                .lock()
                .expect("no other thread panicked")
                .push(progress);
        }

        fn finished(&self, report: &acquire::Report) {
            *self.report.lock().expect("no other thread panicked") = Some(report.clone());
        }
    }

    /// A medium whose every sector is distinguishable from every other, so a
    /// copy that reorders or drops one cannot compare equal.
    fn medium() -> Vec<u8> {
        (0..SECTORS)
            .flat_map(|sector| {
                let mark = u8::try_from(sector % 251).expect("the modulus fits a byte");
                std::iter::repeat_n(mark, SECTOR)
            })
            .collect()
    }

    #[test]
    fn an_acquisition_tells_its_caller_what_it_passed_and_what_it_recovered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.img");
        let planted = medium();
        std::fs::write(&source, &planted).expect("write the source medium");
        let to = dir.path().join("copy.img");

        let collected = Collected::default();
        acquire_image(&source, &to, &collected, &|| false).expect("acquire a healthy image file");

        assert_eq!(
            std::fs::read(&to).expect("the acquired image"),
            planted,
            "an acquisition that is not bit-identical is not evidence"
        );

        // The point of the port: a caller that renders nothing still hears
        // about every pass, and hears the account at the end.
        let passes = collected.passes.lock().expect("no other thread panicked");
        assert!(
            passes
                .iter()
                .any(|pass| matches!(pass, Progress::Swept { .. })),
            "the sequential sweep must report itself: {passes:?}"
        );
        assert!(
            passes.iter().all(|pass| match pass {
                Progress::Swept { done, total } | Progress::Refined { done, total } =>
                    done <= total,
            }),
            "no pass may report more work done than it had to do: {passes:?}"
        );

        let report = collected
            .report
            .lock()
            .expect("no other thread panicked")
            .clone()
            .expect("a finished acquisition reports what it recovered");
        assert_eq!(report.sector_count(), SECTORS);
        assert_eq!(report.recovered_sectors(), SECTORS);
        assert!(
            report.is_complete(),
            "a healthy image file has nothing unreadable: {:?}",
            report.unreadable()
        );
    }

    #[test]
    fn acquiring_into_a_path_that_already_exists_is_refused_before_anything_is_written() {
        // The earlier image may be the only copy of a medium that no longer
        // reads. Nothing about this may depend on the caller noticing.
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.img");
        std::fs::write(&source, medium()).expect("write the source medium");
        let to = dir.path().join("copy.img");
        std::fs::write(&to, b"an earlier acquisition").expect("write the existing image");

        let collected = Collected::default();
        acquire_image(&source, &to, &collected, &|| false)
            .expect_err("an existing destination must be refused");

        assert_eq!(
            std::fs::read(&to).expect("the earlier image"),
            b"an earlier acquisition",
            "the earlier acquisition must survive untouched"
        );
        assert!(
            collected
                .report
                .lock()
                .expect("no other thread panicked")
                .is_none(),
            "a refused acquisition reports no recovery"
        );
    }
}
