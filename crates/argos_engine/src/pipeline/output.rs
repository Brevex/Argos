//! Stages F and H: measuring, hashing and writing what the stages found.
//!
//! The last thing a finding passes through: its bytes are read off the medium,
//! hashed, measured, and handed to the sink with the hash the engine computed
//! at recovery time.
//!
//! An artifact that fits in memory is read **once** and everything below works
//! from that buffer, so the digest recorded and the bytes stored are the same
//! bytes by construction rather than by comparing two reads of them. One that
//! does not fit is streamed twice and the two reads are compared, which is
//! what [`ScanError::unstable_medium`] reports. On a rotational medium the
//! difference is three passes of the head per artifact against one.

use super::*;

/// Largest artifact the read buffer keeps its capacity for between findings.
///
/// A whole-disk recovery is overwhelmingly small artifacts with the occasional
/// large one, and holding the largest one's capacity for the rest of the run
/// would leave tens of megabytes resident to serve a few kilobytes at a time
/// (`M-SHRINK-TO-FIT`).
const KEEP_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// What reading an artifact back established about the picture.
///
/// The dimensions are the only property of a recovered image that is both
/// cheap to establish and impossible to argue with, and they are what
/// separates a photograph from the thumbnail caches that dominate a used disk.
/// They also feed the manifest, so a reader can tell the two apart without
/// opening a single file.
///
/// A triage label decided this once, and could not: measured against real
/// media its rules call 4128x3096 camera frames ambiguous and 258x258 cache
/// entries photographs. A label that unreliable must not choose what reaches
/// the output directory (`A-TRIAGE-NOT-VERDICT`).
#[derive(Clone, Debug, Default)]
pub struct Measured {
    /// Pixel dimensions the frame header declares, or `None` when no header
    /// this engine can read is there.
    ///
    /// `None` is not a verdict: an artifact whose picture cannot be measured
    /// is written, because a decoder that gave up is not evidence that the
    /// bytes are worthless.
    pub pixels: Option<(u32, u32)>,
    /// What the picture records about itself and its camera.
    pub capture: argos_core::ports::Capture,
}

/// Measures the picture `bytes` holds.
///
/// Both answers come from the header: the frame declares its own size, and the
/// metadata walk stops at the start of the scan. Decoding the pixels would
/// establish the same dimensions — [`argos_carve::decode::decode_rgba`] sizes
/// its expectation from these very numbers and refuses an image that does not
/// match them — at the cost of materializing a plane of up to 256 MiB that
/// nothing here would read.
fn measure(format: Format, bytes: &[u8]) -> Measured {
    Measured {
        pixels: argos_carve::decode::dimensions(format, bytes),
        capture: argos_carve::metadata(bytes),
    }
}

/// Whether `dimensions` clear the floor this run was given.
///
/// An artifact whose frame could not be read at all clears it: the floor
/// exists to keep caches of small pictures out of the directory, and a picture
/// whose size is unknown is not known to be small.
fn clears_floor(dimensions: Option<(u32, u32)>, floor: u32) -> bool {
    dimensions.is_none_or(|(width, height)| width.max(height) >= floor)
}

/// Hashes every finding in medium order and hands the survivors to the sink.
///
/// Sequential on purpose: the order artifacts reach the sink is the order they
/// appear in the manifest, and that order must not depend on how many workers
/// the machine had. Returns what was actually persisted, which is what the
/// triage stage is allowed to see.
/// Where each artifact goes, and what happens to it once it is there.
///
/// The three travel together because they are one decision made three times
/// over: what clears the floor, what stores it, and what describes it
/// afterwards.
pub(super) struct Output<'a, S> {
    /// Where artifacts are stored and previews are rendered.
    pub sink: &'a mut S,
    /// Smallest long side an artifact needs to reach the output directory.
    pub min_long_side: u32,
    /// What describes each artifact once the sink has taken it.
    pub annotating: &'a mut crate::annotate::Annotating,
}

pub(super) fn emit<V, S, P>(
    control: &Control,
    views: &mut [V],
    findings: &[Finding],
    output: Output<'_, S>,
    progress: &P,
    report: &mut ScanReport,
) -> Result<Vec<crate::annotate::Emitted>, ScanError>
where
    V: Read + Seek,
    S: ArtifactSink,
    P: ProgressSink + ?Sized,
{
    let mut emitted = Vec::new();
    let Some(view) = views.first_mut() else {
        return Ok(emitted);
    };
    // Cancelling means "stop searching and write what you have". The findings
    // below are everything the earlier stages established, and on a large
    // medium they are hours of reading; a stop aimed at the search must not
    // take them with it. Stopping the writing as well takes a second request,
    // made while the writing is what is running — which is why this is a count
    // read on entry rather than a flag.
    let asked_before = control.stops_requested();
    // The work this stage has to get through: every finding costs a read of
    // its extents whatever becomes of it. This is the denominator of the
    // progress figure, and the numerator counts findings *disposed of* rather
    // than findings stored — a duplicate, an unreadable run and an artifact a
    // caller asked not to write all cost this stage the same read, so counting
    // only the stored ones is what leaves a bar resting short of the end on a
    // run that did everything it could. What was actually stored is a separate
    // figure, reported by `ArtifactStored` and totalled in the manifest.
    let total_work = findings
        .iter()
        .fold(0_u64, |sum, finding| sum.saturating_add(finding.length()));
    progress.emit(ScanEvent::StageStarted {
        stage: Stage::Report,
        unit: Unit::Bytes,
        total: total_work,
    });

    let mut writer = Writing {
        output,
        seen: HashSet::with_capacity(findings.len()),
        whole: Vec::new(),
        chunk: vec![0_u8; STREAM_CHUNK_BYTES],
        stored_bytes: 0,
    };
    let mut done_work = 0_u64;
    for finding in findings {
        // Between two artifacts, which is the only place this stage can stop
        // without leaving a half-written file: the manifest has to describe
        // files that are whole. A stage that reads no request at all is a stage
        // where the button does nothing, and on a system disk this is the stage
        // a run spends its time in.
        if control.stops_requested() > asked_before {
            break;
        }
        let disposition = writer.dispose(view, finding, progress, report)?;

        done_work = done_work.saturating_add(finding.length());
        progress.emit(ScanEvent::StageProgress {
            stage: Stage::Report,
            unit: Unit::Bytes,
            done: done_work,
            total: total_work,
        });
        if let Some(item) = disposition {
            emitted.push(item);
        }
    }

    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Report,
        findings: report.artifacts,
    });
    Ok(emitted)
}

/// What the writing stage carries from one finding to the next.
struct Writing<'a, S> {
    output: Output<'a, S>,
    /// Digests already stored, so one file recovered twice is stored once.
    seen: HashSet<Digest>,
    /// One whole artifact, read once and used for the digest, the measurement
    /// and the bytes handed to the sink. Reused by every finding.
    whole: Vec<u8>,
    /// Working memory for the streaming path's hashing read, which never holds
    /// a whole artifact. Reused by every finding that takes it.
    chunk: Vec<u8>,
    /// Bytes handed to the sink so far.
    stored_bytes: u64,
}

impl<S: ArtifactSink> Writing<'_, S> {
    /// Settles one finding: stores it, records it unwritten, or drops it.
    ///
    /// `Ok(None)` means nothing was stored, and the counter in `report` says
    /// which of the reasons it was. The error cases are a sink that refused
    /// and a medium that changed underneath the run; both end the scan.
    fn dispose<V, P>(
        &mut self,
        view: &mut V,
        finding: &Finding,
        progress: &P,
        report: &mut ScanReport,
    ) -> Result<Option<crate::annotate::Emitted>, ScanError>
    where
        V: Read + Seek,
        P: ProgressSink + ?Sized,
    {
        let expected = finding.length();
        // Which of the two shapes this artifact takes. Both are bounded by a
        // constant of this engine's own, never by a length the medium stated
        // (`A-BOUNDED-ALLOC`).
        let buffered = usize::try_from(expected)
            .ok()
            .filter(|length| *length <= argos_carve::decode::MAX_DECODE_BYTES);

        let (sha256, measured) = if let Some(length) = buffered {
            let Some(sha256) = read_extents(view, &finding.extents, length, &mut self.whole) else {
                // The bytes this finding claims cannot be read back. It is not
                // reported, and nothing is invented in its place.
                report.unrecoverable = report.unrecoverable.saturating_add(1);
                return Ok(None);
            };
            if !self.seen.insert(sha256) {
                report.duplicates = report.duplicates.saturating_add(1);
                return Ok(None);
            }
            (sha256, measure(finding.format, &self.whole))
        } else {
            let Some((sha256, read)) = hash_extents(view, &finding.extents, &mut self.chunk) else {
                report.unrecoverable = report.unrecoverable.saturating_add(1);
                return Ok(None);
            };
            if read != expected {
                report.unrecoverable = report.unrecoverable.saturating_add(1);
                return Ok(None);
            }
            if !self.seen.insert(sha256) {
                report.duplicates = report.duplicates.saturating_add(1);
                return Ok(None);
            }
            // Past the decode bound there is no header this engine will read
            // and no plane it would materialize, so nothing is claimed about
            // the picture.
            (sha256, Measured::default())
        };

        let artifact = Artifact {
            format: finding.format,
            stage: finding.stage,
            confidence: finding.confidence,
            extents: &finding.extents,
            length: expected,
            expected_length: finding.declared_size,
            sha256,
            timestamps: finding.timestamps,
            deleted: finding.deleted,
            recovered_name: finding.name.as_deref(),
            source_object: finding.source_object,
            parent: finding.parent,
            // The picture is measured before anything is written, because that
            // is the decision below: an image too small to be a photograph
            // stays out of the directory. It is recorded either way, with its
            // dimensions, so the manifest describes the medium whole even when
            // the directory does not, and the extents locate the bytes exactly
            // for a rerun with a lower floor.
            pixels: measured.pixels,
            capture: &measured.capture,
        };
        let dimensions = artifact.pixels;
        if !clears_floor(dimensions, self.output.min_long_side) {
            self.output
                .sink
                .omit(&artifact, "below-size-floor")
                .map_err(ScanError::sink)?;
            report.omitted_assets = report.omitted_assets.saturating_add(1);
            return Ok(None);
        }

        if buffered.is_some() {
            // The sink is handed the very bytes the digest was taken over, so
            // the manifest describes what was stored without a second read of
            // the medium to prove it. There is no window between the two for
            // the medium to answer differently.
            self.output
                .sink
                .accept(&artifact, &mut std::io::Cursor::new(&*self.whole))
                .map_err(ScanError::sink)?;
        } else {
            // Nothing held this artifact's bytes, so the sink reads the medium
            // again and the digests of the two reads are compared.
            let mut bytes = Hashing::new(Assembled::new(view, &finding.extents));
            self.output
                .sink
                .accept(&artifact, &mut bytes)
                .map_err(ScanError::sink)?;
            if bytes.finish() != sha256 {
                // The medium answered differently between the two reads.
                // Nothing recovered from it can be trusted, so the run stops
                // rather than record a hash that does not describe the stored
                // bytes.
                return Err(ScanError::unstable_medium(finding.start()));
            }
        }
        // Stored, hashed and recorded. Only now is the picture decoded, and
        // only from the bytes the sink was handed — an annotation cannot reach
        // any decision above, because it produces no value any of them read
        // (`A-TRIAGE-NOT-VERDICT`).
        if buffered.is_some() {
            self.output.annotating.annotate(
                self.output.sink,
                sha256,
                finding.format,
                &self.whole,
                report,
            );
        } else {
            // Nothing held this artifact's bytes and it is past the ceiling
            // anything here would decode, so it is recorded unannotated.
            self.output.annotating.undecoded(sha256);
        }

        report.artifacts = report.artifacts.saturating_add(1);
        self.stored_bytes = self.stored_bytes.saturating_add(expected);
        progress.emit(ScanEvent::ArtifactStored {
            artifacts: report.artifacts,
            bytes: self.stored_bytes,
        });
        Ok(Some(crate::annotate::Emitted {
            sha256,
            offset: finding.start().get(),
            pixels: dimensions,
            // Carried from the measure the floor decision already made, so the
            // ordering below costs no second decode and no second read of the
            // metadata.
            evidence: argos_classify::rank::Evidence::measured(dimensions, &measured.capture),
        }))
    }
}

/// Wraps a reader and digests everything that passes through it.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Hashing<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// The digest of everything read so far.
    fn finish(self) -> Digest {
        Digest::new(self.hasher.finalize().into())
    }
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}

/// Reads the concatenated extents into `buf` and returns their digest.
///
/// One forward pass of the medium: the extents are visited in order and each
/// is seeked to once, so an artifact costs its own length in reads rather than
/// a multiple of it.
///
/// `None` when the medium refuses part of the range or answers with a
/// different number of bytes than the finding claims — in both cases the
/// finding describes something that is not there, and nothing is recorded for
/// it.
fn read_extents<V: Read + Seek>(
    view: &mut V,
    extents: &[ByteRange],
    length: usize,
    buf: &mut Vec<u8>,
) -> Option<Digest> {
    buf.clear();
    // One large artifact leaves a large buffer behind it. Hand the capacity
    // back before the next finding, unless that one needs it too
    // (`M-SHRINK-TO-FIT`).
    let keep = KEEP_BUFFER_BYTES.max(length);
    if buf.capacity() > keep {
        buf.shrink_to(keep);
    }
    // Bounded by the caller against `MAX_DECODE_BYTES` before we get here, so
    // this reserves against a ceiling of this engine's own rather than against
    // a length the medium stated (`A-BOUNDED-ALLOC`).
    buf.reserve(length);
    Assembled::new(view, extents).read_to_end(buf).ok()?;
    if buf.len() != length {
        return None;
    }
    Some(Digest::new(Sha256::digest(&*buf).into()))
}

/// SHA-256 of the concatenated extents, with the byte count actually read.
/// `None` when the medium refuses part of the range.
fn hash_extents<V: Read + Seek>(
    view: &mut V,
    extents: &[ByteRange],
    buf: &mut [u8],
) -> Option<(Digest, u64)> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut bytes = Assembled::new(view, extents);
    loop {
        let read = bytes.read(buf).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        total = total.saturating_add(read as u64);
    }
    Some((Digest::new(hasher.finalize().into()), total))
}
