//! wal's `palette` option: which scheme colors pixels may be pulled
//! toward, and how hard.

use std::collections::HashMap;

use rehue::color::{Lch, circ_lerp, oklch_to_rgb, rgb_to_oklch};
use rehue::grade_wal::{Mode, WalConfig, palette_weights, parse_palette};

mod common;

use common::{fixture, gradient_image, solarized};

fn palette(pairs: &[(&str, f64)]) -> Option<HashMap<String, f64>> {
    Some(pairs.iter().map(|(k, w)| (k.to_string(), *w)).collect())
}

fn weights(pairs: &[(&str, f64)]) -> Result<[f64; 16], String> {
    palette_weights(&WalConfig {
        palette: palette(pairs),
        ..Default::default()
    })
}

#[test]
fn no_palette_keeps_every_slot_at_weight_one() {
    assert_eq!(palette_weights(&WalConfig::default()).unwrap(), [1.0; 16]);
}

#[test]
fn unlisted_slots_are_left_out_and_the_most_specific_key_wins() {
    let accents = weights(&[("ac", 1.0)]).unwrap();
    assert!(accents[..8].iter().all(|w| *w == 0.0));
    assert!(accents[8..].iter().all(|w| *w == 1.0));

    // all < role < slot; slot names in any case
    let w = weights(&[("all", 1.0), ("accents", 2.0), ("base0D", 6.0), ("bg", 0.0)]).unwrap();
    assert_eq!(w[0], 0.0);
    assert_eq!(w[1], 1.0);
    assert_eq!(w[8], 2.0);
    assert_eq!(w[13], 6.0);
}

#[test]
fn bad_palettes_are_errors() {
    for (pairs, needle) in [
        (&[("blue", 1.0)][..], "not a slot"),
        (&[("ac", -1.0)][..], "0 or more"),
        (&[("ac", f64::NAN)][..], "0 or more"),
        (&[("ac", 0.0)][..], "every slot is left out"),
        (&[("bg", 1.0), ("background", 1.0)][..], "given twice"),
        (&[("base0D", 1.0), ("base0d", 2.0)][..], "given twice"),
    ] {
        let err = weights(pairs).unwrap_err();
        assert!(err.contains(needle), "{pairs:?}: {err}");
    }
    // and resolution catches it before any work is done
    let config = WalConfig {
        palette: palette(&[("nope", 1.0)]),
        ..Default::default()
    };
    assert!(rehue::grade_wal::resolved_roles(&config).is_err());
}

#[test]
fn the_cli_form_parses_bare_keys_and_weights() {
    let p = parse_palette("all, base0D=6").unwrap();
    assert_eq!(p["all"], 1.0);
    assert_eq!(p["base0D"], 6.0);
    assert_eq!(parse_palette("ac").unwrap()["ac"], 1.0);
    for bad in ["ac,,fg", "ac=x", "ac,ac", ""] {
        assert!(parse_palette(bad).is_err(), "{bad:?} parsed");
    }
}

/// One pass over `pixels` with the given palette; (pixels, coverage).
fn paint(config: &WalConfig, pixels: &[u8], w: u32, h: u32) -> (Vec<u8>, Vec<u64>) {
    let (_, slots) = solarized();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    let regs = rehue::grade_wal::resolved_roles(config).expect("valid config");
    let options = rehue::grade_wal::slot_options(&regs, config).unwrap();
    let out = rehue::grade_wal::apply(pixels, w, h, &slots, &neutral, &options, config);
    (out.pixels, out.coverage)
}

#[test]
fn an_all_one_palette_is_the_same_as_none() {
    let rgb = gradient_image((48, 48)).to_rgb8();
    for mode in [Mode::Blend, Mode::Snap] {
        let plain = WalConfig {
            harmonize: 1.0,
            match_lightness: 0.3,
            mode: Some(mode),
            ..Default::default()
        };
        let all = WalConfig {
            palette: palette(&[("all", 1.0)]),
            ..plain.clone()
        };
        assert_eq!(
            paint(&plain, rgb.as_raw(), 48, 48),
            paint(&all, rgb.as_raw(), 48, 48),
            "{mode:?}"
        );
    }
}

#[test]
fn accents_only_sends_colors_to_accents_and_leaves_grays_alone() {
    let rgb = gradient_image((48, 48)).to_rgb8();
    let config = WalConfig {
        harmonize: 1.0,
        match_lightness: 1.0,
        palette: palette(&[("ac", 1.0)]),
        ..Default::default()
    };
    let (_, coverage) = paint(&config, rgb.as_raw(), 48, 48);
    assert!(coverage[..8].iter().all(|n| *n == 0), "{coverage:?}");
    assert!(coverage[8..].iter().sum::<u64>() > 0);

    // a gray ramp passes through untouched (and isn't counted)
    let grays: Vec<u8> = (0..64u8).flat_map(|v| [v * 4, v * 4, v * 4]).collect();
    let (out, coverage) = paint(&config, &grays, 64, 1);
    assert_eq!(out, grays);
    assert_eq!(coverage.iter().sum::<u64>(), 0);
}

#[test]
fn a_one_slot_palette_paints_every_color_toward_it() {
    let rgb = gradient_image((48, 48)).to_rgb8();
    let config = WalConfig {
        harmonize: 1.0,
        palette: palette(&[("base0D", 1.0)]),
        ..Default::default()
    };
    let (_, coverage) = paint(&config, rgb.as_raw(), 48, 48);
    let chromatic: u64 = coverage.iter().sum();
    assert!(chromatic > 0);
    // every counted pixel anchored to base0D (grays went to no neutral)
    assert_eq!(coverage[13], chromatic, "{coverage:?}");
}

#[test]
fn in_snap_mode_a_heavier_slot_claims_a_wider_arc() {
    let (_, slots) = solarized();
    let (cyan, blue) = (slots[12].lch.h, slots[13].lch.h);
    // a third of the way from cyan to blue: cyan's by distance
    let lch = Lch {
        l: 0.6,
        c: 0.08,
        h: circ_lerp(cyan, blue, 0.33),
    };
    let px = oklch_to_rgb(&lch);
    let h = rgb_to_oklch(&px).h;
    assert!(rehue::color::circ_dist(h, cyan) < rehue::color::circ_dist(h, blue));
    let run = |blue_weight: f64| {
        let config = WalConfig {
            harmonize: 1.0,
            mode: Some(Mode::Snap),
            reach: 180.0,
            palette: palette(&[("base0C", 1.0), ("base0D", blue_weight)]),
            ..Default::default()
        };
        paint(&config, &px, 1, 1).1
    };
    assert_eq!(run(1.0)[12], 1);
    assert_eq!(run(3.0)[13], 1);
}

#[test]
fn the_flag_replaces_the_config_palette_and_the_preview_dims_left_out_slots() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = std::env::temp_dir().join(format!("rehue-palette-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    gradient_image((32, 32)).save(&wall).unwrap();
    let config = dir.join("wal.json");
    std::fs::write(&config, r#"{ "palette": { "fg": 1 } }"#).unwrap();
    let out = dir.join("out");
    let output = std::process::Command::new(bin)
        .args(["wal", "-H", "1", "--palette", "ac"])
        .arg("-i")
        .arg(&wall)
        .arg("-s")
        .arg(fixture("solarized-dark.yaml"))
        .arg("-c")
        .arg(&config)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("(8 left out)"), "{stdout}");
    let html = std::fs::read_to_string(out.join("preview.html")).unwrap();
    // the eight neutrals dimmed, the accents not
    assert_eq!(html.matches("class=\"sw out\"").count(), 8);

    let bad = std::process::Command::new(bin)
        .args(["wal", "--palette", "blue"])
        .arg("-i")
        .arg(&wall)
        .arg("-s")
        .arg(fixture("solarized-dark.yaml"))
        .arg("-n")
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a slot"));
    let _ = std::fs::remove_dir_all(&dir);
}
