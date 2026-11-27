//! Scheme generation: the guarantees every generated scheme keeps.

mod common;

use common::{fixture, gradient_image};
use rehue::color::oklch_to_hex;
use rehue::generate::analysis::{Analysis, analyse, quarter_size, working_size};
use rehue::generate::math::{deutan_de, wcag};
use rehue::generate::{Method, Variant, generate};

fn analysis_of(img: &image::RgbImage) -> Analysis {
    let mut a = analyse(&[working_size(img)]);
    a.rswal_source(&quarter_size(img));
    a
}

fn colorful() -> image::RgbImage {
    gradient_image((96, 64)).to_rgb8()
}

fn gray(level: u8) -> image::RgbImage {
    image::RgbImage::from_fn(48, 32, |x, _| {
        image::Rgb([level.saturating_add((x % 3) as u8); 3])
    })
}

#[test]
fn every_method_keeps_contrast_and_red_green_apart() {
    let a = analysis_of(&colorful());
    for name in Method::NAMES {
        for variant in [Variant::Dark, Variant::Light] {
            let g = generate(&a, Method::parse(name).unwrap(), variant);
            let base00 = g.slots[0];
            for (i, accent) in g.slots[8..].iter().enumerate() {
                let ratio = wcag(accent, &base00);
                assert!(
                    ratio >= 4.45,
                    "{name} {variant:?} base0{:X}: {ratio:.2}:1",
                    i + 8
                );
            }
            assert!(
                deutan_de(&g.slots[8], &g.slots[11]) >= 0.05,
                "{name} {variant:?}: base08 and base0B too close for deuteranopes"
            );
            assert_eq!(g.light, variant == Variant::Light);
        }
    }
}

#[test]
fn generation_is_deterministic() {
    let a = analysis_of(&colorful());
    for name in Method::NAMES {
        let m = Method::parse(name).unwrap();
        let one: Vec<String> = generate(&a, m, Variant::Auto)
            .slots
            .iter()
            .map(oklch_to_hex)
            .collect();
        let two: Vec<String> = generate(&a, m, Variant::Auto)
            .slots
            .iter()
            .map(oklch_to_hex)
            .collect();
        assert_eq!(one, two, "{name}");
    }
}

#[test]
fn auto_is_found_and_says_so() {
    let a = analysis_of(&colorful());
    let auto = generate(&a, Method::Auto, Variant::Auto);
    let found = generate(&a, Method::Found, Variant::Auto);
    assert_eq!(auto.method, Method::Found);
    assert_eq!(
        auto.slots.map(|c| oklch_to_hex(&c)),
        found.slots.map(|c| oklch_to_hex(&c))
    );
}

#[test]
fn a_bright_wallpaper_gets_a_light_scheme() {
    let bright = image::RgbImage::from_fn(48, 32, |x, y| {
        image::Rgb([
            230 - (x % 20) as u8,
            215 + (y % 10) as u8,
            180 + (x % 30) as u8,
        ])
    });
    let g = generate(&analysis_of(&bright), Method::Auto, Variant::Auto);
    assert!(g.light, "median lightness over 0.6 picks light");
    assert!(
        g.slots[0].l > g.slots[5].l,
        "light: base00 lighter than text"
    );
    let dark = generate(&analysis_of(&gray(20)), Method::Auto, Variant::Auto);
    assert!(!dark.light);
}

#[test]
fn a_wallpaper_with_no_color_gets_conventional_untinted() {
    let a = analysis_of(&gray(90));
    let asked = generate(&a, Method::Distinct, Variant::Dark);
    assert_eq!(asked.method, Method::Conventional);
    assert_eq!(asked.notes, ["image has no color; using conventional"]);
    for n in &asked.slots[..8] {
        assert!(n.c < 0.002, "neutrals stay untinted: {n:?}");
    }
    let auto = generate(&a, Method::Auto, Variant::Dark);
    assert_eq!(auto.method, Method::Conventional);
    assert!(
        auto.notes.is_empty(),
        "auto asked for nothing in particular"
    );
}

#[test]
fn method_names_round_trip() {
    for name in Method::NAMES {
        assert_eq!(Method::parse(name).unwrap().name(), name);
    }
    let err = Method::parse("pywal").unwrap_err();
    assert!(err.contains("rswal"), "{err}");
}

/// The generated scheme pinned on a synthetic gradient, per method, so
/// a change to any rule shows up here.
#[test]
fn generated_schemes_are_pinned() {
    let a = analysis_of(&colorful());
    let got: Vec<(String, Vec<String>)> = ["found", "distinct", "rswal", "conventional", "triadic"]
        .iter()
        .map(|name| {
            let g = generate(&a, Method::parse(name).unwrap(), Variant::Dark);
            (name.to_string(), g.slots.iter().map(oklch_to_hex).collect())
        })
        .collect();
    let text: String = got
        .iter()
        .map(|(name, hexes)| format!("{name}: {}\n", hexes.join(" ")))
        .collect();
    let path = fixture("generated-gradient.txt");
    if std::env::var_os("REHUE_BLESS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let pinned =
        std::fs::read_to_string(&path).expect("REHUE_BLESS=1 cargo test --test generate writes it");
    assert_eq!(text, pinned);
}

fn rehue(args: &[&str], config_home: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(args)
        .env("XDG_CONFIG_HOME", config_home)
        .env("USER", "tester")
        .output()
        .expect("rehue binary runs")
}

#[test]
fn scheme_g_writes_a_named_scheme_and_saves_it() {
    let dir = std::env::temp_dir().join(format!("rehue-generate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    colorful().save(&wall).unwrap();
    let out = dir.join("out");
    let run = rehue(
        &[
            "scheme",
            "-g",
            "-w",
            wall.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--save",
            "mine",
        ],
        &dir,
    );
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let yaml = std::fs::read_to_string(out.join("scheme.yaml")).unwrap();
    assert!(yaml.contains("name: \"mine\""), "{yaml}");
    assert!(yaml.contains("variant: \""), "{yaml}");
    assert!(yaml.contains("author: \"tester\""), "{yaml}");
    let saved = dir.join("rehue/schemes/mine.yaml");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), yaml);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("clusters.json")).unwrap()).unwrap();
    assert_eq!(report["generated"]["method"], "found");
    assert_eq!(report["generated"]["method_asked"], "auto");

    // the saved scheme is found by name
    let again = rehue(
        &["scheme", "-i", "mine", "-w", wall.to_str().unwrap(), "-n"],
        &dir,
    );
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(String::from_utf8_lossy(&again.stdout).contains("reference 'mine'"));

    // a saved name is only replaced when asked (no terminal here to ask in)
    let save_mine = |extra: &[&str]| {
        let mut args = vec![
            "scheme",
            "-g",
            "distinct",
            "-w",
            wall.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--save",
            "mine",
        ];
        args.extend_from_slice(extra);
        rehue(&args, &dir)
    };
    let kept = save_mine(&[]);
    assert!(!kept.status.success());
    assert!(String::from_utf8_lossy(&kept.stderr).contains("--replace"));
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap(),
        yaml,
        "left as it was"
    );
    let replaced = save_mine(&["--replace"]);
    assert!(
        replaced.status.success(),
        "{}",
        String::from_utf8_lossy(&replaced.stderr)
    );
    assert!(String::from_utf8_lossy(&replaced.stdout).contains("(replaced)"));

    // without -i or -g there is nothing to work on
    let neither = rehue(&["scheme", "-w", wall.to_str().unwrap(), "-n"], &dir);
    assert!(!neither.status.success());
    assert!(String::from_utf8_lossy(&neither.stderr).contains("-g"));

    // a name is letters, digits, '-' or '_'
    let bad = rehue(
        &[
            "scheme",
            "-g",
            "-w",
            wall.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--save",
            "../x",
        ],
        &dir,
    );
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn match_one_takes_the_target_schemes_tint() {
    let dir = std::env::temp_dir().join(format!("rehue-generate-match-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    colorful().save(&wall).unwrap();
    let out = dir.join("out");
    let target = fixture("solarized-dark.yaml");
    let run = rehue(
        &[
            "scheme",
            "-g",
            "-w",
            wall.to_str().unwrap(),
            "-i",
            target.to_str().unwrap(),
            "-M",
            "1",
            "-o",
            out.to_str().unwrap(),
        ],
        &dir,
    );
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let made = rehue::scheme::Scheme::parse_file(&out.join("scheme.yaml")).unwrap();
    let solarized = rehue::scheme::Scheme::parse_file(&target).unwrap();
    let hue = |s: &rehue::scheme::Scheme| {
        rehue::color::hex_to_oklch(&s.slot_hexes().unwrap()[1])
            .unwrap()
            .h
    };
    let d = rehue::color::circ_dist(hue(&made), hue(&solarized));
    assert!(
        d < 3.0,
        "base01 takes solarized's tint hue ({d:.1} deg off)"
    );

    // matching fully keeps every accent at 4.5:1, dark and light
    for variant in ["dark", "light"] {
        let run = rehue(
            &[
                "scheme",
                "-g",
                "-w",
                wall.to_str().unwrap(),
                "-i",
                "nord",
                "-M",
                "1",
                "--variant",
                variant,
                "-o",
                out.to_str().unwrap(),
            ],
            &dir,
        );
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let hexes = rehue::scheme::Scheme::parse_file(&out.join("scheme.yaml"))
            .unwrap()
            .slot_hexes()
            .unwrap();
        let lch: Vec<_> = hexes
            .iter()
            .map(|h| rehue::color::hex_to_oklch(h).unwrap())
            .collect();
        for a in &lch[8..] {
            let r = wcag(a, &lch[0]);
            assert!(r >= 4.45, "{variant}: an accent at {r:.2}:1 after matching");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
