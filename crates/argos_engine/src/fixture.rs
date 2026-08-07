//! Test doubles for the engine's ports: an in-memory sink and event collector.
//!
//! These exist so a scan can be asserted on end to end without a filesystem,
//! a device or a temp directory anywhere in the test.

use std::io::Read;
use std::sync::Mutex;

use argos_core::artifact::{Artifact, ArtifactSink, Digest};
use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::progress::{ProgressSink, ScanEvent};
use argos_core::{Confidence, Format, Stage, Timestamps};

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
            bytes: collected,
        });
        Ok(())
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
