//! Test doubles for the engine's ports: an in-memory sink and event collector.
//!
//! These exist so a scan can be asserted on end to end without a filesystem,
//! a device or a temp directory anywhere in the test.

use std::io::{Read, Seek};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use argos_core::ports::{Artifact, ArtifactSink, Digest, ProgressSink, ScanEvent};
use argos_core::{ByteOffset, ByteRange, Confidence, Format, Stage, Timestamps};

/// One artifact as a [`Collector`] received it: provenance plus the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Collected {
    /// Image format.
    pub format: Format,
    /// Stage that produced it.
    pub stage: Stage,
    /// Evidence tier.
    pub confidence: Confidence,
    /// Source extents, in file order.
    pub extents: Vec<ByteRange>,
    /// Digest recorded at recovery time.
    pub sha256: Digest,
    /// Length the source metadata claimed, when it said one.
    pub expected_length: Option<u64>,
    /// Timestamps recovered from the source metadata.
    pub timestamps: Timestamps,
    /// Name recovered from filesystem metadata, if any.
    pub recovered_name: Option<String>,
    /// Filesystem object the metadata came from.
    pub source_object: Option<u64>,
    /// Parent candidate, for embedded thumbnails.
    pub parent: Option<ByteOffset>,
    /// What the picture records about itself and its camera.
    pub capture: argos_core::ports::Capture,
    /// The bytes the sink was handed.
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Collected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Neither the recovered name nor the recovered bytes are printable
        // content, even in a test failure (A-NO-CONTENT-IN-LOGS).
        f.debug_struct("Collected")
            .field("format", &self.format)
            .field("stage", &self.stage)
            .field("confidence", &self.confidence)
            .field("extents", &self.extents)
            .field("sha256", &self.sha256)
            .field("expected_length", &self.expected_length)
            .field("timestamps", &self.timestamps)
            .field(
                "recovered_name",
                &self.recovered_name.as_ref().map(|_| "<redacted>"),
            )
            .field("source_object", &self.source_object)
            .field("parent", &self.parent)
            .field("capture", &self.capture)
            .field("bytes", &format_args!("{} bytes", self.bytes.len()))
            .finish()
    }
}

/// An [`ArtifactSink`] that keeps everything it is given, in order.
#[derive(Debug, Default)]
pub struct Collector {
    artifacts: Vec<Collected>,
    /// When set, the sink refuses the artifact at this index, so a test can
    /// prove that a failing sink stops the scan instead of losing results.
    fail_at: Option<usize>,
}

impl Collector {
    /// A sink that accepts everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A sink that refuses the artifact at `index`.
    #[must_use]
    pub fn failing_at(index: usize) -> Self {
        Self {
            artifacts: Vec::new(),
            fail_at: Some(index),
        }
    }

    /// Everything accepted so far, in the order the engine emitted it.
    #[must_use]
    pub fn artifacts(&self) -> &[Collected] {
        &self.artifacts
    }
}

impl ArtifactSink for Collector {
    type Error = std::io::Error;

    fn accept<R: Read + ?Sized>(
        &mut self,
        artifact: &Artifact<'_>,
        bytes: &mut R,
    ) -> Result<(), Self::Error> {
        if self.fail_at == Some(self.artifacts.len()) {
            return Err(std::io::Error::other("collector refused this artifact"));
        }
        let mut collected = Vec::new();
        bytes.read_to_end(&mut collected)?;
        self.artifacts.push(Collected {
            format: artifact.format,
            stage: artifact.stage,
            confidence: artifact.confidence,
            extents: artifact.extents.to_vec(),
            sha256: artifact.sha256,
            expected_length: artifact.expected_length,
            timestamps: artifact.timestamps,
            recovered_name: artifact.recovered_name.map(str::to_owned),
            source_object: artifact.source_object,
            parent: artifact.parent,
            capture: artifact.capture.clone(),
            bytes: collected,
        });
        Ok(())
    }
}

/// A view that counts what the engine asks the medium for.
///
/// How many times a scan reads each artifact is not visible from its results —
/// a stage that reads every artifact three times produces exactly what one
/// that reads it once produces, and only costs three times as much. On a
/// rotational medium that is the difference between a recovery that finishes
/// and one that is abandoned, so it is a property worth a test of its own.
#[derive(Debug)]
pub struct Counted<V> {
    inner: V,
    bytes: Arc<AtomicU64>,
    reads: Arc<AtomicU64>,
}

/// What a set of [`Counted`] views were asked for, together.
#[derive(Clone, Debug, Default)]
pub struct Reads {
    bytes: Arc<AtomicU64>,
    reads: Arc<AtomicU64>,
}

impl Reads {
    /// A fresh tally.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A view over `inner` that reports into this tally.
    pub fn watching<V>(&self, inner: V) -> Counted<V> {
        Counted {
            inner,
            bytes: Arc::clone(&self.bytes),
            reads: Arc::clone(&self.reads),
        }
    }

    /// Bytes handed back by every view so far.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Read calls every view has answered so far.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
}

impl<V: Read> Read for Counted<V> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.bytes.fetch_add(read as u64, Ordering::Relaxed);
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(read)
    }
}

impl<V: Seek> Seek for Counted<V> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// A [`ProgressSink`] that records every event it is given.
#[derive(Debug, Default)]
pub struct Events {
    seen: Mutex<Vec<ScanEvent>>,
}

impl Events {
    /// A collector with no events yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event received so far.
    #[must_use]
    pub fn seen(&self) -> Vec<ScanEvent> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl ProgressSink for Events {
    fn emit(&self, event: ScanEvent) {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}
