use argos_core::Confidence;
use argos_core::geometry::{ByteOffset, Lba, SectorRange, SectorSize};

#[test]
fn sector_size_accepts_every_power_of_two_in_range() {
    for bytes in [512_u32, 1024, 2048, 4096] {
        let size = SectorSize::from_u32(bytes).expect("in-range power of two must be accepted");
        assert_eq!(size.get(), bytes);
    }
}

#[test]
fn sector_size_rejects_out_of_range_and_non_powers() {
    for bytes in [0_u32, 1, 256, 511, 513, 1536, 8192, u32::MAX] {
        let err = SectorSize::from_u32(bytes).expect_err("invalid size must be rejected");
        assert_eq!(err.bytes(), bytes);
        assert!(err.to_string().contains(&bytes.to_string()));
    }
}

#[test]
fn lba_to_byte_offset_is_checked() {
    let size = SectorSize::new(4096);
    assert_eq!(
        Lba::new(3).to_byte_offset(size).map(ByteOffset::get),
        Some(3 * 4096)
    );
    assert_eq!(Lba::new(u64::MAX / 2).to_byte_offset(size), None);
    assert_eq!(Lba::new(u64::MAX).checked_add(1), None);
}

#[test]
fn sector_range_end_is_checked() {
    assert_eq!(SectorRange::new(Lba::new(10), 5).end(), Some(Lba::new(15)));
    assert_eq!(SectorRange::new(Lba::new(u64::MAX), 1).end(), None);
}

#[test]
fn confidence_orders_metadata_above_all_carving() {
    let tiers = [
        Confidence::PartialOrThumbnail,
        Confidence::Reassembled,
        Confidence::ContiguousCarve,
        Confidence::JournalResidue,
        Confidence::FsMetadata,
    ];
    assert!(tiers.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(tiers.iter().max(), Some(&Confidence::FsMetadata));
}
