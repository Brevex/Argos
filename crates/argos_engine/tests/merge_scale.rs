//! What merging costs on the shape of medium that makes it expensive.
//!
//! The findings a real disk produces are not independent. A carve that
//! swallowed a region of the surface is one weakly evidenced finding spanning
//! thousands of files the filesystem also named — and because its evidence is
//! *weaker* than theirs, it can cover none of them. Every one of them is kept,
//! which is correct, and used to mean every one of them was compared against
//! every one kept before it.
//!
//! That is a silent stage between the sweep and the first recovered image, so
//! nothing on screen moves while it runs. A test that only checked the result
//! would have passed throughout.

use std::time::Instant;

use argos_core::geometry::{ByteOffset, ByteRange};
use argos_core::{Confidence, Format, Stage, Timestamps};
use argos_engine::Finding;

/// Findings in the pathological arrangement, for `count` inner files.
fn spanning_carve_over_named_files(count: u64) -> Vec<Finding> {
    let finding = |start: u64, len: u64, confidence: Confidence| Finding {
        format: Format::Jpeg,
        stage: Stage::Carve,
        confidence,
        extents: Box::from([ByteRange::new(ByteOffset::new(start), len)]),
        declared_size: None,
        timestamps: Timestamps::default(),
        deleted: None,
        name: None,
        source_object: None,
        parent: None,
    };

    let mut findings = vec![finding(0, count * 4096 + 4096, Confidence::ContiguousCarve)];
    for index in 0..count {
        findings.push(finding(index * 4096 + 16, 512, Confidence::FsMetadata));
    }
    findings
}

#[test]
fn merging_stays_affordable_when_one_weak_finding_spans_thousands() {
    let count = 16_000_u64;
    let findings = spanning_carve_over_named_files(count);

    let at = Instant::now();
    let merged = argos_engine::merge_for_test(findings);
    let took = at.elapsed();

    // Nothing may be lost: the span is weaker evidence than what lies inside
    // it, so it covers none of them and everything is reported.
    assert_eq!(
        merged.len() as u64,
        count + 1,
        "a container that cannot cover what it spans must not remove it"
    );

    // The ceiling is deliberately loose — this measures roughly 11 ms here, and
    // the arrangement above took 11 *seconds* before the search was ordered by
    // reach. Two seconds is far enough above the real cost to survive a slow
    // machine and far enough below the quadratic cost to fail if it returns.
    assert!(
        took.as_secs_f64() < 2.0,
        "merging {count} findings took {took:?}; the coverage search is scanning \
         everything kept so far again"
    );
}

#[test]
fn merging_scales_with_the_findings_rather_than_their_square() {
    // Doubling the input must roughly double the work. Quadratic growth
    // quadruples it, which is the difference between a stage that takes a
    // moment on a real disk and one that takes an hour.
    let time = |count: u64| {
        let findings = spanning_carve_over_named_files(count);
        let at = Instant::now();
        let merged = argos_engine::merge_for_test(findings);
        assert_eq!(merged.len() as u64, count + 1);
        at.elapsed().as_secs_f64()
    };

    // Warm the allocator so the first measurement is not the slow one.
    let _ = time(2_000);
    let small = time(4_000).max(f64::MIN_POSITIVE);
    let large = time(16_000);

    // Four times the input. Linear predicts 4x, quadratic predicts 16x; a
    // ceiling of 8 separates them with room for measurement noise.
    let growth = large / small;
    assert!(
        growth < 8.0,
        "four times the findings cost {growth:.1} times the work, which is the \
         shape of a quadratic search, not a linear one"
    );
}
