#[test]
fn probe() {
    for (name, bytes) in [
        ("png(64,64)", argos_carve::fixture::png(64, 64)),
        ("png(200,150)", argos_carve::fixture::png(200, 150)),
    ] {
        let image =
            argos_carve::decode::decode_rgba(argos_core::Format::Png, &bytes).expect("decode");
        let f = argos_classify::rules::features(&image);
        let s = argos_classify::rules::screen(&f, image.pixel_count());
        eprintln!(
            "{name:<14} texture={:.4} flat={:.4} luma={} colors={} alpha={:.3} -> {} ({})",
            f.textured_fraction,
            f.flat_run_fraction,
            f.distinct_luma,
            f.distinct_colors,
            f.transparent_fraction,
            s.label,
            s.decided_by
        );
    }
}
