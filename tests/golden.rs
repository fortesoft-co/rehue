//! Golden tests: the mapping engine's output is pinned byte-for-byte
//! against the snapshot fixtures in `tests/fixtures`.  The injected
//! clusters isolate the engine from extraction drift (resize kernels
//! across image libraries are not bit-identical).  The remaining tests
//! pin the distribution/arrangement semantics and the config contract.

use std::path::{Path, PathBuf};

use rehue::color::{Lch, circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex};
use rehue::extract::Cluster;
use rehue::map_wal::{SlotPalette, WalRegisterConfig};
use rehue::register::{Distribution, MapConfig, RegisterConfig, retint};
use rehue::scheme::{BASE_SLOTS, Scheme};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn solarized() -> (Vec<String>, Vec<SlotPalette>) {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let slot_hexes = scheme.slot_hexes().expect("fixture has 16 slots");
    let slots: Vec<SlotPalette> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| SlotPalette {
            slot,
            lch: hex_to_oklch(hex).expect("normalized hex"),
        })
        .collect();
    (slot_hexes, slots)
}

/// Four vivid families in extraction (weight) order; chroma is high
/// enough that the 8-bit render keeps hue within ~2 degrees.
fn four_families() -> Vec<Cluster> {
    [40.0, 130.0, 250.0, 330.0]
        .iter()
        .enumerate()
        .map(|(i, h)| Cluster {
            hue: *h,
            weight: 0.4 - i as f64 * 0.1,
            chroma: 0.25,
        })
        .collect()
}

/// Hue of a mapped slot, read back through the 8-bit render.
fn mapped_hue(mapped: &std::collections::BTreeMap<String, String>, slot: &str) -> f64 {
    hex_to_oklch(mapped.get(slot).expect("slot in output"))
        .expect("normalized hex")
        .h
}

fn hue_close(a: f64, b: f64, tolerance: f64) {
    let d = circ_dist(a, b);
    assert!(
        d <= tolerance,
        "hue {} vs {} differ by {} deg (tolerance {})",
        a,
        b,
        d,
        tolerance
    );
}

#[test]
fn solarized_mapping_matches_snapshot() {
    let (slot_hexes, _) = solarized();
    // Extraction snapshot for the solarized wallpaper fixture, at full
    // precision.
    let clusters = vec![
        Cluster {
            hue: 222.10166864883163,
            weight: 0.5851796287736896,
            chroma: 0.046325400310273546,
        },
        Cluster {
            hue: 23.071858500846655,
            weight: 0.4148203712263105,
            chroma: 0.12484408205764291,
        },
    ];
    let mapped = retint(&slot_hexes, &clusters, &MapConfig::default()).expect("engine runs");
    let rendered = Scheme::parse_file(&fixture("solarized-dark.yaml"))
        .expect("fixture parses")
        .render(&mapped);
    let expected =
        std::fs::read_to_string(fixture("solarized-mapped.yaml")).expect("fixture is readable");
    assert_eq!(rendered, expected);
}

/// A chromatic gradient touching the whole hue circle, in memory.
fn gradient_image(size: (u32, u32)) -> image::DynamicImage {
    let (w, h) = size;
    let mut img = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let u = f64::from(x) / f64::from(w) * 360.0;
            let rad = u.to_radians();
            img.put_pixel(
                x,
                y,
                image::Rgb(
                    [
                        f64::from(y) / f64::from(h),
                        0.5 + 0.4 * rad.cos(),
                        0.5 + 0.4 * rad.sin(),
                    ]
                    .map(|v: f64| (v.clamp(0.0, 1.0) * 255.0) as u8),
                ),
            );
        }
    }
    image::DynamicImage::ImageRgb8(img)
}

#[test]
fn extraction_is_deterministic() {
    let image = gradient_image((128, 96));
    let params = rehue::extract::ExtractionParams::default();
    let first = rehue::extract::extract_hues_dynamic(&image, &params);
    let second = rehue::extract::extract_hues_dynamic(&image, &params);
    assert_eq!(first, second);
    assert!(!first.is_empty(), "gradient yields clusters");
}

/// A synthetic 16-slot scheme whose fg register is chromatic at mid
/// lightness, so distribution tests can measure hue through the 8-bit
/// render (solarized's near-neutral fg slots smear hue beyond tolerance).
fn vivid_slot_hexes() -> Vec<String> {
    let mut slots: Vec<String> = vec!["1b1b1b".to_string(); 4];
    for h in [40.0, 130.0, 250.0, 330.0] {
        slots.push(oklch_to_hex(&Lch { l: 0.55, c: 0.2, h }));
    }
    for i in 0..8u32 {
        slots.push(oklch_to_hex(&Lch {
            l: 0.6,
            c: 0.2,
            h: f64::from(i) * 45.0,
        }));
    }
    slots
}

#[test]
fn distribution_hues_exact() {
    let families = four_families();
    let assigned = rehue::register::distribution_hues(4, &[3, 1, 0, 2], &families);
    assert_eq!(assigned, vec![330.0, 130.0, 40.0, 250.0]);

    let two_stop = rehue::register::distribution_hues(4, &[0, 2], &families);
    assert_eq!(two_stop[0], 40.0);
    assert_eq!(two_stop[3], 250.0);
    assert!((two_stop[1] - circ_lerp(40.0, 250.0, 1.0 / 3.0)).abs() < 1e-9);
    assert!((two_stop[2] - circ_lerp(40.0, 250.0, 2.0 / 3.0)).abs() < 1e-9);
}

#[test]
fn full_length_distribution_assigns_without_interpolating() {
    let slot_hexes = vivid_slot_hexes();
    let mut config = MapConfig::default();
    config.registers.insert(
        "fg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![3, 1, 0, 2])),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    for (pos, expected) in [330.0, 130.0, 40.0, 250.0].iter().enumerate() {
        let slot = BASE_SLOTS[4 + pos];
        // 8-bit hue floor: the hex render quantizes OKLab's a/b hard enough
        // that hue 250 at c 0.2 reads back 5.6 deg off (gamma-expanded
        // lattice); the Lch-space math itself is pinned exact above.
        hue_close(mapped_hue(&mapped, slot), *expected, 8.0);
    }
}

#[test]
fn partial_distribution_interpolates_across_the_span() {
    let slot_hexes = vivid_slot_hexes();
    let mut config = MapConfig::default();
    config.registers.insert(
        "fg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![0, 2])),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    // Two stops over four slots: endpoints exact, middles interpolated.
    // 8-bit hue floor (see the full-length test's comment).
    hue_close(mapped_hue(&mapped, "base04"), 40.0, 8.0);
    hue_close(mapped_hue(&mapped, "base07"), 250.0, 8.0);
    hue_close(
        mapped_hue(&mapped, "base05"),
        circ_lerp(40.0, 250.0, 1.0 / 3.0),
        8.0,
    );
    hue_close(
        mapped_hue(&mapped, "base06"),
        circ_lerp(40.0, 250.0, 2.0 / 3.0),
        8.0,
    );
}

#[test]
fn distribution_replaces_accent_anchor_matching() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "accents".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![0, 1, 0, 1, 0, 1, 0, 1])),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    for (pos, slot) in BASE_SLOTS[8..16].iter().copied().enumerate() {
        hue_close(mapped_hue(&mapped, slot), four_families()[pos % 2].hue, 5.0);
    }
}

#[test]
fn auto_ramp_spreads_weight_order() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "surfaces".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    // Three stops = three slots: pure assignment in weight order.
    hue_close(mapped_hue(&mapped, "base01"), 40.0, 2.0);
    hue_close(mapped_hue(&mapped, "base02"), 130.0, 2.0);
    hue_close(mapped_hue(&mapped, "base03"), 250.0, 2.0);
}

#[test]
fn bg_takes_a_pin_but_rejects_true_and_arrays() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "bg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Pin(1)),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    hue_close(mapped_hue(&mapped, "base00"), 130.0, 15.0);

    config.registers.insert(
        "bg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            ..Default::default()
        },
    );
    let err = retint(&slot_hexes, &four_families(), &config)
        .expect_err("single-slot register rejects the auto ramp");
    assert!(err.contains("1 slot"), "unexpected error: {}", err);

    config.registers.insert(
        "bg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![0, 1])),
            ..Default::default()
        },
    );
    let err = retint(&slot_hexes, &four_families(), &config)
        .expect_err("single-slot register rejects arrays");
    assert!(err.contains("1 slot"), "unexpected error: {}", err);
}

#[test]
fn distribution_out_of_bounds_is_an_error() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "fg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![0, 1, 2, 3, 0])),
            ..Default::default()
        },
    );
    let err = retint(&slot_hexes, &four_families(), &config)
        .expect_err("more stops than slots is an error");
    assert!(err.contains("5 stop(s)"), "unexpected error: {}", err);

    config.registers.insert(
        "fg".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![9])),
            ..Default::default()
        },
    );
    let err = retint(&slot_hexes, &four_families(), &config)
        .expect_err("out-of-range family index is an error");
    assert!(err.contains("out of range"), "unexpected error: {}", err);
}

#[test]
fn rotate_composes_after_distribution() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "accents".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Stops(vec![0, 1, 0, 1, 0, 1, 0, 1])),
            rotate: Some(1),
            ..Default::default()
        },
    );
    let mapped = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    // Rotation moves every assignment one slot right (wrap-around): the
    // ramp becomes 1,0,1,0,... with base08 carrying base0F's stop.
    for (pos, slot) in BASE_SLOTS[8..16].iter().copied().enumerate() {
        let expected = four_families()[(pos + 7) % 8 % 2].hue;
        hue_close(mapped_hue(&mapped, slot), expected, 5.0);
    }
}

#[test]
fn stale_register_keys_are_rejected() {
    let err = serde_json::from_str::<MapConfig>("{\"registers\":{\"fg\":{\"hue-blend\":0.5}}}")
        .expect_err("renamed knobs are not silently ignored");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn distribution_is_deterministic() {
    let (slot_hexes, _) = solarized();
    let mut config = MapConfig::default();
    config.registers.insert(
        "accents".to_string(),
        RegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            chroma: Some(1.2),
            ..Default::default()
        },
    );
    let first = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    let second = retint(&slot_hexes, &four_families(), &config).expect("engine runs");
    assert_eq!(first, second);
}

#[test]
fn map_wal_is_deterministic() {
    let (_, slots) = solarized();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();

    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let pixels = rgb.as_raw();

    let config = rehue::map_wal::RemapConfig {
        harmonize: 0.8,
        quantize: 0.4,
        ..Default::default()
    };
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let knobs = rehue::map_wal::slot_knobs(&regs);
    let first = rehue::map_wal::apply(pixels, &slots, &neutral, &knobs, &config);
    let second = rehue::map_wal::apply(pixels, &slots, &neutral, &knobs, &config);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!(first.coverage, second.coverage);
}

#[test]
fn wal_arrangement_reshapes_the_palette() {
    let (_, slots) = solarized();
    let mut config = rehue::map_wal::RemapConfig::default();
    config.registers.insert(
        "surfaces".to_string(),
        WalRegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            ..Default::default()
        },
    );
    config.registers.insert(
        "accents".to_string(),
        WalRegisterConfig {
            rotate: Some(-2),
            ..Default::default()
        },
    );
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let arranged =
        rehue::map_wal::arrange_palette(&slots, &four_families(), &regs).expect("arranges");

    // surfaces: three stops over three slots - assignment in weight order.
    assert!((arranged[1].lch.h - 40.0).abs() < 1e-12);
    assert!((arranged[2].lch.h - 130.0).abs() < 1e-12);
    assert!((arranged[3].lch.h - 250.0).abs() < 1e-12);
    // Hue-only: slot lightness and chroma survive arrangement untouched.
    for (pos, slot) in BASE_SLOTS.iter().enumerate() {
        assert_eq!(arranged[pos].lch.l, slots[pos].lch.l, "slot {} l", slot);
        assert_eq!(arranged[pos].lch.c, slots[pos].lch.c, "slot {} c", slot);
    }
    // accents rotate -2 (=6): slot pos receives the hue from pos+2.
    for (idx, target) in [(8usize, 10usize), (9, 11), (10, 12), (11, 13)] {
        assert_eq!(arranged[idx].lch.h, slots[target].lch.h);
    }
    // Unset registers (bg, fg) are untouched.
    assert_eq!(arranged[0].lch.h, slots[0].lch.h);
    assert_eq!(arranged[4].lch.h, slots[4].lch.h);
}

#[test]
fn wal_arrangement_and_per_register_knobs_are_deterministic() {
    let (_, slots) = solarized();
    let mut config = rehue::map_wal::RemapConfig::default();
    config.harmonize = 0.9;
    config.registers.insert(
        "surfaces".to_string(),
        WalRegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            quantize: Some(0.7),
            ..Default::default()
        },
    );
    config.registers.insert(
        "accents".to_string(),
        WalRegisterConfig {
            rotate: Some(1),
            harmonize: Some(1.0),
            light: Some(0.02),
            ..Default::default()
        },
    );
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let arranged =
        rehue::map_wal::arrange_palette(&slots, &four_families(), &regs).expect("arranges");
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let knobs = rehue::map_wal::slot_knobs(&regs);
    let first = rehue::map_wal::apply(rgb.as_raw(), &arranged, &neutral, &knobs, &config);
    let second = rehue::map_wal::apply(rgb.as_raw(), &arranged, &neutral, &knobs, &config);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!(first.coverage, second.coverage);
}
