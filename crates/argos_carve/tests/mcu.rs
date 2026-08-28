//! The entropy decoder as an oracle: what it decodes, and that it still agrees
//! with what it decoded before.
//!
//! Every reassembly answer rests on this decoder, so both halves matter — that
//! it says "complete" for exactly the real image and nothing else, and that a
//! change making it faster did not quietly make it answer differently.

/// The entropy decoder is the oracle every reassembly answer rests on, so what
/// matters is that it says "complete" for exactly the real image and for
/// nothing else.
mod decodes {
    use std::io::Cursor;

    use argos_carve::Scratch;
    use argos_carve::fixture::{Disk, Jpeg, photo_jpeg, truncated};
    use argos_carve::mcu::{self, ScanStop};
    use argos_core::ByteOffset;

    fn scan_of(bytes: &[u8]) -> mcu::ScanOutcome {
        let mut src = Cursor::new(bytes);
        let mut scratch = Scratch::new();
        mcu::scan(
            &mut src,
            ByteOffset::new(0),
            bytes.len() as u64,
            &mut scratch,
        )
        .expect("in-memory read")
    }

    #[test]
    fn a_real_photograph_decodes_every_mcu_it_declares() {
        let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);

        let outcome = scan_of(&image);

        assert!(outcome.is_complete(), "got {outcome:?}");
        assert_eq!(outcome.stop, ScanStop::Complete);
        assert_eq!(outcome.mcus_decoded, outcome.mcus_required);
        // 320x240 of one component: 40 by 30 blocks.
        assert_eq!(outcome.mcus_across, 40);
        assert_eq!(outcome.mcus_required, 40 * 30);
        assert_eq!(outcome.end.get(), image.len() as u64);
    }

    #[test]
    fn a_colour_photograph_with_subsampling_decodes_completely() {
        let image = argos_carve::fixture::photo_jpeg_rgb(160, 128, 0x0F1E_2D3C_4B5A_6978);

        let outcome = scan_of(&image);

        assert!(outcome.is_complete(), "got {outcome:?}");
        assert!(outcome.mcus_required > 0);
    }

    #[test]
    fn a_truncated_photograph_is_never_complete_and_reports_how_far_it_got() {
        let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
        let cut = truncated(&image, image.len() / 2);

        let outcome = scan_of(&cut);

        assert!(!outcome.is_complete());
        assert_eq!(outcome.stop, ScanStop::Broke);
        assert!(
            outcome.mcus_decoded > 0 && outcome.mcus_decoded < outcome.mcus_required,
            "half an image should decode part of its MCUs, got {outcome:?}"
        );
    }

    #[test]
    fn a_photograph_spliced_onto_unrelated_data_is_never_complete() {
        // This is the case the whole oracle exists for: a header followed by bytes
        // that are not its continuation. The marker grammar accepts such splices
        // routinely; the entropy decoder must not.
        let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
        let noise = Disk::noisy(64 * 1024, 0x5DEE_CE66_D000_0001).into_bytes();
        let mut spliced = image[..image.len() / 2].to_vec();
        spliced.extend_from_slice(&noise);

        let outcome = scan_of(&spliced);

        assert!(
            !outcome.is_complete(),
            "a splice onto noise must never read as a whole image, got {outcome:?}"
        );
        // And the noise must not inflate the progress the search ranks by: the
        // decoder gives up within a fraction of an MCU row of the splice.
        let truth = scan_of(&image);
        assert!(
            outcome.mcus_decoded < truth.mcus_required,
            "spliced progress {} must stay below the real MCU count {}",
            outcome.mcus_decoded,
            truth.mcus_required
        );
    }

    #[test]
    fn noise_alone_decodes_almost_nothing() {
        // Progress has to be ungameable, because the graph walk ranks candidates
        // by it. A block of unrelated data must not look like a long stretch of
        // successfully decoded image.
        let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
        let noise = Disk::noisy(256 * 1024, 0x1357_9BDF_2468_ACE0).into_bytes();
        // Keep the frame headers so the decoder has tables and geometry, then feed
        // it nothing but noise where the scan should be.
        let scan_start = image
            .windows(2)
            .position(|pair| pair == [0xFF, 0xDA])
            .expect("the fixture has a scan");
        let mut headers_then_noise = image[..scan_start].to_vec();
        // Re-emit the SOS segment itself, then noise instead of entropy data.
        let sos_len = usize::from(u16::from_be_bytes([
            image[scan_start + 2],
            image[scan_start + 3],
        ]));
        headers_then_noise.extend_from_slice(&image[scan_start..scan_start + 2 + sos_len]);
        headers_then_noise.extend_from_slice(&noise);

        let outcome = scan_of(&headers_then_noise);

        assert!(!outcome.is_complete());
        // A whole 256 KiB of noise must not decode more MCUs than the real,
        // far smaller, scan does.
        let truth = scan_of(&image);
        assert!(
            outcome.mcus_decoded < truth.mcus_required,
            "noise decoded {} MCUs against a real {}",
            outcome.mcus_decoded,
            truth.mcus_required
        );
    }

    #[test]
    fn restart_markers_out_of_order_break_the_scan() {
        let image = photo_jpeg(256, 128, 0x2468_ACE0_1357_9BDF);
        let outcome = scan_of(&image);
        assert!(outcome.is_complete(), "baseline fixture must decode");

        // The synthetic builder emits cyclic RSTn; scrambling one must be caught.
        let with_restarts = Jpeg::new().with_restart_interval(4).build();
        let broken = argos_carve::fixture::with_flipped_byte(
            &with_restarts,
            with_restarts
                .windows(2)
                .position(|pair| pair[0] == 0xFF && (0xD0..=0xD7).contains(&pair[1]))
                .map(|at| at + 1)
                .expect("the fixture has restart markers"),
        );
        assert!(!scan_of(&broken).is_complete());
    }

    #[test]
    fn a_progressive_frame_is_reported_unsupported_not_guessed_at() {
        // SOF2 is a progressive frame. The decoder must say so rather than fail it
        // as corrupt: "we cannot check this" and "this is not an image" are
        // different answers, and only one of them is honest here.
        let image = photo_jpeg(64, 64, 0xDEAD_BEEF_CAFE_F00D);
        let sof = image
            .windows(2)
            .position(|pair| pair == [0xFF, 0xC0])
            .expect("the fixture has a baseline frame header");
        let progressive = argos_carve::fixture::with_flipped_byte(&image, sof + 1);
        // Flipping produces some other marker; force it to SOF2 explicitly.
        let mut progressive = progressive;
        progressive[sof + 1] = 0xC2;

        assert_eq!(scan_of(&progressive).stop, ScanStop::Unsupported);
    }

    #[test]
    fn an_mcu_index_maps_to_the_pixel_row_it_starts() {
        let image = photo_jpeg(320, 240, 0xA1B2_C3D4_E5F6_0718);
        let outcome = scan_of(&image);

        assert_eq!(outcome.pixel_row_of(0), 0);
        // One full MCU row in: 40 MCUs across, 8 pixel rows tall.
        assert_eq!(outcome.pixel_row_of(outcome.mcus_across), 8);
        assert_eq!(outcome.pixel_row_of(outcome.mcus_across * 3), 24);
    }

    #[test]
    fn arbitrary_bytes_never_panic_and_never_claim_completeness() {
        let mut disk = Disk::noisy(32 * 1024, 0xFEED_FACE_BEEF_0001).into_bytes();
        // Plant a bare SOI so the parser actually enters the header loop.
        disk[0] = 0xFF;
        disk[1] = 0xD8;

        let outcome = scan_of(&disk);

        assert!(!outcome.is_complete());
    }
}

/// The entropy decoder's observable behaviour, frozen.
///
/// The reassembly oracle decides what counts as evidence, so a change that
/// makes it faster and also makes it answer differently has not optimised it —
/// it has replaced it. "The measured recovery rate did not move" does not rule
/// that out: the rate is a sample over six fragmentation patterns, and a
/// decoder can shift on inputs no pattern happens to pose.
///
/// What rules it out is **observational equivalence**: for every input, the
/// same output. This suite fixes that by recording the whole [`ScanOutcome`] —
/// every field, not a chosen one — over a corpus that is deterministic, and
/// comparing against a golden file. Any divergence, on any input, on any field,
/// fails.
///
/// The corpus covers what a hypothesis actually meets on a medium: whole
/// photographs, frames cut mid-scan, single flipped bytes, one photograph
/// spliced onto another, and pure noise behind a real header — that last one
/// being the case `docs/defects/07` measured at 580 µs, where the codes stay
/// legal and the decoder walks on through nonsense.
///
/// It exercises both entry points that matter: `scan_watching`, which a
/// validation pass uses, and `resume_at` + `scan_resumed`, which is the path
/// every reassembly hypothesis takes and therefore the one being optimised.
///
/// # Re-recording
///
/// `UPDATE_GOLDEN=1 cargo test -p argos_carve --test mcu`
///
/// Only ever with a deliberate, reviewed change to what the decoder reports.
/// Re-recording to make a red suite green is discarding the only evidence that
/// an optimisation preserved the answer.
mod equivalence {
    use std::fmt::Write as _;
    use std::io::Cursor;
    use std::path::Path;

    use argos_carve::Scratch;
    use argos_carve::fixture::{
        Disk, photo_jpeg, photo_jpeg_quality, photo_jpeg_rgb, photo_jpeg_with_restarts, truncated,
        with_flipped_byte,
    };
    use argos_carve::mcu::{self, ScanOutcome};
    use argos_core::ByteOffset;

    /// One corpus entry: a name that says what it poses, and the bytes.
    struct Case {
        name: String,
        bytes: Vec<u8>,
    }

    fn case(name: impl Into<String>, bytes: Vec<u8>) -> Case {
        Case {
            name: name.into(),
            bytes,
        }
    }

    /// Every input the decoder is held to, built deterministically from seeds.
    fn corpus() -> Vec<Case> {
        // Whole photographs across the geometries the decoder distinguishes:
        // single-component, interleaved colour with 2x2 subsampling, and a frame
        // written with restart markers.
        let mut cases = vec![
            case("luma-640x480", photo_jpeg(640, 480, 0x1111_0000_0000_0001)),
            case("luma-97x61-odd", photo_jpeg(97, 61, 0x1111_0000_0000_0002)),
            case("rgb-96x64", photo_jpeg_rgb(96, 64, 0x1111_0000_0000_0003)),
            case(
                "restarts-320x240",
                photo_jpeg_with_restarts(320, 240, 0x1111_0000_0000_0004, 2),
            ),
            case(
                "quality-30",
                photo_jpeg_quality(160, 120, 0x1111_0000_0000_0005, 30),
            ),
            case(
                "quality-98",
                photo_jpeg_quality(160, 120, 0x1111_0000_0000_0006, 98),
            ),
        ];

        // Cut mid-scan at several depths: the decoder must stop where it stopped,
        // and say how far it got, identically.
        let full = photo_jpeg(256, 192, 0x2222_0000_0000_0001);
        for numerator in [1_usize, 2, 3, 7] {
            let keep = full.len() * numerator / 8;
            cases.push(case(
                format!("truncated-{numerator}-8"),
                truncated(&full, keep),
            ));
        }

        // One flipped byte inside the entropy data. The stream stays legal for a
        // while and then does not, which is the boundary this suite exists to pin.
        for percent in [55_usize, 70, 85] {
            let at = full.len() * percent / 100;
            cases.push(case(
                format!("flipped-at-{percent}pc"),
                with_flipped_byte(&full, at),
            ));
        }

        // One photograph's header over another's data: legal codes, nonsense
        // pictures, and the case that costs 580 us (`docs/defects/07`).
        let other = photo_jpeg(256, 192, 0x2222_0000_0000_0002);
        let head = full.len() / 3;
        let mut spliced = full[..head].to_vec();
        spliced.extend_from_slice(&other[head..]);
        cases.push(case("spliced-onto-a-sibling", spliced));

        // A real header followed by noise, which is what most hypotheses are.
        let mut noisy = full[..head].to_vec();
        noisy.extend_from_slice(&Disk::noisy(1 << 15, 0x3333_0000_0000_0001).into_bytes());
        cases.push(case("header-then-noise", noisy));

        // Not a JPEG at all, and the empty input.
        cases.push(case(
            "pure-noise",
            Disk::noisy(1 << 12, 0x3333_0000_0000_0002).into_bytes(),
        ));
        cases.push(case("empty", Vec::new()));

        cases
    }

    /// Formats an outcome with every field named, so a diff says which one moved.
    fn render(outcome: &ScanOutcome) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "decoded={} required={} across={} rows={} {}x{} end={} settled={} stop={:?} seams={}",
            outcome.mcus_decoded,
            outcome.mcus_required,
            outcome.mcus_across,
            outcome.mcu_rows,
            outcome.width,
            outcome.height,
            outcome.end.get(),
            outcome.settled.get(),
            outcome.stop,
            outcome.seams,
        );
        for mcu in &outcome.seam_mcus[..outcome.seams] {
            let _ = write!(out, " {mcu}");
        }
        out
    }

    /// Runs every entry point over every case and renders the whole result.
    fn observe() -> String {
        let mut out = String::new();
        let mut scratch = Scratch::new();

        for case in corpus() {
            let limit = case.bytes.len() as u64;
            let start = ByteOffset::new(0);

            // Plain scan.
            let plain = mcu::scan(&mut Cursor::new(&case.bytes), start, limit, &mut scratch)
                .expect("an in-memory read cannot fail");
            let _ = writeln!(out, "{}\tscan\t{}", case.name, render(&plain));

            // Watched scan, with splice offsets a reassembly would ask about.
            let watch: Vec<u64> = [4_u64, 3, 2]
                .iter()
                .map(|d| limit / d)
                .filter(|at| *at > 0)
                .collect();
            let watched = mcu::scan_watching(
                &mut Cursor::new(&case.bytes),
                start,
                limit,
                &watch,
                &mut scratch,
            )
            .expect("an in-memory read cannot fail");
            let _ = writeln!(out, "{}\twatched\t{}", case.name, render(&watched));

            // The hypothesis path: resume partway, then continue. This is what the
            // reassembly search runs hundreds of thousands of times.
            for divisor in [4_u64, 2] {
                let until = limit / divisor;
                let resumed =
                    mcu::resume_at(&mut Cursor::new(&case.bytes), start, until, &mut scratch)
                        .expect("an in-memory read cannot fail");
                match resumed {
                    None => {
                        let _ = writeln!(out, "{}\tresume/{divisor}\tnone", case.name);
                    }
                    Some(resume) => {
                        let _ = writeln!(
                            out,
                            "{}\tresume/{divisor}\treplay_from={} decoded={} required={} {:?}",
                            case.name,
                            resume.replay_from().get(),
                            resume.mcus_decoded(),
                            resume.mcus_required(),
                            resume.dimensions(),
                        );
                        let continued = mcu::scan_resumed(
                            &resume,
                            &mut Cursor::new(&case.bytes),
                            limit,
                            &watch,
                            &mut scratch,
                        )
                        .expect("an in-memory read cannot fail");
                        let _ = writeln!(
                            out,
                            "{}\tcontinued/{divisor}\t{}",
                            case.name,
                            render(&continued)
                        );
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_entropy_decoder_reports_exactly_what_it_reported_before() {
        let observed = observe();
        let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("oracle_equivalence.golden");

        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &observed).expect("recording the golden file");
            return;
        }

        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|error| {
            panic!(
                "{} is missing ({error}). Record it with UPDATE_GOLDEN=1, but only from a decoder \
                 already known good — a golden file recorded from a change is a change that proves \
                 itself.",
                golden.display()
            )
        });

        if observed == expected {
            return;
        }
        let mut divergences = String::new();
        for (line, (was, now)) in expected.lines().zip(observed.lines()).enumerate() {
            if was != now {
                let _ = write!(
                    divergences,
                    "\n  line {}:\n    was {was}\n    now {now}",
                    line + 1
                );
            }
        }
        if expected.lines().count() != observed.lines().count() {
            let _ = write!(
                divergences,
                "\n  the corpus itself changed: {} lines recorded, {} observed",
                expected.lines().count(),
                observed.lines().count()
            );
        }
        panic!(
            "the entropy decoder now answers differently. An optimisation that changes what the \
             oracle reports has replaced it, not sped it up.{divergences}"
        );
    }
}
