//! Turning the stages' overlapping claims into one ordered set of findings.
//!
//! Two stages routinely find the same image: filesystem metadata names it and
//! carving re-derives it from its signature. Merging keeps the strongest
//! evidence for each set of bytes and never invents a stronger tier than the
//! evidence that produced it (A-CONFIDENCE-HONEST). Content-hash deduplication
//! happens later, at emit time, because it needs the bytes.

use argos_core::Stage;
use argos_core::geometry::ByteRange;

use crate::finding::Finding;

/// Drops what cannot honestly be reported and orders what remains.
///
/// In order: findings with no bytes, findings whose bytes lie in a region the
/// medium could not read, exact-extent duplicates (strongest tier wins), and
/// findings entirely covered by another of at least equal tier. Embedded
/// thumbnails survive coverage — a thumbnail always lies inside its parent and
/// is a separate artifact.
///
/// The resulting order is a total order over the findings' own fields, so two
/// runs over the same medium produce the same manifest.
pub(crate) fn consolidate(findings: &mut Vec<Finding>, unreadable: &[ByteRange]) {
    findings.retain(|finding| {
        !finding.extents.is_empty()
            && finding.length() > 0
            && !unreadable.iter().any(|range| finding.intersects(*range))
    });

    findings.sort_by(|a, b| {
        a.start()
            .cmp(&b.start())
            // Longest first, so a container is seen before what it contains.
            .then(b.length().cmp(&a.length()))
            // Strongest evidence first, so it is the one kept.
            .then(b.confidence.cmp(&a.confidence))
            .then(a.stage.cmp(&b.stage))
            .then(a.format.cmp(&b.format))
            .then(a.extents.cmp(&b.extents))
    });

    // Exact-extent duplicates: keep the strongest, but carry over the metadata
    // the weaker one had and the stronger one lacks.
    findings.dedup_by(|dropped, kept| {
        if dropped.extents != kept.extents {
            return false;
        }
        // A name and the object it was read from move together or not at all.
        // Splicing one finding's name onto another's object would assert an
        // association that exists nowhere on the medium (A-PROVENANCE).
        if kept.name.is_none() && kept.source_object.is_none() {
            kept.name = dropped.name.take();
            kept.source_object = dropped.source_object;
        } else if kept.name.is_none() && kept.source_object == dropped.source_object {
            kept.name = dropped.name.take();
        } else if kept.source_object.is_none() && dropped.name.is_none() {
            kept.source_object = dropped.source_object;
        }
        if kept.timestamps.is_empty() && kept.source_object == dropped.source_object {
            kept.timestamps = dropped.timestamps;
        }
        if kept.declared_size.is_none() && kept.source_object == dropped.source_object {
            kept.declared_size = dropped.declared_size;
        }
        if kept.parent.is_none() && dropped.parent.is_some() {
            kept.parent = dropped.parent;
            // Being an embedded thumbnail is a fact about what these bytes
            // *are*, and it outranks how they were found: a thumbnail that
            // also carves cleanly on its own is still a thumbnail. Metadata
            // evidence is never overridden this way — only carving is.
            if kept.stage == Stage::Carve {
                kept.confidence = kept.confidence.min(dropped.confidence);
            }
        }
        true
    });

    let mut kept: Vec<Finding> = Vec::with_capacity(findings.len());
    let mut furthest_end = 0_u64;
    for finding in findings.drain(..) {
        let end = finding
            .extents
            .iter()
            .map(|extent| extent.end_saturating().get())
            .max()
            .unwrap_or(0);
        // Reaching past everything kept so far cannot be covered by it: the
        // common case, and what keeps this a linear sweep.
        let covered = end <= furthest_end
            && finding.parent.is_none()
            && kept.iter().rev().any(|container| {
                container.confidence >= finding.confidence
                    && container.extents != finding.extents
                    && finding.is_covered_by(container)
            });
        if !covered {
            furthest_end = furthest_end.max(end);
            kept.push(finding);
        }
    }
    *findings = kept;
}
