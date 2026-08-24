//! Stage D: turning signature candidates into findings.
//!
//! A magic-byte hit is not a recovery. Every candidate is driven through its
//! format's state machine, and only a stream that validates end to end
//! becomes a contiguous-carve finding.

use super::*;

/// Drives every signature hit through its format's state machine.
///
/// Each worker owns one view of the medium, so validation is parallel without
/// the views ever being shared. A hit that fails to validate is counted, never
/// reported: a magic number is not evidence.
fn validate_candidates<V, P>(
    control: &Control,
    views: &mut [V],
    candidates: Vec<Candidate>,
    medium_end: u64,
    progress: &P,
    report: &mut ScanReport,
) -> (Vec<Finding>, Vec<Broken>)
where
    V: Read + Seek + Send,
    P: ProgressSink + ?Sized,
{
    // Announced even when there is nothing to do, so the stage order a client
    // sees is the stage order that ran.
    let counter = Counter::start(
        progress,
        Stage::Validation,
        candidates.len() as u64,
        Unit::Items,
    );
    if candidates.is_empty() || views.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let (queue_tx, queue_rx) = crossbeam_channel::unbounded::<Candidate>();
    for candidate in candidates {
        // The queue is unbounded and pre-filled: it holds work already in
        // memory, so there is nothing to apply backpressure against.
        let _ = queue_tx.send(candidate);
    }
    drop(queue_tx);

    let outputs = thread::scope(|scope| {
        let mut workers = Vec::with_capacity(views.len());
        for view in views.iter_mut() {
            let queue = queue_rx.clone();
            let counter = &counter;
            workers.push(scope.spawn(move || {
                let mut scratch = Scratch::new();
                let mut found = Vec::new();
                let mut broken = Vec::new();
                let mut rejected = 0_u64;
                while let Ok(candidate) = queue.recv() {
                    if control.is_cancelled() {
                        break;
                    }
                    let validated = validate_one(view, candidate, medium_end, &mut scratch);
                    if validated.rejected {
                        rejected += 1;
                    }
                    found.extend(validated.findings);
                    broken.extend(validated.broken);
                    counter.step();
                }
                (found, broken, rejected)
            }));
        }
        drop(queue_rx);
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Vec<_>>()
    });

    let mut found = Vec::new();
    let mut broken = Vec::new();
    for (results, breaks, rejected) in outputs {
        found.extend(results);
        broken.extend(breaks);
        report.rejected_candidates = report.rejected_candidates.saturating_add(rejected);
    }
    // Deterministic order, so the stage that follows does not depend on which
    // worker finished first.
    broken.sort_unstable_by_key(|entry: &Broken| (entry.header, entry.break_at));
    (found, broken)
}

/// What validating one candidate established.
#[derive(Default)]
struct Validated {
    /// The image itself when it validated, plus its thumbnail when that did.
    findings: Vec<Finding>,
    /// Where the stream stopped being this file, when it broke. This is what
    /// reassembly starts from, so it comes from the entropy decoder rather
    /// than the marker grammar (see `argos_carve::reassemble::locate_break`).
    broken: Option<Broken>,
    /// Whether the candidate produced no finding at all.
    rejected: bool,
}

/// Validates one candidate.
///
/// A candidate that breaks is not simply discarded: it may still carry an
/// embedded thumbnail, and it always carries a fragmentation point, which is
/// the only thing that makes reassembly tractable.
fn validate_one<V: Read + Seek>(
    view: &mut V,
    candidate: Candidate,
    medium_end: u64,
    scratch: &mut Scratch,
) -> Validated {
    let limit = medium_end.min(
        candidate
            .offset
            .get()
            .saturating_add(argos_carve::MAX_IMAGE_BYTES),
    );
    let Ok(verdict) =
        argos_carve::validate(candidate.format, view, candidate.offset, limit, scratch)
    else {
        // A candidate we cannot read is a candidate we cannot report.
        return Validated {
            rejected: true,
            ..Validated::default()
        };
    };

    let (complete, thumbnail) = match verdict {
        Verdict::Complete { length, thumbnail } => (Some(length), thumbnail),
        Verdict::Corrupt { thumbnail, .. } => (None, thumbnail),
    };

    // A candidate that did not carve whole is where reassembly begins.
    let broken = if complete.is_none() {
        argos_carve::reassemble::locate_break(
            view,
            candidate.offset,
            candidate.format,
            medium_end,
            scratch,
        )
        .ok()
        .flatten()
    } else {
        None
    };

    let mut found = Vec::new();
    if let Some(length) = complete {
        found.push(Finding {
            format: candidate.format,
            stage: Stage::Carve,
            confidence: Confidence::ContiguousCarve,
            extents: Box::from([ByteRange::new(candidate.offset, length)]),
            declared_size: None,
            timestamps: argos_core::Timestamps::default(),
            deleted: None,
            name: None,
            source_object: None,
            parent: None,
        });
    }
    if let Some(thumb) = thumbnail
        && let Ok(Some(length)) = argos_carve::validate_thumbnail(view, thumb, medium_end, scratch)
    {
        found.push(Finding {
            format: Format::Jpeg,
            stage: Stage::Carve,
            confidence: Confidence::PartialOrThumbnail,
            extents: Box::from([ByteRange::new(thumb.offset, length)]),
            declared_size: None,
            timestamps: argos_core::Timestamps::default(),
            deleted: None,
            name: None,
            source_object: None,
            parent: Some(candidate.offset),
        });
    }

    Validated {
        rejected: found.is_empty(),
        findings: found,
        broken,
    }
}

/// Validates every signature hit, then recovers what did not carve whole.
///
/// Three outcomes reach `findings`: the images that carved contiguously, the
/// ones reassembly put back together, and — for the rest — the part of each
/// that decodes, because a photograph whose remainder was overwritten still
/// has a beginning on the medium.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; bundling the stage's inputs would name them twice"
)]
pub(super) fn carve<V, P>(
    config: &ScanConfig,
    control: &Control,
    views: &mut [V],
    candidates: Vec<Candidate>,
    range_end: u64,
    volumes: &[Volume],
    findings: &mut Vec<Finding>,
    progress: &P,
    report: &mut ScanReport,
) where
    V: Read + Seek + Send,
    P: ProgressSink + ?Sized,
{
    let (carved, broken) =
        validate_candidates(control, views, candidates, range_end, progress, report);
    // Kept whatever the search then does with them: locating a fragmentation
    // point is what the sweep and the validation stage cost, and a later run
    // that starts from these skips both.
    report.fragmentation.clone_from(&broken);
    progress.emit(ScanEvent::StageFinished {
        stage: Stage::Validation,
        findings: carved.len() as u64,
    });
    findings.extend(carved);

    let mut whole_again = HashSet::new();
    if config.stages().reassembly && !broken.is_empty() {
        let reassembled = reassemble_broken(
            control,
            views,
            Reassembling {
                broken: &broken,
                already_recovered: findings,
                budget: config.reassembly_budget(),
                min_long_side: config.min_long_side(),
                volumes,
                medium_len: range_end,
            },
            progress,
            report,
        );
        whole_again.extend(
            reassembled
                .iter()
                .filter_map(|finding| finding.extents.first().map(|extent| extent.start)),
        );
        progress.emit(ScanEvent::StageFinished {
            stage: Stage::Reassembly,
            findings: reassembled.len() as u64,
        });
        findings.extend(reassembled);
    }

    let partials = partial_prefixes(&broken, &whole_again, config.min_long_side());
    report.partial_prefixes = partials.len() as u64;
    findings.extend(partials);
}
