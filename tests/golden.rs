//! Golden tests: the mapping engine's output is pinned byte-for-byte
//! against the snapshot fixtures in `tests/fixtures`.  The injected
//! clusters isolate the engine from extraction drift (resize kernels
//! across image libraries are not bit-identical).  The remaining tests
//! pin the distribution/arrangement semantics and the config contract.

use std::path::{Path, PathBuf};

use rehue::color::{Lch, circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex, oklch_to_rgb};

use rehue::enhance::{EnhanceConfig, SrModel};
use rehue::extract::Cluster;
use rehue::map_wal::{SlotPalette, Territory, WalRegisterConfig};
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
    // The pre-v0.2 vocabulary fails loudly instead of aliasing.
    let err = serde_json::from_str::<MapConfig>("{\"registers\":{\"fg\":{\"harmonize\":0.5}}}")
        .expect_err("renamed options are not silently ignored");
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
        blend_hue: Some(0.8),
        blend_chroma: Some(0.4),
        ..Default::default()
    };
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let options = rehue::map_wal::slot_options(&regs);
    let first = rehue::map_wal::apply(pixels, 64, 64, &slots, &neutral, &options, &config);
    let second = rehue::map_wal::apply(pixels, 64, 64, &slots, &neutral, &options, &config);
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
fn wal_arrangement_and_per_register_options_are_deterministic() {
    let (_, slots) = solarized();
    let mut config = rehue::map_wal::RemapConfig::default();
    config.harmonize = 0.9;
    config.registers.insert(
        "surfaces".to_string(),
        WalRegisterConfig {
            distribution: Some(Distribution::Auto(true)),
            blend_chroma: Some(0.7),
            ..Default::default()
        },
    );
    config.registers.insert(
        "accents".to_string(),
        WalRegisterConfig {
            rotate: Some(1),
            blend_hue: Some(1.0),
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
    let options = rehue::map_wal::slot_options(&regs);
    let first = rehue::map_wal::apply(rgb.as_raw(), 64, 64, &arranged, &neutral, &options, &config);
    let second =
        rehue::map_wal::apply(rgb.as_raw(), 64, 64, &arranged, &neutral, &options, &config);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!(first.coverage, second.coverage);
}

#[test]
fn blend_dial_resolution() {
    let mut config = rehue::map_wal::RemapConfig::default();
    config.harmonize = 0.5;
    config.blend_light = 0.3;
    config.registers.insert(
        "accents".to_string(),
        WalRegisterConfig {
            blend_chroma: Some(0.1),
            ..Default::default()
        },
    );
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let options = rehue::map_wal::slot_options(&regs);
    // accents: the register's explicit chroma dial wins over the facade;
    // hue follows the facade and light is decoupled from it.
    let accents = options[8];
    assert!((accents.blend_hue - 0.5).abs() < 1e-9);
    assert!((accents.blend_light - 0.3).abs() < 1e-9);
    assert!((accents.blend_chroma - 0.1).abs() < 1e-9);
    // surfaces inherit the facade for hue and chroma; light stays where
    // the config put it (never facade-seeded).
    let surfaces = options[1];
    assert!((surfaces.blend_hue - 0.5).abs() < 1e-9);
    assert!((surfaces.blend_light - 0.3).abs() < 1e-9);
    assert!((surfaces.blend_chroma - 0.5).abs() < 1e-9);
    // An explicitly set dial beats the facade for every register.
    config.blend_hue = Some(1.0);
    let options_strict = rehue::map_wal::slot_options(
        &rehue::map_wal::resolved_registers(&config).expect("valid config"),
    );
    assert!((options_strict[1].blend_hue - 1.0).abs() < 1e-9);
    assert!((options_strict[8].blend_hue - 1.0).abs() < 1e-9);
}

#[test]
fn dithering_varies_full_blend_and_is_deterministic() {
    let (_, slots) = solarized();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let mut config = rehue::map_wal::RemapConfig::default();
    config.harmonize = 1.0;
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let options = rehue::map_wal::slot_options(&regs);

    config.dithering = 0.0;
    let plain = rehue::map_wal::apply(rgb.as_raw(), 64, 64, &slots, &neutral, &options, &config);
    config.dithering = 1.0;
    let first = rehue::map_wal::apply(rgb.as_raw(), 64, 64, &slots, &neutral, &options, &config);
    let second = rehue::map_wal::apply(rgb.as_raw(), 64, 64, &slots, &neutral, &options, &config);
    assert_eq!(first.pixels, second.pixels);
    assert!(
        first.pixels != plain.pixels,
        "dithering must change the output"
    );
}

#[test]
fn dither_modes_differ_and_each_is_deterministic() {
    let (_, slots) = solarized();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let mut config = rehue::map_wal::RemapConfig::default();
    // Partial adoption is what separates the kernels: at full strength
    // l2/c2 are the adopted targets themselves - independent of the
    // diffused residual - so every diffusion kernel would emit identical
    // pixels (and in soft territory the weighted-mean target absorbs the
    // residual entirely).  Full hue with half L/C keeps a residual field
    // the kernels actually fight over.
    config.blend_hue = Some(1.0);
    config.blend_light = 0.5;
    config.blend_chroma = Some(0.5);
    config.dithering = 1.0;
    config.territory = Some(Territory::Hard);
    let regs = rehue::map_wal::resolved_registers(&config).expect("valid config");
    let options = rehue::map_wal::slot_options(&regs);
    let mut run = |mode: rehue::map_wal::DitherMode| {
        config.dithering_mode = Some(mode);
        let first =
            rehue::map_wal::apply(rgb.as_raw(), 64, 64, &slots, &neutral, &options, &config);
        let second =
            rehue::map_wal::apply(rgb.as_raw(), 64, 64, &slots, &neutral, &options, &config);
        assert_eq!(
            first.pixels,
            second.pixels,
            "{} deterministic",
            mode.describe()
        );
        first.pixels
    };
    let blue = run(rehue::map_wal::DitherMode::BlueNoise);
    let bayer = run(rehue::map_wal::DitherMode::Bayer);
    let fs = run(rehue::map_wal::DitherMode::FloydSteinberg);
    let atkinson = run(rehue::map_wal::DitherMode::Atkinson);
    assert_ne!(bayer, blue, "distinct masks give distinct dithers");
    assert_ne!(fs, blue, "diffusion differs from ordered");
    assert_ne!(fs, atkinson, "kernels differ");
}

#[test]
fn soft_territory_blends_between_slots() {
    let slots: Vec<SlotPalette> = (0..16)
        .map(|i| SlotPalette {
            slot: BASE_SLOTS[i],
            lch: if i < 8 {
                Lch {
                    l: 0.2 + 0.03 * f64::from(i as u32),
                    c: 0.005,
                    h: 0.0,
                }
            } else {
                Lch {
                    l: 0.55,
                    c: 0.2,
                    h: if i % 2 == 0 { 0.0 } else { 90.0 },
                }
            },
        })
        .collect();
    let neutral: Vec<usize> = (0..8).collect();
    let pixel = oklch_to_rgb(&Lch {
        l: 0.55,
        c: 0.15,
        h: 60.0,
    });
    let pixels = [pixel[0], pixel[1], pixel[2]];
    let base = rehue::map_wal::RemapConfig {
        blend_hue: Some(1.0),
        reach_deg: 180.0,
        blend_light: 0.0,
        blend_chroma: Some(0.0),
        ..Default::default()
    };
    let run = |territory: Option<Territory>| {
        let mut cfg = base.clone();
        cfg.territory = territory;
        let regs = rehue::map_wal::resolved_registers(&cfg).expect("valid config");
        let options = rehue::map_wal::slot_options(&regs);
        rehue::map_wal::apply(&pixels, 1, 1, &slots, &neutral, &options, &cfg)
    };
    let hard = run(Some(Territory::Hard));
    let soft = run(Some(Territory::Soft));
    let soft_again = run(Some(Territory::Soft));
    assert_eq!(soft.pixels, soft_again.pixels, "soft stays deterministic");
    let hue_of = |buf: &[u8]| rehue::color::rgb_to_oklch(&[buf[0], buf[1], buf[2]]).h;
    let hard_h = hue_of(&hard.pixels);
    let soft_h = hue_of(&soft.pixels);
    assert!(
        circ_dist(hard_h, 90.0) < 6.0,
        "hard snaps to the nearest slot hue (90), got {hard_h}"
    );
    let to0 = circ_dist(soft_h, 0.0);
    let to90 = circ_dist(soft_h, 90.0);
    assert!(
        to0 > 5.0 && to90 > 5.0 && to0 + to90 < 91.0,
        "soft blends strictly between the two slot hues, got {soft_h}"
    );
}

#[test]
fn map_wal_cli_flags_track_the_config_surface() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let scheme = fixture("solarized-dark.yaml");
    let wall = std::env::temp_dir().join(format!("rehue-cli-wall-{}.png", std::process::id()));
    gradient_image((64, 64))
        .save(&wall)
        .expect("gradient writes");

    let run = |dir: &str, json: Option<&str>, flags: &[&str]| {
        let out = std::env::temp_dir().join(dir);
        let _ = std::fs::remove_dir_all(&out);
        let cfg = std::env::temp_dir().join(format!("{dir}.json"));
        if let Some(text) = json {
            std::fs::write(&cfg, text).expect("config writes");
        }
        let mut cmd = std::process::Command::new(bin);
        cmd.args(["map-wal", "--wallpaper"]);
        cmd.arg(&wall);
        cmd.args(["--scheme"]);
        cmd.arg(&scheme);
        cmd.args(["--out"]);
        cmd.arg(&out);
        if json.is_some() {
            cmd.args(["--config"]);
            cmd.arg(&cfg);
        }
        for flag in flags {
            cmd.arg(flag);
        }
        let status = cmd.status().expect("rehue binary runs");
        assert!(status.success(), "map-wal {dir} exited with {status}");
        std::fs::read(out.join("wallpaper.png")).expect("output readable")
    };

    let by_flags = run(
        "rehue-cli-flags",
        None,
        &[
            "--harmonize",
            "0.9",
            "--blend-chroma",
            "0.8",
            "--reach-deg",
            "45",
        ],
    );
    let by_config = run(
        "rehue-cli-json",
        Some("{\"harmonize\": 0.9, \"blend-chroma\": 0.8, \"reach-deg\": 45.0}"),
        &[],
    );
    assert_eq!(by_flags, by_config, "flags and config JSON must agree");

    let quiet = run("rehue-cli-cfg05", Some("{\"harmonize\": 0.5}"), &[]);
    let mixed = run(
        "rehue-cli-mixed",
        Some("{\"harmonize\": 0.5}"),
        &["--harmonize", "0.9"],
    );
    let loud = run("rehue-cli-flags09", None, &["--harmonize", "0.9"]);
    // A named flag beats the same option in the config file...
    assert_eq!(mixed, loud, "flag wins over the config value");
    // ...and the override actually did something.
    assert_ne!(mixed, quiet, "the override changed the output");

    // Targeted dial occurrences write the named register records.
    let targeted = run(
        "rehue-cli-targeted",
        None,
        &[
            "--blend-hue",
            "accents",
            "1",
            "--blend-light",
            "surfaces",
            "0.5",
        ],
    );
    let targeted_json = run(
        "rehue-cli-targeted-json",
        Some(
            "{\"registers\":{\"accents\":{\"blend-hue\":1.0},\"surfaces\":{\"blend-light\":0.5}}}",
        ),
        &[],
    );
    assert_eq!(
        targeted, targeted_json,
        "targeted flags match register records"
    );
    // ...and they beat the file at that key, like every other flag.
    let beats = run(
        "rehue-cli-targeted-beats",
        Some("{\"registers\":{\"accents\":{\"blend-hue\":0.3}}}"),
        &["--blend-hue", "accents", "1"],
    );
    let beats_json = run(
        "rehue-cli-beats-json",
        Some("{\"registers\":{\"accents\":{\"blend-hue\":1.0}}}"),
        &[],
    );
    assert_eq!(
        beats, beats_json,
        "targeted flag wins over the config record"
    );

    let _ = std::fs::remove_file(&wall);
    for dir in [
        "rehue-cli-flags",
        "rehue-cli-json",
        "rehue-cli-cfg05",
        "rehue-cli-mixed",
        "rehue-cli-flags09",
        "rehue-cli-targeted",
        "rehue-cli-targeted-json",
        "rehue-cli-targeted-beats",
        "rehue-cli-beats-json",
    ] {
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(dir));
    }
}

#[test]
fn map_scheme_cli_flags_track_the_config_surface() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let scheme = fixture("solarized-dark.yaml");
    let wall = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/kelly-ishmael-butterfly-closeup.webp");

    let run = |dir: &str, json: Option<&str>, flags: &[&str]| {
        let out = std::env::temp_dir().join(dir);
        let _ = std::fs::remove_dir_all(&out);
        let cfg = std::env::temp_dir().join(format!("{dir}.json"));
        if let Some(text) = json {
            std::fs::write(&cfg, text).expect("config writes");
        }
        let mut cmd = std::process::Command::new(bin);
        cmd.args(["map-scheme", "--wallpaper"]);
        cmd.arg(&wall);
        cmd.args(["--scheme"]);
        cmd.arg(&scheme);
        cmd.args(["--out"]);
        cmd.arg(&out);
        if json.is_some() {
            cmd.args(["--config"]);
            cmd.arg(&cfg);
        }
        for flag in flags {
            cmd.arg(flag);
        }
        let status = cmd.status().expect("rehue binary runs");
        assert!(status.success(), "map-scheme {dir} exited with {status}");
        std::fs::read(out.join("scheme.yaml")).expect("output readable")
    };

    let by_flags = run(
        "rehue-scheme-flags",
        None,
        &["--distribution", "3", "--rotate", "3", "--chroma", "0.75"],
    );
    let by_config = run(
        "rehue-scheme-json",
        Some("{\"registers\":{\"all\":{\"distribution\":3,\"rotate\":3,\"chroma\":0.75}}}"),
        &[],
    );
    assert_eq!(
        by_flags, by_config,
        "flags seeding `all` must match the config record"
    );

    // Per-register records beat the `all` seed, whichever way it arrives.
    let sculpted = run(
        "rehue-scheme-sculpt",
        Some("{\"registers\":{\"all\":{\"chroma\":0.75},\"accents\":{\"chroma\":1.2}}}"),
        &[],
    );
    let sculpted_mixed = run(
        "rehue-scheme-sculpt-mixed",
        Some("{\"registers\":{\"accents\":{\"chroma\":1.2}}}"),
        &["--chroma", "0.75"],
    );
    assert_eq!(
        sculpted, sculpted_mixed,
        "the accents record wins over the flag-seeded all"
    );

    // Targeted flags are the everyday sculpting form.
    let targeted = run(
        "rehue-scheme-targeted",
        None,
        &[
            "--distribution",
            "accents",
            "[0,1,2,3]",
            "--rotate",
            "accents",
            "3",
            "--chroma",
            "accents",
            "1.2",
        ],
    );
    let targeted_json = run(
        "rehue-scheme-targeted-json",
        Some(
            "{\"registers\":{\"accents\":{\"distribution\":[0,1,2,3],\"rotate\":3,\"chroma\":1.2}}}",
        ),
        &[],
    );
    assert_eq!(
        targeted, targeted_json,
        "targeted flags match register records"
    );

    // A wrong register name errors loudly instead of ghosting.
    let out = std::env::temp_dir().join("rehue-scheme-badtarget");
    let _ = std::fs::remove_dir_all(&out);
    let status = std::process::Command::new(bin)
        .args(["map-scheme", "--wallpaper"])
        .arg(&wall)
        .args(["--scheme"])
        .arg(&scheme)
        .args(["--out"])
        .arg(&out)
        .args(["--chroma", "wall", "1.6"])
        .status()
        .expect("rehue binary runs");
    assert!(!status.success(), "unknown register targets are rejected");

    for dir in [
        "rehue-scheme-flags",
        "rehue-scheme-json",
        "rehue-scheme-sculpt",
        "rehue-scheme-sculpt-mixed",
        "rehue-scheme-targeted",
        "rehue-scheme-targeted-json",
        "rehue-scheme-badtarget",
    ] {
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(dir));
    }
}

#[test]
fn collection_fixture_pins_the_embedded_text() {
    // The vendored fixture is the audit pin: an upstream scheme bump
    // changes this test, not the binary's behaviour silently.
    let text =
        rehue::scheme::collection_text("gruvbox-light").expect("collection carries gruvbox-light");
    let pinned =
        std::fs::read_to_string(fixture("gruvbox-light-upstream.yaml")).expect("fixture readable");
    assert_eq!(text, pinned);
}

#[test]
fn scheme_name_resolution() {
    // Bare name and .yaml-suffixed name resolve identically, and match a
    // file parse of the same text.
    let bare = Scheme::resolve("gruvbox-light", None).expect("bare name resolves");
    let suffixed = Scheme::resolve("gruvbox-light.yaml", None).expect("suffixed name resolves");
    assert_eq!(
        bare.slot_hexes().unwrap(),
        suffixed.slot_hexes().unwrap(),
        "name resolution is suffix-symmetric"
    );
    let from_file =
        Scheme::parse_file(&fixture("gruvbox-light-upstream.yaml")).expect("file parses");
    assert_eq!(bare.slot_hexes().unwrap(), from_file.slot_hexes().unwrap());

    // A dir-provided name wins over the collection.
    let dir = std::env::temp_dir().join("rehue-scheme-dir-probe");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("gruvbox-light.yaml"),
        "name: \"GRUVBOX-LOCAL\"\nbase00: \"123456\"\n",
    )
    .unwrap();
    let from_dir = Scheme::resolve("gruvbox-light", Some(&dir)).expect("dir resolves");
    assert_eq!(from_dir.meta_name(), "GRUVBOX-LOCAL", "scheme-dir wins");
    let _ = std::fs::remove_dir_all(&dir);

    // Unknown names error loudly and suggest the nearest names.
    let err = Scheme::resolve("gruvbox-lite", None).expect_err("unknown name");
    assert!(err.contains("unknown scheme name"), "{err}");
    assert!(err.contains("gruvbox-light"), "{err}");

    // Paths stay paths.
    let by_path =
        Scheme::resolve("tests/fixtures/solarized-dark.yaml", None).expect("path resolves");
    let by_name = Scheme::resolve("solarized-dark", None).expect("name resolves");
    assert_eq!(
        by_path.slot_hexes().unwrap(),
        by_name.slot_hexes().unwrap(),
        "path and collection name give the same slots"
    );
}

#[test]
fn schemes_subcommand_lists_sorted() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let out = std::process::Command::new(bin)
        .args(["schemes"])
        .output()
        .expect("rehue binary runs");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("gruvbox-light"), "{text}");
    assert!(
        text.contains("0x96f"),
        "the lexicographic first name\n{text}"
    );
}

#[test]
fn terminal_strips_render_and_stay_deterministic() {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let hexes = scheme.slot_hexes().expect("slots");
    let strip = rehue::terminal::scheme_strip(&hexes);
    assert_eq!(strip.lines().count(), 4, "four slots per line\n{strip}");
    assert!(
        strip.contains("002b36") && strip.contains("d33682"),
        "canonical order + hex labels\n{strip}"
    );
    assert_eq!(
        strip,
        rehue::terminal::scheme_strip(&hexes),
        "deterministic"
    );

    // Families mode: one cell per family, positions = the [i] indices.
    let families = vec![
        Cluster {
            hue: 222.0,
            weight: 0.6,
            chroma: 0.05,
        },
        Cluster {
            hue: 23.1,
            weight: 0.4,
            chroma: 0.125,
        },
    ];
    let line = rehue::terminal::families_strip(&families);

    assert_eq!(line.matches("██").count(), 2, "{line}");

    assert_eq!(
        line,
        rehue::terminal::families_strip(&families),
        "deterministic"
    );
}

// ---- enhance ----

fn enhance_temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rehue-test-enhance-{}-{}", std::process::id(), tag))
}

#[test]
fn enhance_model_table() {
    assert_eq!(SrModel::Photos.tool_name(), "realesrgan-x4plus");
    assert_eq!(SrModel::Anime.tool_name(), "realesrgan-x4plus-anime");
    assert_eq!(SrModel::Photos.native_scale(), 4);
    assert_eq!(SrModel::Anime.native_scale(), 4);
}

#[test]
fn enhance_validates_its_params() {
    let temp = enhance_temp("params");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");

    let cfg = |upscale: u32, sr: bool, sr_tool: Option<PathBuf>| EnhanceConfig {
        upscale,
        sr,
        model: SrModel::Photos,
        sr_tool,
    };
    let err =
        rehue::enhance::run(&src, &temp, &cfg(0, false, None), None).expect_err("scale 0 rejected");
    assert!(err.contains("at least 1"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(1, false, None), None)
        .expect_err("identity lanczos rejected");
    assert!(err.contains("nothing to do"), "{err}");
    let err =
        rehue::enhance::run(&src, &temp, &cfg(9, false, None), None).expect_err("lanczos cap");
    assert!(err.contains("sanity cap"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(8, true, None), None)
        .expect_err("beyond native rejected before any tool runs");
    assert!(err.contains("natively"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(2, true, Some(temp.join("absent"))), None)
        .expect_err("missing tool");
    assert!(err.contains("not a file"), "{err}");

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn enhance_lanczos_scales_deterministically() {
    let temp = enhance_temp("lanczos");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");
    let cfg = EnhanceConfig {
        upscale: 2,
        sr: false,
        model: SrModel::Photos,
        sr_tool: None,
    };
    let first = rehue::enhance::run(&src, &temp.join("a"), &cfg, None).expect("enhance runs");
    assert_eq!(first, temp.join("a").join("src.png"), "stem-named artifact");
    let second = rehue::enhance::run(&src, &temp.join("b"), &cfg, None).expect("enhance runs");
    let decoded = image::ImageReader::open(&first)
        .expect("artifact reads")
        .decode()
        .expect("artifact decodes");
    assert_eq!((decoded.width(), decoded.height()), (96, 64));
    let a = std::fs::read(&first).expect("artifact bytes");
    let b = std::fs::read(&second).expect("artifact bytes");
    assert_eq!(a, b, "lanczos enhance is byte-deterministic");

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn preview_html_renders_the_full_page_deterministically() {
    let (slot_hexes, _) = solarized();
    let html = rehue::preview::render("Solarized Dark", &slot_hexes).expect("render runs");
    for (slot, hex) in BASE_SLOTS.iter().zip(&slot_hexes) {
        assert!(
            html.contains(&format!("--{slot}: #{hex};")),
            "{slot} lands as a css variable"
        );
    }
    for token in [
        "view-palette",
        "view-gtk",
        "view-qt",
        "adwswitch",
        "qprog",
        "cursor",
    ] {
        assert!(html.contains(token), "views present: {token}");
    }
    assert!(!html.contains("{{"), "no unfilled placeholders");
    let again = rehue::preview::render("Solarized Dark", &slot_hexes).expect("render runs");
    assert_eq!(html, again, "deterministic");

    let short: Vec<String> = slot_hexes[..15].to_vec();
    let err = rehue::preview::render("x", &short).expect_err("wrong slot count");
    assert!(err.contains("16 slots"), "{err}");
}

#[test]
fn dry_run_writes_nothing() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let wall = std::env::temp_dir().join(format!("rehue-dry-wall-{}.png", std::process::id()));
    gradient_image((32, 32)).save(&wall).expect("wall writes");
    let scheme = fixture("solarized-dark.yaml");

    let run = |command: &[&str], out: &Path| {
        let _ = std::fs::remove_dir_all(out);
        let status = std::process::Command::new(bin)
            .args(command)
            .arg("--wallpaper")
            .arg(&wall)
            .args(["--scheme"])
            .arg(&scheme)
            .args(["--out"])
            .arg(out)
            .arg("--dry-run")
            .status()
            .expect("rehue binary runs");
        assert!(status.success(), "dry run {command:?} failed");
        assert!(!out.exists(), "dry run must not create --out");
    };
    run(
        &["map-wal"],
        &std::env::temp_dir().join(format!("rehue-dry-wal-{}", std::process::id())),
    );
    run(
        &["map-scheme"],
        &std::env::temp_dir().join(format!("rehue-dry-scheme-{}", std::process::id())),
    );

    let _ = std::fs::remove_file(&wall);
}
