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
            Self::PartialOrThumbnail => "partial-or-thumbnail",
            Self::Reassembled => "reassembled",
            Self::ContiguousCarve => "contiguous-carve",
            Self::JournalResidue => "journal-residue",
            Self::FsMetadata => "fs-metadata",
        };
        f.write_str(name)
    }
}
