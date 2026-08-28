//! Where a recovered artifact stands in a list, by the evidence about it.
//!
//! A whole-disk recovery produces hundreds of thousands of artifacts and a few
//! hundred photographs. Measured on a 1 TB disk of ten years' use: 348,361
//! records, of which 952 carried a camera capture date and 4,738 declared a
//! frame of 1024 pixels or more. Both numbers are small, and both were
//! unfindable — the photographs were there and the person looking could not
//! reach them.
//!
//! So this orders. It does **not** estimate a likelihood, and there is no
//! probability here for the same reason [`Decision`] carries none: the facts
//! behind a photograph are either recorded or not, and a number attached to
//! that would have nothing behind it. [`Standing`] names the strongest fact
//! that placed an artifact where it is, which is checkable against the picture
//! in front of the examiner.
//!
//! Nothing here removes, hides or reclassifies anything
//! (`A-TRIAGE-NOT-VERDICT`). It is a sort key, and every artifact has one.
//!
//! It is also derived entirely from fields the manifest already records —
//! dimensions, capture metadata, same-size neighbours — so a standing can be
//! recomputed from a session directory alone, with no re-reading of the medium
//! and no version to pin against.
//!
//! [`Decision`]: argos_core::ports::Decision

use std::fmt;

use argos_core::ports::Capture;

/// Smallest long side, in pixels, that a frame of a photograph has.
///
/// 640x480 is the smallest resolution the consumer cameras of the target era
/// produced, and it is what the oldest capture on the measured disk records —
/// a `Canon PowerShot` frame from 2007. Below it a frame may still be a
/// photograph, and it is never hidden; it simply carries no evidence of being
/// one from its size alone (`M-DOCUMENTED-MAGIC`).
pub(crate) const PHOTOGRAPH_MIN_LONG_SIDE: u32 = 640;

/// What is known about one recovered artifact, as far as ordering cares.
///
/// Every field is something the report stage already established: the decode
/// it performs to measure the picture, the `APP1` segment it reads for the
/// manifest, and the layout of the artifacts around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    /// Decoded pixel dimensions, or `None` when the picture did not decode.
    pub pixels: Option<(u32, u32)>,
    /// Whether the picture names the camera that took it.
    pub camera_named: bool,
    /// Whether the picture records when it was taken.
    pub dated: bool,
    /// How many same-sized neighbours the artifact was found among, when it
    /// was found among a run of them.
    pub same_size_neighbours: Option<u32>,
}

impl Evidence {
    /// The evidence in `capture` and `pixels`, before neighbours are known.
    ///
    /// The report stage measures a picture and reads its metadata in one pass;
    /// the run of neighbours around it is only known once every artifact has
    /// been written, so it arrives separately through
    /// [`Evidence::among_neighbours`].
    #[must_use]
    pub fn measured(pixels: Option<(u32, u32)>, capture: &Capture) -> Self {
        Self {
            pixels,
            camera_named: capture.make.is_some() || capture.model.is_some(),
            dated: capture.taken.is_some(),
            same_size_neighbours: None,
        }
    }

    /// The same evidence, with the size of the run this artifact sits in.
    #[must_use]
    pub fn among_neighbours(mut self, neighbours: u32) -> Self {
        self.same_size_neighbours = Some(neighbours);
        self
    }

    /// Long side of the decoded picture, zero when it did not decode.
    fn long_side(self) -> u32 {
        self.pixels.map_or(0, |(width, height)| width.max(height))
    }
}

/// Where an artifact stands, named by the strongest fact that put it there.
///
/// Ordered weakest first, so sorting descending puts what a person is looking
/// for at the top. The order is a claim about *evidence*, not about worth: an
/// [`Standing::Unremarkable`] artifact may well be a photograph whose metadata
/// did not survive, which is why nothing is ever hidden by it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Standing {
    /// Found among a run of same-sized neighbours: the layout a thumbnail
    /// cache has, and what a used disk holds most of.
    CacheNeighbour,
    /// Nothing recorded about it suggests a photograph either way.
    #[default]
    Unremarkable,
    /// Its frame is at least `PHOTOGRAPH_MIN_LONG_SIDE` on the long side.
    PhotographSized,
    /// It records when it was taken.
    Dated,
    /// It names the camera that took it — the one property a cache entry, an
    /// icon and a sprite never have.
    CameraNamed,
}

impl fmt::Display for Standing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::CacheNeighbour => "cache-neighbour",
            Self::Unremarkable => "unremarkable",
            Self::PhotographSized => "photograph-sized",
            Self::Dated => "dated",
            Self::CameraNamed => "camera-named",
        };
        f.write_str(name)
    }
}

impl std::str::FromStr for Standing {
    type Err = UnknownStanding;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "cache-neighbour" => Ok(Self::CacheNeighbour),
            "unremarkable" => Ok(Self::Unremarkable),
            "photograph-sized" => Ok(Self::PhotographSized),
            "dated" => Ok(Self::Dated),
            "camera-named" => Ok(Self::CameraNamed),
            _ => Err(UnknownStanding),
        }
    }
}

/// The text did not name a [`Standing`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownStanding;

impl fmt::Display for UnknownStanding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not the name of a standing")
    }
}

impl std::error::Error for UnknownStanding {}

/// Where `evidence` places an artifact.
///
/// The strongest recorded fact wins, with one exception that runs the other
/// way: an artifact found among same-sized neighbours is a cache entry
/// whatever else it carries, because a cache of photographs still keeps the
/// camera metadata of the pictures it previews. The run of identical
/// dimensions is the stronger statement, and it is a fact about the medium's
/// layout rather than about the picture (`docs/defects/02-thumbnail-provenance.md`).
#[must_use]
pub fn standing(evidence: &Evidence) -> Standing {
    // A run of same-sized neighbours is what `cache_run` recognises, and it
    // only ever reports one when there were enough of them to mean something.
    if evidence.same_size_neighbours.is_some() {
        return Standing::CacheNeighbour;
    }
    if evidence.camera_named {
        return Standing::CameraNamed;
    }
    if evidence.dated {
        return Standing::Dated;
    }
    if evidence.long_side() >= PHOTOGRAPH_MIN_LONG_SIDE {
        return Standing::PhotographSized;
    }
    Standing::Unremarkable
}
