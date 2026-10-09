//! map-wal: the per-pixel engine — territory, dither kernels,
//! arrangement, blend-dial resolution.

use rehue::color::{Lch, circ_dist, oklch_to_rgb};
use rehue::map_wal::{DitherMode, RemapConfig, SlotPalette, Territory, WalRegisterConfig};
use rehue::register::Distribution;
use rehue::scheme::BASE_SLOTS;

mod common;

use common::gradient_image;
use common::solarized;

#[test]
fn map_wal_is_deterministic() {
    let (_, slots) = solarized();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();

    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let pixels = rgb.as_raw();

    let config = RemapConfig {
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
    let mut config = RemapConfig::default();
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
        rehue::map_wal::arrange_palette(&slots, &common::four_families(), &regs).expect("arranges");

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
    let mut config = RemapConfig::default();
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
        rehue::map_wal::arrange_palette(&slots, &common::four_families(), &regs).expect("arranges");
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
    let mut config = RemapConfig::default();
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
    let mut config = RemapConfig::default();
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
    let mut config = RemapConfig::default();
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
    let mut run = |mode: DitherMode| {
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
    let blue = run(DitherMode::BlueNoise);
    let bayer = run(DitherMode::Bayer);
    let fs = run(DitherMode::FloydSteinberg);
    let atkinson = run(DitherMode::Atkinson);
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
    let base = RemapConfig {
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
    let scheme = common::fixture("solarized-dark.yaml");
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
