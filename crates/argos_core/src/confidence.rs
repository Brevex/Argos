use std::fmt;

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
