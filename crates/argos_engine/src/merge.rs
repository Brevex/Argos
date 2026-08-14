//! Turning the stages' overlapping claims into one ordered set of findings.
//!
//! Two stages routinely find the same image: filesystem metadata names it and
//! carving re-derives it from its signature. Merging keeps the strongest
//! evidence for each set of bytes and never invents a stronger tier than the
//! evidence that produced it (A-CONFIDENCE-HONEST). Content-hash deduplication
//! happens later, at emit time, because it needs the bytes.

use std::cmp::Reverse;
use std::collections::BTreeSet;

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
/// Returns how many were dropped for touching an unreadable range. That one is
/// counted and the others are not, because it is the only drop here that is
/// about the *medium* rather than about the findings: the signature that
/// started each of them was real, and a run that lost recoveries to damage must
/// not read as one that found the medium empty (A-CONFIDENCE-HONEST).
///
/// The resulting order is a total order over the findings' own fields, so two
/// runs over the same medium produce the same manifest.
pub(crate) fn consolidate(findings: &mut Vec<Finding>, unreadable: &[ByteRange]) -> u64 {
    let mut dropped_unreadable = 0_u64;
    let mut truncated = Vec::new();
    findings.retain_mut(|finding| {
        if finding.extents.is_empty() || finding.length() == 0 {
            return false;
        }
        if !unreadable.iter().any(|range| finding.intersects(*range)) {
            return true;
        }
        // The bytes before the damage are still the medium's own, and they are
        // still the start of this image. Damage is recorded at retry-span
        // granularity, so a bad sector condemns a whole span around it and the
        // photograph that reached into it loses everything — including the
        // part that read cleanly.
        dropped_unreadable += 1;
        if let Some(head) = head_before_damage(finding, unreadable) {
            truncated.push(head);
        }
        false
    });
    findings.append(&mut truncated);

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
    // Indices into `kept`, ordered by the byte each one reaches, furthest
    // first.
    //
    // A container has to reach at least as far as what it contains, so a
    // search can stop at the first candidate that does not reach far enough.
    // Without that order the search is against everything kept so far, and
    // there is a shape of medium that makes that quadratic: one weakly
    // evidenced carve spanning thousands of files the filesystem also named.
    // It cannot cover any of them — its evidence is weaker — so every one of
    // them is kept, and every one of them is then compared against every one
    // before it. That is a real disk, not a contrived one, and it costs hours
    // in a stage that says nothing while it runs.
    let mut by_reach: BTreeSet<(Reverse<u64>, usize)> = BTreeSet::new();
    let mut furthest_end = 0_u64;
    for finding in findings.drain(..) {
        let end = finding
            .extents
            .iter()
            .map(|extent| extent.end_saturating().get())
            .max()
            .unwrap_or(0);
        // Reaching past everything kept so far cannot be covered by it: the
        // common case, and the cheapest possible answer.
        let covered = end <= furthest_end
            && finding.parent.is_none()
            && by_reach
                .iter()
                .take_while(|(Reverse(reach), _)| *reach >= end)
                .any(|&(_, index)| {
                    let container = &kept[index];
                    container.confidence >= finding.confidence
                        && container.extents != finding.extents
                        && finding.is_covered_by(container)
                });
        if !covered {
            furthest_end = furthest_end.max(end);
            by_reach.insert((Reverse(end), kept.len()));
            kept.push(finding);
        }
    }
    *findings = kept;
    dropped_unreadable
}

/// The part of `finding` that lies before the first damage it meets.
///
/// Damage is recorded at the granularity a failed read is retried at, so one
/// bad sector condemns a whole span around it. A photograph reaching into that
/// span used to lose everything — including the extents that read cleanly and
/// are, byte for byte, the start of the picture.
///
/// So the head is kept and the rest is not. What comes back is the medium's
/// own bytes up to the damage, at the weakest tier and with the length the
/// metadata expected recorded beside it, exactly as a decoder-truncated
/// recovery is. Nothing is padded and no zero is ever presented as data
/// (`A-CONFIDENCE-HONEST`).
///
/// `None` when the damage starts at or before the first extent: there is no
/// head, only a finding whose bytes are unknown.
fn head_before_damage(finding: &Finding, unreadable: &[ByteRange]) -> Option<Finding> {
    let first_damage = unreadable
        .iter()
        .filter(|range| finding.intersects(**range))
        .map(|range| range.start.get())
        .min()?;

    let mut extents = Vec::new();
    for extent in &finding.extents {
        let start = extent.start.get();
        if start >= first_damage {
            break;
        }
        let len = extent.len.min(first_damage.saturating_sub(start));
        if len == 0 {
            break;
        }
        extents.push(ByteRange::new(extent.start, len));
        // A later extent may sit before the damage on the medium while a
        // earlier one already reached it; stopping here keeps the head a
        // prefix of the *file* rather than of the disk.
        if len < extent.len {
            break;
        }
    }
    if extents.is_empty() {
        return None;
    }

    Some(Finding {
        format: finding.format,
        stage: finding.stage,
        // Never above the weakest tier: what this describes is a file that
        // stops, and how it was found does not change that.
        confidence: argos_core::Confidence::PartialOrThumbnail,
        extents: extents.into_boxed_slice(),
        // What the file was supposed to be, so the shortfall is stated rather
        // than the head presented as whole.
        declared_size: finding.declared_size.or_else(|| Some(finding.length())),
        timestamps: finding.timestamps,
        deleted: finding.deleted,
        name: finding.name.clone(),
        source_object: finding.source_object,
        parent: finding.parent,
    })
}
