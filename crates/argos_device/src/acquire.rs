//! Multi-pass acquisition of a [`BlockSource`] into a raw image.
//!
//! The strategy follows ddrescue: a fast sequential sweep first, skipping over
//! failing regions so a dying medium yields its healthy majority quickly, then a
//! refinement pass that revisits every suspect region sector by sector. Sectors
//! that stay unreadable are zero-filled in the image and — crucially — recorded in
//! the [`Report`]; the zeros are placeholders, never presented as read data.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::io::{self, Seek, SeekFrom, Write};

use argos_core::geometry::{Lba, SectorRange, SectorSize};
use argos_core::source::BlockSource;

/// Sectors per read during the sweep pass. 128 sectors is 64 KiB at 512-byte
/// sectors: large enough to stream a healthy HDD near platter speed, small enough
/// that one failing region costs little re-reading during refinement.
const SWEEP_CHUNK_SECTORS: u64 = 128;

/// Tuning for [`run`].
#[derive(Clone, Copy, Debug)]
pub struct Options {
    chunk_sectors: u64,
}

impl Options {
    /// Default acquisition tuning.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            chunk_sectors: SWEEP_CHUNK_SECTORS,
        }
    }

    /// Sets the sweep read size in sectors.
    ///
    /// # Panics
    ///
    /// Panics if `chunk_sectors` is zero — a zero-sector read is a caller bug.
    #[must_use]
    pub const fn with_chunk_sectors(mut self, chunk_sectors: u64) -> Self {
        assert!(chunk_sectors > 0, "sweep chunk must be at least 1 sector");
        self.chunk_sectors = chunk_sectors;
        self
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

/// How far an acquisition has got.
///
/// Two passes with different characters, so they count different things: the
/// sweep's denominator is the medium, the refinement's is only what the sweep
/// could not read. One figure spanning both would need an exchange rate
/// between a healthy sector and a dying one.
///
/// A run over a terabyte takes hours, and a stage that emits nothing is
/// indistinguishable from one that has stopped
/// (`docs/defects/01-reassembly-unbounded.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Progress {
    /// Sequential sweep: `done` of `total` sectors passed.
    Swept {
        /// Sectors the sweep has passed.
        done: u64,
        /// Sectors the medium holds.
        total: u64,
    },
    /// Refinement: `done` of `total` suspect sectors revisited one by one.
    Refined {
        /// Suspect sectors revisited.
        done: u64,
        /// Suspect sectors the sweep left behind.
        total: u64,
    },
}

/// Progress reports one pass will emit, at most.
///
/// A medium of two billion sectors must not produce two billion callbacks
/// (`M-LOG-OVERHEAD`); at this cap a bar still moves in steps too small for an
/// eye to catch.
const PROGRESS_STEPS: u64 = 200;

/// Outcome of an acquisition: how much was recovered and exactly what was not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    sector_count: u64,
    unreadable: Vec<SectorRange>,
    not_attempted: u64,
}

impl Report {
    /// Total sectors of the source medium.
    #[must_use]
    pub fn sector_count(&self) -> u64 {
        self.sector_count
    }

    /// Sectors recovered into the image.
    ///
    /// Excludes both what the medium refused and what a stopped run never
    /// reached. A sector that was not tried is not a sector that was recovered,
    /// and counting one as the other would overstate the copy
    /// (`A-CONFIDENCE-HONEST`).
    ///
    /// # Panics
    ///
    /// Panics if the two together claim more sectors than the medium has — a
    /// broken invariant must stop the program, not misreport a recovery.
    #[must_use]
    pub fn recovered_sectors(&self) -> u64 {
        let refused: u64 = self.unreadable.iter().map(|range| range.sectors).sum();
        let lost = refused.saturating_add(self.not_attempted);
        self.sector_count.checked_sub(lost).unwrap_or_else(|| {
            panic!(
                "the account claims {lost} sectors lost on a medium of {} ({refused} refused, {} \
                 not attempted) — a wrong number must never reach a report",
                self.sector_count, self.not_attempted
            )
        })
    }

    /// Sectors the run never tried, because it was stopped before reaching
    /// them.
    ///
    /// Deliberately not folded into [`Report::unreadable`]: that map is what
    /// the *medium* refused, and a run stopped by its operator says nothing
    /// about the medium. Merging the two would turn a cancelled copy into a
    /// report of a damaged disk.
    #[must_use]
    pub fn not_attempted(&self) -> u64 {
        self.not_attempted
    }

    /// Whether the run was stopped before it covered the medium.
    #[must_use]
    pub fn stopped_early(&self) -> bool {
        self.not_attempted > 0
    }

    /// Runs that stayed unreadable after refinement; their image bytes are zero
    /// placeholders. Sorted, non-adjacent, non-overlapping.
    #[must_use]
    pub fn unreadable(&self) -> &[SectorRange] {
        &self.unreadable
    }

    /// Whether every sector of the medium was recovered.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unreadable.is_empty() && self.not_attempted == 0
    }
}

/// Acquisition could not proceed at all (the *destination* failed).
///
/// Source read failures are the expected condition and end up in the [`Report`],
/// not here.
#[derive(Debug)]
pub struct AcquireError {
    source: io::Error,
    backtrace: Backtrace,
}

impl AcquireError {
    /// Backtrace captured where the failure was detected.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl From<io::Error> for AcquireError {
    fn from(source: io::Error) -> Self {
        Self {
            source,
            backtrace: Backtrace::capture(),
        }
    }
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write acquired image: {}", self.source)?;
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl Error for AcquireError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Acquires all of `src` into `dest` as a raw image.
///
/// Pass 1 sweeps sequentially in chunks, zero-filling any chunk that fails; pass 2
/// revisits each failed chunk sector by sector, writing every sector it can still
/// recover in place. The report maps whatever remains unreadable.
///
/// The caller must ensure `dest` does not resolve onto the medium behind `src`
/// (A-READ-ONLY): this function writes whatever destination it is handed. The
/// image is written from byte 0 of `dest`; `dest` is rewound before the sweep and
/// all later seeks are absolute.
///
/// `progress` is called as each pass advances, at most [`PROGRESS_STEPS`] times
/// per pass. It reports the shape of the work and nothing about its content
/// (`A-NO-CONTENT-IN-LOGS`).
///
/// # Errors
///
/// Fails only when writing or seeking `dest` fails; source damage is reported, not
/// raised.
///
/// # Panics
///
/// Panics if `options` sets a sweep chunk whose byte size does not fit `usize`
/// for this medium's sector size — an absurd configuration is a caller bug.
pub fn run<S, W>(
    src: &mut S,
    dest: &mut W,
    options: Options,
    progress: &mut dyn FnMut(Progress),
    cancelled: &dyn Fn() -> bool,
) -> Result<Report, AcquireError>
where
    S: BlockSource,
    W: Write + Seek,
{
    let geometry = src.geometry();
    let sector_bytes = geometry.sector_size.get() as usize;
    let sector_count = geometry.sector_count;

    let chunk_len = usize::try_from(options.chunk_sectors)
        .ok()
        .and_then(|sectors| sectors.checked_mul(sector_bytes))
        .unwrap_or_else(|| {
            panic!(
                "sweep chunk of {} sectors x {} does not fit usize",
                options.chunk_sectors, geometry.sector_size
            )
        });
    let mut buf = vec![0_u8; chunk_len];

    let swept = sweep(
        src,
        dest,
        options,
        &mut buf,
        progress,
        cancelled,
        sector_count,
    )?;
    let refined = refine(
        src,
        dest,
        &mut buf[..sector_bytes],
        progress,
        cancelled,
        geometry.sector_size,
        &swept,
    )?;

    dest.seek(SeekFrom::End(0))?;
    dest.flush()?;

    Ok(Report {
        sector_count,
        unreadable: refined.unreadable,
        not_attempted: swept.not_attempted.saturating_add(refined.not_attempted),
    })
}

/// What the sequential pass left behind.
struct Swept {
    /// Chunks the sweep could not read and zero-filled. Each is revisited
    /// sector by sector unless the run was stopped.
    suspects: Vec<SectorRange>,
    /// Sectors past where a stopped sweep got to.
    not_attempted: u64,
    /// Whether the sweep was stopped before it covered the medium.
    stopped: bool,
}

/// What refinement resolved.
struct Refined {
    /// Runs that stayed unreadable after being retried one sector at a time.
    unreadable: Vec<SectorRange>,
    /// Suspect sectors a stopped run never retried. Neither recovered nor known
    /// to be refused, so counted apart from both.
    not_attempted: u64,
}

/// Pass one: read the medium in chunks, skipping over what fails.
///
/// A failing chunk is zero-filled and noted rather than retried here, so the
/// healthy majority is off the medium quickly — which on a disk that is dying
/// is the difference between most of it and none of it.
fn sweep<S, W>(
    src: &mut S,
    dest: &mut W,
    options: Options,
    buf: &mut [u8],
    progress: &mut dyn FnMut(Progress),
    cancelled: &dyn Fn() -> bool,
    sector_count: u64,
) -> Result<Swept, AcquireError>
where
    S: BlockSource,
    W: Write + Seek,
{
    let sector_bytes = buf.len() / usize::try_from(options.chunk_sectors).unwrap_or(1).max(1);
    let mut suspects = Vec::new();
    dest.seek(SeekFrom::Start(0))?;
    let sweep_stride = stride(sector_count);
    let mut lba = Lba::new(0);
    while lba.get() < sector_count {
        // Checked once per chunk, so a stop takes effect within one read rather
        // than at the end of the medium. What is already written stays written:
        // the image is a prefix of the medium, and the report says how much of
        // it was never reached.
        if cancelled() {
            let not_attempted = sector_count - lba.get();
            return Ok(Swept {
                suspects,
                not_attempted,
                stopped: true,
            });
        }
        let sectors = options.chunk_sectors.min(sector_count - lba.get());
        let len = usize::try_from(sectors).unwrap_or_else(|_| {
            panic!("chunk sector count {sectors} fits usize: it is capped by the chunk size")
        }) * sector_bytes;
        let chunk = &mut buf[..len];
        if src.read_at(lba, chunk).is_ok() {
            dest.write_all(chunk)?;
        } else {
            chunk.fill(0);
            dest.write_all(chunk)?;
            suspects.push(SectorRange::new(lba, sectors));
        }
        lba = lba.checked_add(sectors).unwrap_or_else(|| {
            panic!("sweep position {lba}+{sectors} overflowed past sector count {sector_count}")
        });
        let done = lba.get();
        if done.is_multiple_of(sweep_stride) || done >= sector_count {
            progress(Progress::Swept {
                done,
                total: sector_count,
            });
        }
    }
    Ok(Swept {
        suspects,
        not_attempted: 0,
        stopped: false,
    })
}

/// Pass two: revisit each chunk the sweep skipped, one sector at a time.
///
/// Skipped entirely when the sweep was stopped: a suspect is a chunk the sweep
/// zero-filled, and one that is never retried is neither recovered nor known to
/// be refused, so its sectors are counted as untried.
fn refine<S, W>(
    src: &mut S,
    dest: &mut W,
    sector_buf: &mut [u8],
    progress: &mut dyn FnMut(Progress),
    cancelled: &dyn Fn() -> bool,
    sector_size: SectorSize,
    swept: &Swept,
) -> Result<Refined, AcquireError>
where
    S: BlockSource,
    W: Write + Seek,
{
    // The denominator is what the sweep could not read, not the medium: a disk
    // with one bad chunk and one with a thousand spend very different amounts
    // of time here, and only this number predicts which.
    let suspect_sectors: u64 = swept
        .suspects
        .iter()
        .fold(0, |sum, range| sum.saturating_add(range.sectors));
    if swept.stopped {
        return Ok(Refined {
            unreadable: Vec::new(),
            not_attempted: suspect_sectors,
        });
    }

    let mut unreadable = Vec::new();
    let refine_stride = stride(suspect_sectors);
    let mut refined = 0_u64;
    for suspect in &swept.suspects {
        for step in 0..suspect.sectors {
            // Once per sector: refinement is where a failing disk spends its
            // time, and a stop that waited for the whole pass would not be one.
            if cancelled() {
                return Ok(Refined {
                    unreadable,
                    not_attempted: suspect_sectors - refined,
                });
            }
            let sector = suspect.start.checked_add(step).unwrap_or_else(|| {
                panic!(
                    "suspect sector {}+{step} overflowed; range was {suspect}",
                    suspect.start
                )
            });
            match src.read_at(sector, sector_buf) {
                Ok(()) => {
                    let offset = sector.to_byte_offset(sector_size).unwrap_or_else(|| {
                        panic!(
                            "byte offset of sector {sector} at {sector_size} overflowed u64 after \
                             the sweep already wrote it"
                        )
                    });
                    dest.seek(SeekFrom::Start(offset.get()))?;
                    dest.write_all(sector_buf)?;
                }
                Err(_) => push_merged(&mut unreadable, sector),
            }
            refined += 1;
            if refined.is_multiple_of(refine_stride) || refined >= suspect_sectors {
                progress(Progress::Refined {
                    done: refined,
                    total: suspect_sectors,
                });
            }
        }
    }
    Ok(Refined {
        unreadable,
        not_attempted: 0,
    })
}

/// Sectors between two progress reports over a pass of `total`, never zero.
fn stride(total: u64) -> u64 {
    total.div_ceil(PROGRESS_STEPS).max(1)
}

/// Appends `sector` to `runs`, extending the last run when adjacent. Sectors
/// arrive in ascending order, so this yields sorted, merged, disjoint runs.
fn push_merged(runs: &mut Vec<SectorRange>, sector: Lba) {
    if let Some(last) = runs.last_mut()
        && last.end() == Some(sector)
    {
        last.sectors += 1;
    } else {
        runs.push(SectorRange::new(sector, 1));
    }
}
