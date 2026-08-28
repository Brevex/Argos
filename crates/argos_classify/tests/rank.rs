//! Ordering recovered artifacts by the evidence they carry.
//!
//! The shapes here are the ones measured on a 1 TB disk of ten years' use
//! (`docs/defects/02-thumbnail-provenance.md`, `03-triage-miscalibrated.md`):
//! a thumbnail cache of hundreds of entries at one size to the pixel, and a
//! few hundred camera frames among them.

use argos_classify::rank::{Evidence, Standing, standing};
use argos_core::ports::Capture;

fn camera(taken: &str) -> Capture {
    Capture {
        make: Some("samsung".to_owned()),
        model: Some("SM-G610M".to_owned()),
        taken: Some(taken.to_owned()),
        modified: None,
        pixels: None,
    }
}

#[test]
fn a_cache_entry_never_outranks_a_photograph_even_carrying_its_camera() {
    // The case that produced the wrong conclusion on the real disk: a cache
    // keeps the metadata of the picture it previews, so a 258x258 entry can
    // name a camera and a capture date exactly as the original does. What
    // separates them is not the metadata; it is that the entry sits in a run
    // of hundreds of its own size.
    let entry =
        Evidence::measured(Some((258, 258)), &camera("2016:03:04 11:00:00")).among_neighbours(51);
    let original = Evidence::measured(Some((4128, 3096)), &camera("2016:03:04 11:00:00"));

    assert_eq!(standing(&entry), Standing::CacheNeighbour);
    assert_eq!(standing(&original), Standing::CameraNamed);
    assert!(
        standing(&entry) < standing(&original),
        "a preview must never sort above the picture it previews"
    );
}

#[test]
fn evidence_orders_camera_above_date_above_size_above_nothing() {
    let nothing = Evidence::measured(Some((320, 240)), &Capture::default());
    let sized = Evidence::measured(Some((1600, 1200)), &Capture::default());
    let dated = Evidence::measured(
        Some((1600, 1200)),
        &Capture {
            taken: Some("2009:07:17 19:48:02".to_owned()),
            ..Capture::default()
        },
    );
    let named = Evidence::measured(Some((1600, 1200)), &camera("2009:07:17 19:48:02"));

    let order = [
        standing(&nothing),
        standing(&sized),
        standing(&dated),
        standing(&named),
    ];
    assert_eq!(
        order,
        [
            Standing::Unremarkable,
            Standing::PhotographSized,
            Standing::Dated,
            Standing::CameraNamed,
        ]
    );
    assert!(
        order.is_sorted(),
        "the variants must order weakest first, or sorting puts assets on top"
    );
}

#[test]
fn a_picture_that_did_not_decode_is_ranked_by_what_it_still_says() {
    // A partial prefix often fails to decode into dimensions. That is not
    // evidence the bytes are worthless, and it must not push a camera frame to
    // the bottom of the list.
    let undecoded = Evidence::measured(None, &camera("2013:01:05 09:30:00"));
    assert_eq!(standing(&undecoded), Standing::CameraNamed);

    let bare = Evidence::measured(None, &Capture::default());
    assert_eq!(standing(&bare), Standing::Unremarkable);
}

#[test]
fn the_size_floor_sits_at_the_smallest_frame_a_camera_of_the_era_produced() {
    // 640x480 is the oldest capture on the measured disk, a Canon PowerShot
    // frame. It must clear the floor; one pixel under it must not, or the
    // constant is not the boundary it claims to be.
    let era = Evidence::measured(Some((640, 480)), &Capture::default());
    let under = Evidence::measured(Some((639, 480)), &Capture::default());

    assert_eq!(standing(&era), Standing::PhotographSized);
    assert_eq!(standing(&under), Standing::Unremarkable);
}

#[test]
fn every_standing_survives_being_written_and_read_back() {
    // The manifest carries the name, and the report and the export parse it
    // back. A variant that does not round-trip is one an export silently drops.
    for standing in [
        Standing::CacheNeighbour,
        Standing::Unremarkable,
        Standing::PhotographSized,
        Standing::Dated,
        Standing::CameraNamed,
    ] {
        let text = standing.to_string();
        assert_eq!(
            text.parse::<Standing>().ok(),
            Some(standing),
            "{text} did not read back"
        );
    }
    // A triage label is not a standing; reading one as the other would order
    // a session by something the engine never said.
    "photograph".parse::<Standing>().unwrap_err();
}
