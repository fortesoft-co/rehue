//! map-scheme: the register pipeline — distribution/stop semantics,
//! dial behaviour, the config contract, and the CLI surface.

use std::collections::BTreeMap;

use rehue::color::{Lch, circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex};
use rehue::extract::Cluster;
use rehue::register::{Distribution, MapConfig, RegisterConfig, retint};
use rehue::scheme::{BASE_SLOTS, Scheme};

mod common;

use common::{fixture, four_families, solarized};

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

/// Hue of a mapped slot, read back through the 8-bit render.
fn mapped_hue(mapped: &BTreeMap<String, String>, slot: &str) -> f64 {
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
