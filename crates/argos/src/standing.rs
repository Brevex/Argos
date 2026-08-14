//! Where a recorded artifact stands, whether or not the scan wrote it down.
//!
//! One place, because three readers need the same answer — the report, the
//! export filter and the gallery — and three implementations of an ordering
//! would be three orderings (`A-ONE-IMPLEMENTATION`).
//!
//! A session written before standings existed carries none. It is still
//! ordered, and correctly: the standing is derived from dimensions, camera,
//! capture date and same-size neighbours, and a manifest records all four
//! beside every artifact. So an older session sorts exactly as a new one does,
//! without re-reading the medium and without a migration.

use argos_classify::rank::{Evidence, Standing};
use argos_report::ArtifactRecord;

/// The standing of `record`: the one the scan recorded, or the one its own
/// fields imply.
///
/// Deriving rather than defaulting is what makes this work on a session the
/// running version did not produce. The two agree by construction — the scan
/// computes it from the same four facts it then writes down.
#[must_use]
pub fn of(record: &ArtifactRecord) -> Standing {
    if let Some(recorded) = record
        .standing
        .as_deref()
        .and_then(|text| text.parse::<Standing>().ok())
    {
        return recorded;
    }

    // The picture's measured size, falling back to what its metadata declares
    // when the decode produced nothing — exactly as the export filter does.
    let pixels = match (
        record.width.or(record.declared_width),
        record.height.or(record.declared_height),
    ) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    };
    let evidence = Evidence {
        pixels,
        camera_named: record.camera_make.is_some() || record.camera_model.is_some(),
        dated: record.taken.is_some(),
        same_size_neighbours: record.same_size_neighbours,
    };
    argos_classify::rank::standing(&evidence)
}

/// Sort order of a record's standing, strongest last.
#[must_use]
pub fn rank(record: &ArtifactRecord) -> u8 {
    match of(record) {
        Standing::CacheNeighbour => 0,
        Standing::Unremarkable => 1,
        Standing::PhotographSized => 2,
        Standing::Dated => 3,
        Standing::CameraNamed => 4,
    }
}

/// Long side of a record's picture, zero when nothing said what it was.
#[must_use]
pub fn long_side(record: &ArtifactRecord) -> u32 {
    record
        .width
        .unwrap_or(0)
        .max(record.height.unwrap_or(0))
        .max(record.declared_width.unwrap_or(0))
        .max(record.declared_height.unwrap_or(0))
}
