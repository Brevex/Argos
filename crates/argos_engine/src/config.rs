//! What a scan covers and how much machine it is allowed to use.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{Bound, RangeBounds};
use std::thread;

use argos_core::geometry::{ByteOffset, ByteRange};

/// Bytes carried between consecutive chunks so a structure straddling the
/// boundary is still found whole: the larger of what the two detectors need.
pub const CHUNK_OVERLAP_BYTES: usize =
    if argos_fs::residue::WINDOW_OVERLAP_BYTES > argos_carve::SIGNATURE_OVERLAP_BYTES {
        argos_fs::residue::WINDOW_OVERLAP_BYTES
    } else {
        argos_carve::SIGNATURE_OVERLAP_BYTES
    };

/// Bytes read per chunk by default.
///
/// 8 MiB keeps the reader in large sequential requests — the throughput driver
/// on rotational media — while bounding both the memory a queued chunk costs
/// and the cancellation latency, which is one chunk.
pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Smallest chunk a scan may use. A chunk must exceed the overlap by a wide
/// margin, or the sweep re-reads more than it advances.
pub const MIN_CHUNK_BYTES: usize = 64 * 1024;

/// Largest chunk a scan may use. Above this, one queued chunk per worker
/// costs more memory than it saves in syscalls.
pub const MAX_CHUNK_BYTES: usize = 256 * 1024 * 1024;

/// Workers used when the machine's parallelism cannot be determined.
const FALLBACK_WORKERS: NonZeroUsize = NonZeroUsize::new(4).expect("4 is not zero");

/// Queued chunks per worker, so a worker always has its next chunk ready
/// without the reader running arbitrarily far ahead.
const QUEUE_DEPTH_PER_WORKER: usize = 2;

/// Ceiling on the bytes held in queued chunk buffers at once.
///
/// Queue depth follows the worker count, but only up to here. The reader
/// writes each buffer and a worker reads it moments later, so the set of
/// buffers in rotation wants to stay inside last-level cache; past that every
/// worker's first touch of a chunk is a DRAM round trip. Measured on a 2 GiB
/// image with a 32 MiB L3: 16 MiB in flight sustains ~11.9 GB/s at any worker
/// count, while 64 MiB drops to ~5.8 GB/s and holds 3x the resident memory
/// (`M-HOTPATH`, `M-THROUGHPUT`).
const MAX_INFLIGHT_BYTES: usize = 16 * 1024 * 1024;

/// Chunks that must be in flight regardless of the ceiling, so the reader and
/// a worker can always run at the same time.
const MIN_QUEUE_DEPTH: usize = 2;

/// Decode attempts reassembly may spend across a whole scan.
///
/// The per-candidate budget bounds one search; this bounds the stage, which
/// runs by default. A medium carrying thousands of false signature hits would
/// otherwise turn a scan that reads at gigabytes a second into one that
/// decodes for hours.
///
/// Measured: a hypothesis the entropy decoder rejects costs about 5 us, since
/// unrelated bytes fail on the first Huffman code outside the table, so this
/// budget caps the stage at roughly a second and a half of decoding whatever
/// the medium holds. When it runs out the report says so, so a user knows
/// there was more to try rather than being told the medium held nothing else
/// (`M-DOCUMENTED-MAGIC`).
pub const REASSEMBLY_BUDGET: u32 = 250_000;

/// Smallest long side, in pixels, an artifact is written to disk for.
///
/// A disk holds far more derived images than photographs, and they are small:
/// icons, sprites, avatars and — dominating everything else — the thumbnail
/// caches that desktops and phones keep. Measured over a 1 TB disk of ten
/// years' use, out of 154,023 images written by a run with no floor, a sample
/// of 1,499 found 57% at exactly 258x258, 87% at 300 pixels or less, and 1.2%
/// above 1024. The dimensions cluster because a cache writes one size.
///
/// The floor sits above the caches measured there and below 640x480, the
/// smallest resolution a camera of that era produced, so no photograph on that
/// medium falls under it. Nothing is discarded: what is not written is still
/// recorded with its extents and digest (`M-DOCUMENTED-MAGIC`).
pub const DEFAULT_MIN_LONG_SIDE: u32 = 300;

/// Which recovery stages a scan runs.
///
/// Both are on by default: filesystem metadata is the strongest evidence and
/// carving is what finds files whose metadata is gone, so a scan that omits
/// either reports less than the medium holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stages {
    /// Partition tables, residue sweep and filesystem metadata recovery.
    pub filesystem: bool,
    /// Full-surface signature carving with structural validation.
    pub carving: bool,
    /// Reassembly of images the medium stored in pieces, from the candidates
    /// carving could not complete. Needs `carving`, which is where the
    /// fragmentation points come from.
    pub reassembly: bool,
}

impl Default for Stages {
    fn default() -> Self {
        Self {
            filesystem: true,
            carving: true,
            reassembly: true,
        }
    }
}

/// A validated scan configuration.
///
/// Built through [`ScanConfig::builder`]; every setter is infallible and all
/// validation happens in [`ScanConfigBuilder::build`] (`M-BUILD-RESULT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanConfig {
    workers: NonZeroUsize,
    chunk_bytes: usize,
    start: u64,
    end: Option<u64>,
    stages: Stages,
    previews: bool,
    min_long_side: u32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self::builder()
            .build()
            .unwrap_or_else(|err| unreachable!("the default configuration must validate: {err}"))
    }
}

impl ScanConfig {
    /// A builder over the defaults: the whole medium, every stage, one worker
    /// per available core.
    #[must_use]
    pub fn builder() -> ScanConfigBuilder {
        ScanConfigBuilder {
            workers: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            start: 0,
            end: None,
            stages: Stages::default(),
            previews: false,
            min_long_side: DEFAULT_MIN_LONG_SIDE,
        }
    }

    /// Number of worker threads the sweep and validation stages use.
    #[must_use]
    pub fn workers(&self) -> NonZeroUsize {
        self.workers
    }

    /// Smallest long side, in pixels, an artifact is written to disk for.
    ///
    /// Zero writes everything. Whatever is not written is still recorded with
    /// its extents, digest and dimensions, so the manifest stays a complete
    /// account of the medium and the extents locate the bytes exactly — a
    /// rerun with a lower floor produces them. `argos export` cannot: it reads
    /// the session directory, where an unwritten artifact has no file.
    #[must_use]
    pub fn min_long_side(&self) -> u32 {
        self.min_long_side
    }

    /// Bytes read per chunk.
    #[must_use]
    pub fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }

    /// Chunks that may be in flight between the reader and the workers.
    ///
    /// One per worker plus one in reserve, capped so the buffers in flight
    /// never exceed [`MAX_INFLIGHT_BYTES`].
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        let affordable = MAX_INFLIGHT_BYTES / self.chunk_bytes.max(1);
        self.workers
            .get()
            .saturating_mul(QUEUE_DEPTH_PER_WORKER)
            .min(affordable)
            .max(MIN_QUEUE_DEPTH)
    }

    /// Stages this scan runs.
    #[must_use]
    pub fn stages(&self) -> Stages {
        self.stages
    }

    /// Whether the scan renders a preview of every artifact that decodes.
    ///
    /// Off by default: previews are derived files, and a forensic tool should
    /// not put bytes in an output directory that nobody asked for.
    #[must_use]
    pub fn previews(&self) -> bool {
        self.previews
    }

    /// The byte range of a medium of `medium_len` bytes this scan covers,
    /// clamped to the medium.
    #[must_use]
    pub fn range_within(&self, medium_len: u64) -> ByteRange {
        let start = self.start.min(medium_len);
        let end = self.end.unwrap_or(medium_len).min(medium_len).max(start);
        ByteRange::new(ByteOffset::new(start), end.saturating_sub(start))
    }
}

/// Chainable builder for [`ScanConfig`].
#[derive(Clone, Copy, Debug)]
pub struct ScanConfigBuilder {
    workers: Option<NonZeroUsize>,
    chunk_bytes: usize,
    start: u64,
    end: Option<u64>,
    stages: Stages,
    previews: bool,
    min_long_side: u32,
}

impl ScanConfigBuilder {
    /// Sets the number of worker threads. Defaults to the machine's available
    /// parallelism — never a hardcoded count.
    #[must_use]
    pub fn workers(mut self, workers: NonZeroUsize) -> Self {
        self.workers = Some(workers);
        self
    }

    /// Sets the bytes read per chunk.
    #[must_use]
    pub fn chunk_bytes(mut self, chunk_bytes: usize) -> Self {
        self.chunk_bytes = chunk_bytes;
        self
    }

    /// Smallest long side, in pixels, an artifact is written for. Defaults to
    /// [`DEFAULT_MIN_LONG_SIDE`]; zero writes everything.
    ///
    /// Every artifact below it is still recorded with its extents, digest and
    /// dimensions, so the manifest stays a complete account of the medium and
    /// the extents locate the bytes exactly for a rerun with a lower floor.
    #[must_use]
    pub fn min_long_side(mut self, min_long_side: u32) -> Self {
        self.min_long_side = min_long_side;
        self
    }

    /// Whether to render a preview of every artifact that decodes.
    #[must_use]
    pub fn previews(mut self, previews: bool) -> Self {
        self.previews = previews;
        self
    }

    /// Restricts the scan to a byte range of the medium. Defaults to all of it.
    #[must_use]
    pub fn range(mut self, range: impl RangeBounds<ByteOffset>) -> Self {
        self.start = match range.start_bound() {
            Bound::Included(at) => at.get(),
            Bound::Excluded(at) => at.get().saturating_add(1),
            Bound::Unbounded => 0,
        };
        self.end = match range.end_bound() {
            Bound::Included(at) => Some(at.get().saturating_add(1)),
            Bound::Excluded(at) => Some(at.get()),
            Bound::Unbounded => None,
        };
        self
    }

    /// Sets which recovery stages run.
    #[must_use]
    pub fn stages(mut self, stages: Stages) -> Self {
        self.stages = stages;
        self
    }

    /// Validates the accumulated settings.
    ///
    /// # Errors
    ///
    /// Fails when the chunk size is outside `MIN_CHUNK_BYTES..=MAX_CHUNK_BYTES`,
    /// when the range is empty, or when no stage is enabled.
    pub fn build(self) -> Result<ScanConfig, ConfigError> {
        if !(MIN_CHUNK_BYTES..=MAX_CHUNK_BYTES).contains(&self.chunk_bytes) {
            return Err(ConfigError::new(ConfigProblem::ChunkSize {
                chunk_bytes: self.chunk_bytes,
            }));
        }
        if let Some(end) = self.end
            && end <= self.start
        {
            return Err(ConfigError::new(ConfigProblem::EmptyRange {
                start: self.start,
                end,
            }));
        }
        if !self.stages.filesystem && !self.stages.carving {
            return Err(ConfigError::new(ConfigProblem::NoStages));
        }
        if self.stages.reassembly && !self.stages.carving {
            return Err(ConfigError::new(ConfigProblem::ReassemblyNeedsCarving));
        }
        Ok(ScanConfig {
            workers: self
                .workers
                .unwrap_or_else(|| thread::available_parallelism().unwrap_or(FALLBACK_WORKERS)),
            chunk_bytes: self.chunk_bytes,
            start: self.start,
            end: self.end,
            stages: self.stages,
            previews: self.previews,
            min_long_side: self.min_long_side,
        })
    }
}

/// A scan could not be configured as asked.
#[derive(Debug)]
pub struct ConfigError {
    problem: ConfigProblem,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum ConfigProblem {
    ChunkSize { chunk_bytes: usize },
    EmptyRange { start: u64, end: u64 },
    NoStages,
    NoViews,
    ReassemblyNeedsCarving,
}

impl ConfigError {
    fn new(problem: ConfigProblem) -> Self {
        Self {
            problem,
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn no_views() -> Self {
        Self::new(ConfigProblem::NoViews)
    }

    /// Backtrace captured where the configuration was rejected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.problem {
            ConfigProblem::ChunkSize { chunk_bytes } => write!(
                f,
                "chunk size {chunk_bytes} bytes is outside {MIN_CHUNK_BYTES}..={MAX_CHUNK_BYTES}"
            ),
            ConfigProblem::EmptyRange { start, end } => {
                write!(f, "scan range {start}..{end} covers no bytes")
            }
            ConfigProblem::NoStages => {
                f.write_str("a scan with neither filesystem recovery nor carving finds nothing")
            }
            ConfigProblem::NoViews => {
                f.write_str("a scan needs at least one read-only view of the medium")
            }
            ConfigProblem::ReassemblyNeedsCarving => f.write_str(
                "reassembly works on the candidates carving could not complete, so it cannot \
                 run with carving disabled",
            ),
        }?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for ConfigError {}
