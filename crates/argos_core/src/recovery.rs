//! How an artifact was recovered and what it is: stage, format, evidence tier.
//!
//! This vocabulary is shared by every crate that produces, merges or reports a
//! finding, so a recovery stage and an evidence tier mean exactly one thing
//! across the workspace.

use std::fmt;
use std::time::SystemTime;

/// Timestamps recovered from filesystem metadata.
///
/// An absent field was not stored by the filesystem or did not survive. They
/// are never inferred from anything else and never defaulted to "now"
/// (A-PROVENANCE): for an image-possession question the recorded time is often
/// the most probative field there is, and a fabricated one is worse than none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamps {
    /// Creation time, where the filesystem records one.
    pub created: Option<SystemTime>,
    /// Last content modification time.
    pub modified: Option<SystemTime>,
}

impl Timestamps {
    /// Whether no timestamp survived.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.created.is_none() && self.modified.is_none()
    }
}

/// Pipeline stage that produced a finding.
///
/// The stages run cheapest-and-most-trusted first, so a finding's stage also
/// says how much work stood behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// Partition tables and the prior-filesystem residue sweep.
    Volumes,
    /// Filesystem metadata recovery over located volumes.
    Filesystem,
    /// Full-surface signature carving.
    Carve,
    /// Reassembly of images the medium stored in pieces.
    Reassembly,
    /// Structural validation, hashing and scoring of candidates.
    Validation,
    /// Writing artifacts and the manifest.
    Report,
    /// Rendering previews of persisted artifacts.
    Preview,
    /// ML triage: labeling artifacts photograph vs synthetic asset, after
    /// they are persisted.
    Triage,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Volumes => "volumes",
            Self::Filesystem => "filesystem",
            Self::Carve => "carve",
            Self::Reassembly => "reassembly",
            Self::Validation => "validation",
            Self::Report => "report",
            Self::Preview => "preview",
            Self::Triage => "triage",
        };
        f.write_str(name)
    }
}

/// Image format of a recovered artifact.
///
/// Exhaustive on purpose: a new format needs a validator, an extension, a
/// signature and a report mapping, and every one of those sites should fail to
/// compile until it is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    /// JFIF/EXIF JPEG (ITU-T T.81).
    Jpeg,
    /// PNG (ISO 15948).
    Png,
}

impl Format {
    /// Conventional file extension for the format.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

impl std::str::FromStr for Format {
    type Err = UnknownFormat;

    /// Parses the name [`Display`](fmt::Display) writes.
    ///
    /// This is how a format survives a round trip through a manifest, which is
    /// what lets a later run pick up where an earlier one left off.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "jpeg" => Ok(Self::Jpeg),
            "png" => Ok(Self::Png),
            _ => Err(UnknownFormat),
        }
    }
}

/// A name that is not one of the formats this tool handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownFormat;

impl fmt::Display for UnknownFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an image format this tool recovers")
    }
}

impl std::error::Error for UnknownFormat {}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
        };
        f.write_str(name)
    }
}

/// Evidence tier of a recovered artifact, ordered weakest to strongest.
///
/// The tier is fixed by how the artifact was obtained and is never raised by
/// post-processing; reporting a tier above the evidence would fabricate certainty.
/// The ladder is deliberately exhaustive — a new tier is a change to the recovery
/// model, not a routine addition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Medium bytes decoded inside a header this tool supplied.
    ///
    /// An entropy-coded fragment with no header of its own, entered at a
    /// restart marker and grafted onto the header of a surviving file from the
    /// same batch. The pixels came off the medium; the container did not, and
    /// the frame size is another file's. It is the floor of the ladder because
    /// it is the one tier whose artifact is **not a file the medium ever
    /// held** — an examiner looking at one is looking at real pixels in an
    /// arrangement this tool built.
    Grafted,
    /// A fragment or embedded thumbnail; the parent image was not recovered.
    PartialOrThumbnail,
    /// Reassembled from non-contiguous fragments by carving.
    Reassembled,
    /// Carved as one contiguous, fully validated run.
    ContiguousCarve,
    /// Extents recovered from stale filesystem journal copies.
    JournalResidue,
    /// Extents taken from live or residual filesystem metadata.
    FsMetadata,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Grafted => "grafted",
            Self::PartialOrThumbnail => "partial-or-thumbnail",
            Self::Reassembled => "reassembled",
            Self::ContiguousCarve => "contiguous-carve",
            Self::JournalResidue => "journal-residue",
            Self::FsMetadata => "fs-metadata",
        };
        f.write_str(name)
    }
}
