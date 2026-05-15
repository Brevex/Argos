mod common;

use argos::carve::ImageFormat;
use argos::carve::hdd::pup::{self, Seed};
use argos::carve::hdd::sht::{self, Decision, SprtAccumulator};
use argos::carve::ssd::Scanner;
use argos::validate::{jpeg, png};
use proptest::prelude::*;

use common::{
    JPEG_EOI, JPEG_SOI, MARKER_DHT, MARKER_DQT, MARKER_SOF0, MARKER_SOS, PNG_SIGNATURE,
    baseline_jpeg_with_nonzero_huffman_selectors, baseline_jpeg_with_stuffed_entropy,
    minimal_baseline_jpeg, multi_block_baseline_jpeg, png_chunk, progressive_jpeg, segment,
    single_symbol_dht, valid_png,
};

const BLOCK_SIZE: usize = 4096;

fn bytes_for_candidate<'a>(data: &'a [u8], candidate: &argos::carve::Candidate) -> &'a [u8] {
    let start = candidate.offset as usize;
    let end = start + candidate.length.expect("length") as usize;
    &data[start..end]
}

fn scan_full(data: &[u8]) -> Vec<argos::carve::Candidate> {
    let mut scanner = Scanner::new().expect("scanner");
    scanner.scan_block(data).expect("scan")
}

fn scan_split(data: &[u8], boundary: usize) -> Vec<argos::carve::Candidate> {
    let mut scanner = Scanner::new().expect("scanner");
    let mut out = Vec::new();
    let (a, b) = data.split_at(boundary);
    out.extend(scanner.scan_block(a).expect("first"));
    out.extend(scanner.scan_block(b).expect("second"));
    out
}

#[test]
fn aho_corasick_locates_synthesized_jpeg_at_known_offset() {
    let mut buffer = vec![0xABu8; 200];
    let payload = [0xFF, 0xD8, 0xAA, 0xBB, 0xFF, 0xD9];
    buffer[50..56].copy_from_slice(&payload);
    let cands = scan_full(&buffer);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].offset, 50);
    assert_eq!(cands[0].length, Some(6));
    assert_eq!(cands[0].format, ImageFormat::Jpeg);
}

#[test]
fn aho_corasick_locates_synthesized_png_at_known_offset() {
    let png = valid_png();
    let mut buffer = vec![0u8; 256];
    buffer[64..64 + png.len()].copy_from_slice(&png);
    let cands = scan_full(&buffer);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].offset, 64);
    assert_eq!(cands[0].length, Some(png.len() as u64));
    assert_eq!(cands[0].format, ImageFormat::Png);
}

#[test]
fn aho_corasick_block_boundary_invariance_jpeg() {
    let payload = [
        0xABu8, 0xAB, 0xFF, 0xD8, 0xAA, 0xBB, 0xCC, 0xFF, 0xD9, 0xAB, 0xAB,
    ];
    let reference = scan_full(&payload);
    assert_eq!(reference.len(), 1);

    for boundary in 1..payload.len() {
        let split = scan_split(&payload, boundary);
        assert_eq!(
            split.len(),
            reference.len(),
            "split at {boundary} produced different match count"
        );
        for (a, b) in reference.iter().zip(split.iter()) {
            assert_eq!(a.offset, b.offset, "split at {boundary}: offset differs");
            assert_eq!(a.length, b.length, "split at {boundary}: length differs");
            assert_eq!(a.format, b.format, "split at {boundary}: format differs");
        }
    }
}

#[test]
fn aho_corasick_block_boundary_invariance_png_signature() {
    let png = valid_png();
    let mut payload = vec![0u8; 10 + png.len() + 5];
    payload[10..10 + png.len()].copy_from_slice(&png);

    let reference = scan_full(&payload);
    assert_eq!(reference.len(), 1);

    for boundary in 10..18 {
        let split = scan_split(&payload, boundary);
        assert_eq!(split.len(), 1, "split at {boundary}");
        assert_eq!(split[0].offset, reference[0].offset);
        assert_eq!(split[0].length, reference[0].length);
    }
}

#[test]
fn aho_corasick_emits_multiple_disjoint_jpegs() {
    let payload = [0xFF, 0xD8, 0x01, 0xFF, 0xD9, 0xFF, 0xD8, 0x02, 0xFF, 0xD9];
    let cands = scan_full(&payload);
    assert_eq!(cands.len(), 2);
    assert_eq!(cands[0].offset, 0);
    assert_eq!(cands[1].offset, 5);
}

#[test]
fn aho_corasick_jpeg_candidate_bytes_validate() {
    let jpeg = minimal_baseline_jpeg();
    let mut data = vec![0xABu8; 128];
    let offset = 40usize;
    data.splice(offset..offset, jpeg.clone());

    let cands = scan_full(&data);
    assert_eq!(cands.len(), 1);
    let recovered = bytes_for_candidate(&data, &cands[0]);
    let score = jpeg::validate(recovered).expect("validate");
    assert_eq!(score, 1.0);
    assert_eq!(recovered, jpeg.as_slice());
}

#[test]
fn aho_corasick_png_candidate_bytes_validate() {
    let png = valid_png();
    let mut data = vec![0xABu8; 128];
    let offset = 40usize;
    data.splice(offset..offset, png.clone());

    let cands = scan_full(&data);
    assert_eq!(cands.len(), 1);
    let recovered = bytes_for_candidate(&data, &cands[0]);
    let score = png::validate(recovered).expect("validate");
    assert_eq!(score, 1.0);
    assert_eq!(recovered, png.as_slice());
}

#[test]
fn aho_corasick_png_candidate_includes_complete_iend_chunk() {
    let png = valid_png();
    let cands = scan_full(&png);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].offset, 0);
    assert_eq!(cands[0].length, Some(png.len() as u64));
}

#[test]
fn aho_corasick_boundary_invariance_for_complete_png() {
    let png = valid_png();
    let reference = scan_full(&png);
    assert_eq!(reference.len(), 1);

    for boundary in 1..png.len() {
        let split = scan_split(&png, boundary);
        assert_eq!(split.len(), reference.len(), "split at {boundary}");
        assert_eq!(split[0].offset, reference[0].offset);
        assert_eq!(split[0].length, reference[0].length);
        assert_eq!(split[0].format, reference[0].format);
    }
}

#[test]
fn aho_corasick_nested_headers_do_not_emit_overlapping_artifacts() {
    let payload = [
        0xFF, 0xD8, 0x44, 0x55, 0xFF, 0xD8, 0x66, 0x77, 0xFF, 0xD9, 0x88, 0xFF, 0xD9,
    ];
    let cands = scan_full(&payload);
    let mut ranges = Vec::with_capacity(cands.len());
    for cand in cands {
        let start = cand.offset;
        let end = start + cand.length.expect("length");
        ranges.push((start, end));
    }
    for a in 0..ranges.len() {
        for b in a + 1..ranges.len() {
            assert!(
                ranges[a].1 <= ranges[b].0 || ranges[b].1 <= ranges[a].0,
                "overlapping candidates: {:?}",
                ranges
            );
        }
    }
}

#[test]
fn aho_corasick_ignores_orphan_footer() {
    let payload = [0u8, 0u8, 0xFF, 0xD9, 0u8];
    let cands = scan_full(&payload);
    assert!(cands.is_empty());
}

proptest! {
    #[test]
    fn aho_corasick_never_panics_on_arbitrary_input(data: Vec<u8>) {
        let _ = scan_full(&data);
    }
}

#[test]
fn sprt_decision_thresholds_match_closed_form_for_default_alpha_beta() {
    let expected_a = ((1.0 - sht::BETA) / sht::ALPHA).ln();
    let expected_b = (sht::BETA / (1.0 - sht::ALPHA)).ln();
    assert!((sht::A - expected_a).abs() < 1e-9);
    assert!((sht::B - expected_b).abs() < 1e-9);
}

#[test]
fn sprt_accepts_h1_when_statistic_exceeds_upper_bound() {
    let mut acc = SprtAccumulator::new();
    acc.update(sht::A + 0.01);
    assert_eq!(acc.decision(), Decision::H1);
}

#[test]
fn sprt_accepts_h0_when_statistic_falls_below_lower_bound() {
    let mut acc = SprtAccumulator::new();
    acc.update(sht::B - 0.01);
    assert_eq!(acc.decision(), Decision::H0);
}

#[test]
fn sprt_continues_inside_open_interval() {
    let mut acc = SprtAccumulator::new();
    acc.update((sht::A + sht::B) / 2.0);
    assert_eq!(acc.decision(), Decision::Continue);
}

#[test]
fn sprt_accumulates_observations_until_decision_threshold_reached() {
    let mut acc = SprtAccumulator::new();
    let step = sht::A / 4.0;
    for _ in 0..3 {
        acc.update(step);
        assert_eq!(acc.decision(), Decision::Continue);
    }
    acc.update(step);
    acc.update(step);
    assert_eq!(acc.decision(), Decision::H1);
}

#[test]
fn pup_emits_at_least_one_candidate_for_a_single_seed_with_continuation() {
    let mut data = vec![0xABu8; BLOCK_SIZE * 4];
    let jpeg = minimal_baseline_jpeg();
    let seed_block = 1usize;
    data[seed_block * BLOCK_SIZE..seed_block * BLOCK_SIZE + jpeg.len()].copy_from_slice(&jpeg);

    let seeds = vec![Seed {
        block_index: seed_block as u64,
        format: ImageFormat::Jpeg,
    }];
    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    assert!(!cands.is_empty(), "PUP must emit at least one candidate");
    assert!(cands.iter().all(|c| c.length.is_some()));
    assert_eq!(cands[0].offset, (seed_block * BLOCK_SIZE) as u64);
    assert_eq!(cands[0].format, ImageFormat::Jpeg);
}

#[test]
fn pup_preserves_block_uniqueness_across_two_seeds() {
    let mut data = vec![0xABu8; BLOCK_SIZE * 8];
    let jpeg = minimal_baseline_jpeg();
    for &seed_block in &[1usize, 5usize] {
        let start = seed_block * BLOCK_SIZE;
        data[start..start + jpeg.len()].copy_from_slice(&jpeg);
    }
    let seeds = vec![
        Seed {
            block_index: 1,
            format: ImageFormat::Jpeg,
        },
        Seed {
            block_index: 5,
            format: ImageFormat::Jpeg,
        },
    ];
    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);

    let mut occupied_blocks = std::collections::HashSet::new();
    for cand in &cands {
        let start = cand.offset / BLOCK_SIZE as u64;
        let span = cand.length.expect("length") / BLOCK_SIZE as u64;
        for b in start..start + span.max(1) {
            assert!(
                occupied_blocks.insert(b),
                "block {b} was claimed by more than one candidate"
            );
        }
    }
}

#[test]
fn pup_is_deterministic_for_the_same_input() {
    let mut data = vec![0xABu8; BLOCK_SIZE * 4];
    data[BLOCK_SIZE..BLOCK_SIZE + minimal_baseline_jpeg().len()]
        .copy_from_slice(&minimal_baseline_jpeg());
    let seeds = vec![Seed {
        block_index: 1,
        format: ImageFormat::Jpeg,
    }];

    let first = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    let second = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);

    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.offset, b.offset);
        assert_eq!(a.length, b.length);
        assert_eq!(a.format, b.format);
    }
}

#[test]
fn pup_terminates_within_max_blocks_bound() {
    let data = vec![0xCDu8; BLOCK_SIZE * 32];
    let seeds = vec![Seed {
        block_index: 0,
        format: ImageFormat::Jpeg,
    }];
    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 5);
    for cand in &cands {
        let span = cand.length.expect("length") / BLOCK_SIZE as u64;
        assert!(span <= 5, "PUP exceeded max_blocks bound");
    }
}

#[test]
fn pup_reconstructs_contiguous_multiblock_jpeg_until_footer_block() {
    let jpeg = multi_block_baseline_jpeg(BLOCK_SIZE, 3);
    let mut data = vec![0x00u8; BLOCK_SIZE * 8];
    let seed_block = 1usize;
    let start = seed_block * BLOCK_SIZE;
    data[start..start + jpeg.len()].copy_from_slice(&jpeg);
    let seeds = vec![Seed {
        block_index: seed_block as u64,
        format: ImageFormat::Jpeg,
    }];

    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    let recovered = cands
        .iter()
        .find(|cand| cand.offset == start as u64 && cand.length == Some(jpeg.len() as u64))
        .expect("contiguous multi-block candidate");
    let score = jpeg::validate(bytes_for_candidate(&data, recovered)).expect("validate");
    assert_eq!(score, 1.0);
}

#[test]
fn pup_fragmented_jpeg_does_not_claim_gap_block_as_content() {
    let jpeg = multi_block_baseline_jpeg(BLOCK_SIZE, 2);
    let first = &jpeg[..BLOCK_SIZE];
    let second = &jpeg[BLOCK_SIZE..];
    let mut data = vec![0x00u8; BLOCK_SIZE * 6];
    data[BLOCK_SIZE..BLOCK_SIZE + first.len()].copy_from_slice(first);
    data[BLOCK_SIZE * 3..BLOCK_SIZE * 3 + second.len()].copy_from_slice(second);
    let seeds = vec![Seed {
        block_index: 1,
        format: ImageFormat::Jpeg,
    }];

    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    for cand in &cands {
        let start = cand.offset / BLOCK_SIZE as u64;
        let span = cand.length.expect("length") / BLOCK_SIZE as u64;
        assert!(
            !(start <= 2 && 2 < start + span),
            "gap block was claimed by candidate {cand:?}"
        );
    }
}

#[test]
fn pup_zero_padding_terminates_before_padding_run() {
    let jpeg = minimal_baseline_jpeg();
    let mut data = vec![0x00u8; BLOCK_SIZE * 5];
    data[0..jpeg.len()].copy_from_slice(&jpeg);
    let seeds = vec![Seed {
        block_index: 0,
        format: ImageFormat::Jpeg,
    }];

    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    assert!(
        cands
            .iter()
            .any(|cand| cand.length == Some(jpeg.len() as u64))
    );
}

#[test]
fn pup_entropy_like_blocks_do_not_end_candidate_before_footer() {
    let jpeg = multi_block_baseline_jpeg(BLOCK_SIZE, 4);
    let mut data = vec![0x00u8; BLOCK_SIZE * 6];
    data[0..jpeg.len()].copy_from_slice(&jpeg);
    let seeds = vec![Seed {
        block_index: 0,
        format: ImageFormat::Jpeg,
    }];

    let cands = pup::run(&seeds, &data, BLOCK_SIZE, 10_000);
    assert!(
        cands
            .iter()
            .any(|cand| cand.length.expect("length") >= jpeg.len() as u64),
        "candidate ended before the JPEG footer block"
    );
}

#[test]
fn pup_empty_seed_set_produces_no_candidates() {
    let data = vec![0u8; BLOCK_SIZE * 4];
    let cands = pup::run(&[], &data, BLOCK_SIZE, 10_000);
    assert!(cands.is_empty());
}

#[test]
fn jpeg_validate_returns_one_for_canonical_baseline() {
    let score = jpeg::validate(&minimal_baseline_jpeg()).expect("validate");
    assert_eq!(score, 1.0);
}

#[test]
fn jpeg_validate_rejects_pure_garbage() {
    let score = jpeg::validate(&[0u8; 1024]).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn jpeg_validate_returns_zero_when_dqt_is_missing() {
    let mut data = Vec::new();
    data.extend_from_slice(&JPEG_SOI);
    data.extend_from_slice(&segment(MARKER_DHT, &single_symbol_dht(0)));
    data.extend_from_slice(&segment(
        MARKER_SOF0,
        &[0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00],
    ));
    data.extend_from_slice(&segment(MARKER_SOS, &[0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]));
    data.extend_from_slice(&JPEG_EOI);

    let score = jpeg::validate(&data).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn jpeg_validate_marks_progressive_with_partial_score() {
    let score = jpeg::validate(&progressive_jpeg()).expect("validate");
    assert_eq!(score, 0.5);
}

#[test]
fn jpeg_validate_accepts_byte_stuffed_entropy_before_eoi() {
    let score = jpeg::validate(&baseline_jpeg_with_stuffed_entropy()).expect("validate");
    assert_eq!(score, 1.0);
}

#[test]
fn jpeg_validate_honors_nonzero_huffman_selectors_from_sos() {
    let score = jpeg::validate(&baseline_jpeg_with_nonzero_huffman_selectors()).expect("validate");
    assert_eq!(score, 1.0);
}

#[test]
fn jpeg_validate_returns_zero_for_soi_without_eoi() {
    let mut data = Vec::new();
    data.extend_from_slice(&JPEG_SOI);
    data.extend_from_slice(&segment(MARKER_DQT, &common::baseline_dqt()));
    let score = jpeg::validate(&data).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn jpeg_validate_is_deterministic() {
    let data = minimal_baseline_jpeg();
    let a = jpeg::validate(&data).expect("a");
    let b = jpeg::validate(&data).expect("b");
    assert_eq!(a, b);
}

#[test]
fn jpeg_continuation_score_signals_padding_as_low() {
    assert!(jpeg::continuation_score(&[0u8; 1024]) <= 0.2);
}

#[test]
fn jpeg_continuation_score_signals_eoi_marker_as_low() {
    let mut block = vec![0x7Bu8; 512];
    block[200] = 0xFF;
    block[201] = 0xD9;
    assert!(jpeg::continuation_score(&block) <= 0.3);
}

#[test]
fn jpeg_continuation_score_signals_restart_marker_as_low() {
    let mut block = vec![0x7Bu8; 512];
    block[300] = 0xFF;
    block[301] = 0xD3;
    assert!(jpeg::continuation_score(&block) <= 0.3);
}

#[test]
fn jpeg_continuation_score_signals_dense_entropy_as_high() {
    let block: Vec<u8> = (0..=255).cycle().take(2048).collect();
    assert!(jpeg::continuation_score(&block) >= 0.5);
}

proptest! {
    #[test]
    fn jpeg_validate_never_panics(data: Vec<u8>) {
        let _ = jpeg::validate(&data);
    }

    #[test]
    fn jpeg_validate_never_panics_on_soi_eoi_envelope(payload: Vec<u8>) {
        let mut buf = Vec::with_capacity(payload.len() + 4);
        buf.extend_from_slice(&JPEG_SOI);
        buf.extend_from_slice(&payload);
        buf.extend_from_slice(&JPEG_EOI);
        let _ = jpeg::validate(&buf);
    }

    #[test]
    fn jpeg_continuation_score_never_panics(block: Vec<u8>) {
        let _ = jpeg::continuation_score(&block);
    }
}

#[test]
fn png_validate_accepts_canonical_valid_png() {
    let score = png::validate(&valid_png()).expect("validate");
    assert_eq!(score, 1.0);
}

#[test]
fn png_validate_rejects_garbage() {
    let score = png::validate(&[0u8; 1024]).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn png_validate_returns_partial_score_on_corrupt_crc() {
    let mut data = valid_png();
    let iend_crc_start = data.len() - 4;
    data[iend_crc_start] ^= 0xFF;
    let score = png::validate(&data).expect("validate");
    assert!(score < 1.0);
    assert!(score > 0.0);
}

#[test]
fn png_validate_rejects_missing_iend() {
    let mut data = valid_png();
    data.truncate(data.len() - 12);
    let score = png::validate(&data).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn png_validate_rejects_iend_tag_without_crc() {
    let mut data = valid_png();
    data.truncate(data.len() - 4);
    let score = png::validate(&data).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn png_validate_rejects_chunk_declaring_length_beyond_input() {
    let mut data = Vec::new();
    data.extend_from_slice(&PNG_SIGNATURE);
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    data.extend_from_slice(b"IDAT");
    data.extend_from_slice(&[0u8; 8]);
    let score = png::validate(&data).expect("validate");
    assert_eq!(score, 0.0);
}

#[test]
fn png_continuation_score_completes_chunk_with_valid_crc() {
    let mut partial = png::PartialChunk::default();
    let body = [0u8; 13];
    let chunk = png_chunk(b"IHDR", &body);
    let score = png::continuation_score(&mut partial, &chunk);
    assert_eq!(score, 1.0);
}

#[test]
fn png_continuation_score_rejects_chunk_with_bad_crc() {
    let mut partial = png::PartialChunk::default();
    let body = [0u8; 13];
    let mut chunk = png_chunk(b"IHDR", &body);
    let crc_pos = chunk.len() - 1;
    chunk[crc_pos] ^= 0xFF;
    let score = png::continuation_score(&mut partial, &chunk);
    assert_eq!(score, 0.0);
}

proptest! {
    #[test]
    fn png_validate_never_panics(data: Vec<u8>) {
        let _ = png::validate(&data);
    }

    #[test]
    fn png_validate_never_panics_on_signature_envelope(payload: Vec<u8>) {
        let mut buf = Vec::with_capacity(payload.len() + 8);
        buf.extend_from_slice(&PNG_SIGNATURE);
        buf.extend_from_slice(&payload);
        let _ = png::validate(&buf);
    }
}
