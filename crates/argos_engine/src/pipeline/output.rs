//! Stages F and H: measuring, hashing and writing what the stages found.
//!
//! The last thing a finding passes through: its bytes are decoded once to
//! measure it, hashed as they stream to the sink, and recorded with the hash
//! the engine computed at recovery time.

use super::*;

/// Measures an artifact's picture before the run decides whether to write it.
///
/// The dimensions are the only property of a recovered image that is both
/// cheap to establish and impossible to argue with, and they are what
/// separates a photograph from the thumbnail caches that dominate a used disk.
/// The decode also feeds the manifest, so a reader can tell the two apart
/// without opening a single file.
///
/// A triage label decided this once, and could not: measured against real
/// media its rules call 4128x3096 camera frames ambiguous and 258x258 cache
/// entries photographs. A label that unreliable must not choose what reaches
/// the output directory (`A-TRIAGE-NOT-VERDICT`).
pub(crate) struct Measure {
    /// Scratch for reading an artifact back, reused across findings.
    buf: Vec<u8>,
}

/// What reading an artifact back established about the picture.
#[derive(Clone, Debug, Default)]
pub(crate) struct Measured {
    /// Pixel dimensions, or `None` when the artifact does not decode.
    ///
    /// `None` is not a verdict: an artifact whose picture cannot be measured
    /// is written, because a decoder that gave up is not evidence that the
    /// bytes are worthless.
    pub pixels: Option<(u32, u32)>,
    /// What the picture records about itself and its camera.
    pub capture: argos_core::artifact::Capture,
}

impl Measure {
    /// Reads the artifact back and measures its picture.
    ///
    /// One read serves both answers, and both come from bytes whose digest was
    /// checked against the one recorded at recovery — a description read from
    /// a medium that changed underneath the scan would describe something else.
    fn measure<V: Read + Seek>(
        &mut self,
        view: &mut V,
        finding: &Finding,
        sha256: Digest,
    ) -> Measured {
        let nothing = Measured::default;
        let Ok(length) = usize::try_from(finding.length()) else {
            return nothing();
        };
        if length > argos_carve::decode::MAX_DECODE_BYTES {
            return nothing();
        }
        self.buf.clear();
        self.buf.reserve(length);
        let mut bytes = Assembled::new(view, &finding.extents);
        if bytes.read_to_end(&mut self.buf).is_err() || self.buf.len() != length {
            return nothing();
        }
        if Digest::new(Sha256::digest(&*self.buf).into()) != sha256 {
            return nothing();
        }
        Measured {
            pixels: argos_carve::decode::decode_rgba(finding.format, &self.buf)
                .map(|image| (image.width(), image.height())),
            capture: argos_carve::metadata(&self.buf),
        }
    }
}

/// Whether `dimensions` clear the floor this run was given.
///
/// An artifact that could not be measured clears it: the floor exists to keep
/// caches of small pictures out of the directory, not to punish a decoder.
fn clears_floor(dimensions: Option<(u32, u32)>, floor: u32) -> bool {
    dimensions.is_none_or(|(width, height)| width.max(height) >= floor)
}

/// Hashes every finding in medium order and hands the survivors to the sink.
///
/// Sequential on purpose: the order artifacts reach the sink is the order they
/// appear in the manifest, and that order must not depend on how many workers
/// the machine had. Returns what was actually persisted, which is what the
/// triage stage is allowed to see.
pub(super) fn emit<V, S, P>(
    control: &Control,
    views: &mut [V],
    findings: &[Finding],
    sink: &mut S,
    min_long_side: u32,
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
        sink,
        measure: Measure { buf: Vec::new() },
        min_long_side,
        seen: HashSet::with_capacity(findings.len()),
        buf: vec![0_u8; STREAM_CHUNK_BYTES],
        stored_bytes: 0,
    };
    let mut done_work = 0_u64;
    for (index, finding) in findings.iter().enumerate() {
        // Between two artifacts, which is the only place this stage can stop
        // without leaving a half-written file: the manifest has to describe
        // files that are whole. A stage that reads no request at all is a stage
        // where the button does nothing, and on a system disk this is the stage
        // a run spends its time in.
        if control.stops_requested() > asked_before {
            break;
        }
        let disposition = writer.dispose(view, finding, index, progress, report)?;

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
    sink: &'a mut S,
    /// What measures each artifact's picture, and the floor it must clear.
    measure: Measure,
    min_long_side: u32,
    /// Digests already stored, so one file recovered twice is stored once.
    seen: HashSet<Digest>,
    /// Working memory for the hashing read, reused by every finding.
    buf: Vec<u8>,
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
        index: usize,
        progress: &P,
        report: &mut ScanReport,
    ) -> Result<Option<crate::annotate::Emitted>, ScanError>
    where
        V: Read + Seek,
        P: ProgressSink + ?Sized,
    {
        let expected = finding.length();
        let Some((sha256, read)) = hash_extents(view, &finding.extents, &mut self.buf) else {
            // The bytes this finding claims cannot be read back. It is not
            // reported, and nothing is invented in its place.
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

        let measured = self.measure.measure(view, finding, sha256);
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
        if !clears_floor(dimensions, self.min_long_side) {
            self.sink
                .omit(&artifact, "below-size-floor")
                .map_err(ScanError::sink)?;
            report.omitted_assets = report.omitted_assets.saturating_add(1);
            return Ok(None);
        }

        // The sink reads through a hasher, so the digest in the manifest is
        // checked against the bytes the sink actually received rather than
        // against an earlier, separate read of the same extents.
        let mut bytes = Hashing::new(Assembled::new(view, &finding.extents));
        self.sink
            .accept(&artifact, &mut bytes)
            .map_err(ScanError::sink)?;
        if bytes.finish() != sha256 {
            // The medium answered differently between the two reads. Nothing
            // recovered from it can be trusted, so the run stops rather than
            // record a hash that does not describe the stored bytes.
            return Err(ScanError::unstable_medium(finding.start()));
        }
        report.artifacts = report.artifacts.saturating_add(1);
        self.stored_bytes = self.stored_bytes.saturating_add(expected);
        progress.emit(ScanEvent::ArtifactStored {
            artifacts: report.artifacts,
            bytes: self.stored_bytes,
        });
        Ok(Some(crate::annotate::Emitted {
            finding: index,
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
