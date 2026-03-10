use proptest::prelude::*;

use argos::core::{
    calculate_entropy, categorize_dimensions, score_jpeg, score_png, ConfidenceTier, Fragment,
    FragmentKind, FragmentMap, JpegMetadata, PngMetadata, QuantizationQuality,
};
use argos::format::jpeg::{detect_jpeg_break, matches_jpeg_continuation, validate_jpeg};
use argos::format::png::{detect_png_break, matches_png_continuation, validate_png_header};
use argos::scan::scan_block;

proptest! {
    #[test]
    fn entropy_is_non_negative(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let e = calculate_entropy(&data);
        prop_assert!(e >= 0.0, "entropy must be >= 0, got {}", e);
        prop_assert!(e.is_finite(), "entropy must be finite, got {}", e);
    }

    #[test]
    fn entropy_bounded_by_eight(data in proptest::collection::vec(any::<u8>(), 1..4096)) {
        let e = calculate_entropy(&data);
        prop_assert!(e <= 8.01, "entropy must be <= 8.0, got {}", e);
    }

    #[test]
    fn entropy_uniform_byte_is_zero(byte in any::<u8>(), len in 1usize..4096) {
        let data = vec![byte; len];
        let e = calculate_entropy(&data);
        prop_assert!(e < 0.001, "uniform data should have ~0 entropy, got {}", e);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn validate_jpeg_never_panics(data in proptest::collection::vec(any::<u8>(), 0..8192)) {
        let _ = validate_jpeg(&data);
    }

    #[test]
    fn detect_jpeg_break_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..8192),
        sos_offset in 0u64..8192u64
    ) {
        let _ = detect_jpeg_break(&data, sos_offset as usize);
    }

    #[test]
    fn matches_jpeg_continuation_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = matches_jpeg_continuation(&data);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn validate_png_never_panics(data in proptest::collection::vec(any::<u8>(), 0..8192)) {
        let _ = validate_png_header(&data);
    }

    #[test]
    fn detect_png_break_never_panics(data in proptest::collection::vec(any::<u8>(), 0..8192)) {
        let _ = detect_png_break(&data);
    }

    #[test]
    fn matches_png_continuation_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = matches_png_continuation(&data);
    }
}

proptest! {
    #[test]
    fn score_jpeg_bounded(
        w in 0u16..10000,
        h in 0u16..10000,
        has_exif in any::<bool>(),
        has_icc in any::<bool>(),
        has_jfif in any::<bool>(),
        marker_count in 0u16..50,
        has_sos in any::<bool>(),
        scan_entropy in 0.0f32..8.0
    ) {
        let meta = JpegMetadata {
            has_exif,
            has_icc_profile: has_icc,
            has_jfif,
            quantization_quality: QuantizationQuality::Medium,
            marker_count,
            has_sos,
            scan_data_entropy: scan_entropy,
        };
        let score = score_jpeg(w, h, &meta);
        prop_assert!(score <= 100, "JPEG score must be <= 100, got {}", score);
    }

    #[test]
    fn score_png_bounded(
        w in 0u32..10000,
        h in 0u32..10000,
        has_text in any::<bool>(),
        has_icc in any::<bool>(),
        has_phys in any::<bool>(),
        is_screen in any::<bool>(),
        variety in 0u8..20,
        idat_count in 0usize..100
    ) {
        let meta = PngMetadata {
            has_text_chunks: has_text,
            has_icc_profile: has_icc,
            has_physical_dimensions: has_phys,
            is_screen_resolution: is_screen,
            chunk_variety: variety,
        };
        let score = score_png(w, h, &meta, idat_count);
        prop_assert!(score <= 100, "PNG score must be <= 100, got {}", score);
    }
}

proptest! {
    #[test]
    fn categorize_dimensions_never_panics(w in any::<u32>(), h in any::<u32>()) {
        let _ = categorize_dimensions(w, h);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn scan_block_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..16384),
        offset in any::<u64>()
    ) {
        let mut map = FragmentMap::new();
        scan_block(offset, &data, &mut map);
    }
}

proptest! {
    #[test]
    fn fragment_map_sort_dedup_idempotent(
        offsets in proptest::collection::vec(0u64..100_000, 0..200)
    ) {
        let mut map = FragmentMap::new();
        for off in &offsets {
            map.push(Fragment::new(*off, FragmentKind::JpegHeader, 7.0));
        }
        map.sort_by_offset();
        map.dedup();
        let len_after_first = map.jpeg_headers().len();

        map.sort_by_offset();
        map.dedup();
        let len_after_second = map.jpeg_headers().len();

        prop_assert_eq!(len_after_first, len_after_second, "sort+dedup should be idempotent");

        let offsets: Vec<u64> = map.jpeg_headers().iter().map(|f| f.offset).collect();
        for w in offsets.windows(2) {
            prop_assert!(w[0] <= w[1], "offsets must be sorted: {} > {}", w[0], w[1]);
        }
    }
}

proptest! {
    #[test]
    fn confidence_tier_covers_all_scores(score in 0u8..=255) {
        let tier = ConfidenceTier::from_score(score);
        match tier {
            ConfidenceTier::High => prop_assert!(score >= 60),
            ConfidenceTier::Partial => prop_assert!(score >= 30 && score < 60),
            ConfidenceTier::Low => prop_assert!(score < 30),
        }
    }
}
