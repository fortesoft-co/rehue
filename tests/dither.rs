//! The dither stage: palette colors only, drawn as a pattern.

mod common;

use std::collections::BTreeSet;

use rehue::dither::{Dither, Ink, Sample, Scale, Style, bayer, dither};
use rehue::grade_wal::WalConfig;

use common::{fixture, gradient_image};

#[test]
fn bayer_thresholds_come_from_the_bits() {
    let two: Vec<f64> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .map(|&(x, y)| bayer(x, y, 2))
        .collect();
    assert_eq!(two, [0.125, 0.625, 0.875, 0.375], "0 2 / 3 1");
    for n in [4, 8] {
        let mut seen: Vec<u32> = (0..n * n)
            .map(|i| (bayer(i % n, i / n, n) * f64::from(n * n) - 0.5).round() as u32)
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            (0..n * n).collect::<Vec<_>>(),
            "{n}x{n} is a permutation"
        );
        assert_eq!(bayer(n, n, n), bayer(0, 0, n), "it tiles");
    }
}

#[test]
fn the_flag_form_and_the_config_form_agree() {
    let bare = Dither::parse("").unwrap();
    assert_eq!(bare, Dither::default());
    assert_eq!(bare.style, Style::Checker);
    assert_eq!(bare.scale, Scale::Auto);
    assert_eq!(bare.sample, Sample::Block);
    let d = Dither::parse("checker,scale=3,sample=pixel,flat=0.2,size=4").unwrap();
    assert_eq!(
        (d.scale, d.sample, d.flat, d.size),
        (Scale::Fixed(3), Sample::Pixel, 0.2, 4)
    );
    assert_eq!(
        Dither::parse(&d.describe()).unwrap(),
        d,
        "describe round-trips"
    );

    let from = |v: serde_json::Value| {
        serde_json::from_value::<WalConfig>(serde_json::json!({ "dither": v }))
            .map(|c| c.dither.unwrap())
    };
    assert_eq!(from(serde_json::json!("checker")).unwrap(), bare);
    assert_eq!(
        from(serde_json::json!({ "scale": 3, "sample": "pixel", "flat": 0.2, "size": 4 })).unwrap(),
        d
    );
    assert_eq!(from(serde_json::json!({ "scale": "auto" })).unwrap(), bare);

    for (text, needle) in [
        ("blots", "no dither style"),
        ("checker,size=3", "2, 4 or 8"),
        ("checker,scale=0", "1..16"),
        ("checker,scale=big", "auto or a whole number"),
        ("checker,sample=lines", "block or pixel"),
        ("checker,flat=1.5", "0..1"),
        ("checker,angle=45", "no setting 'angle'"),
        ("checker,4", "key=value"),
    ] {
        let err = Dither::parse(text).unwrap_err();
        assert!(err.contains(needle), "{text}: {err}");
    }
}

#[test]
fn auto_scale_aims_for_640_across() {
    assert_eq!(Scale::Auto.resolve(1920), 3);
    assert_eq!(Scale::Auto.resolve(3840), 6);
    assert_eq!(Scale::Auto.resolve(300), 1);
    assert_eq!(Scale::Fixed(2).resolve(3840), 2);
}

fn inks(rgbs: &[[u8; 3]]) -> Vec<Ink> {
    rgbs.iter()
        .enumerate()
        .map(|(i, &c)| Ink::new(i, c))
        .collect()
}

fn flat_image(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    (0..w * h).flat_map(|_| rgb).collect()
}

#[test]
fn a_tone_between_two_colors_becomes_a_pattern_of_them() {
    let pal = inks(&[[0, 0, 0], [255, 255, 255]]);
    let one = Dither::parse("checker,scale=1").unwrap();
    // a mid gray: a checkerboard of black and white
    let gray = rehue::color::oklch_to_rgb(&rehue::color::Lch {
        l: 0.5,
        c: 0.0,
        h: 0.0,
    });
    let (out, counts) = dither(&flat_image(8, 8, gray), 8, 8, &pal, &one);
    assert_eq!(counts.iter().sum::<u64>(), 64);
    assert_eq!(counts, [32, 32], "half each");
    assert_ne!(out[0..3], out[3..6], "neighbors alternate");
    assert_eq!(out[0..3], out[6..9], "every other pixel matches");

    // on a palette color, or inside the flat zone: solid
    let (_, counts) = dither(&flat_image(8, 8, [255, 255, 255]), 8, 8, &pal, &one);
    assert_eq!(counts, [0, 64]);
    let near = rehue::color::oklch_to_rgb(&rehue::color::Lch {
        l: 0.95,
        c: 0.0,
        h: 0.0,
    });
    let (_, counts) = dither(&flat_image(8, 8, near), 8, 8, &pal, &one);
    assert_eq!(counts, [0, 64], "within flat of white stays white");
}

#[test]
fn the_output_is_palette_colors_only_and_deterministic() {
    let img = gradient_image((96, 64)).to_rgb8();
    let pal = inks(&[
        [29, 32, 33],
        [251, 241, 199],
        [204, 36, 29],
        [69, 133, 136],
        [215, 153, 33],
    ]);
    for spec in [
        "checker",
        "checker,scale=2,size=4",
        "checker,scale=3,sample=pixel",
    ] {
        let d = Dither::parse(spec).unwrap();
        let (a, counts) = dither(img.as_raw(), 96, 64, &pal, &d);
        let (b, _) = dither(img.as_raw(), 96, 64, &pal, &d);
        assert_eq!(a, b, "{spec}: deterministic");
        assert_eq!(counts.iter().sum::<u64>(), 96 * 64);
        let colors: BTreeSet<[u8; 3]> = a.chunks(3).map(|p| [p[0], p[1], p[2]]).collect();
        let allowed: BTreeSet<[u8; 3]> = pal.iter().map(|i| i.rgb).collect();
        assert!(colors.is_subset(&allowed), "{spec}: only palette colors");
        assert!(colors.len() > 2, "{spec}: the gradient uses several");
    }
}

#[test]
fn block_sampling_makes_big_pixels_and_pixel_sampling_keeps_edges() {
    // a sharp vertical edge through the middle of 3-pixel blocks
    let (w, h) = (9u32, 3u32);
    let img: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            if i % w < 4 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            }
        })
        .collect();
    let pal = inks(&[[0, 0, 0], [255, 255, 255]]);
    let block = Dither::parse("checker,scale=3,flat=0.5").unwrap();
    let (out, _) = dither(&img, w, h, &pal, &block);
    let row: Vec<u8> = (0..w as usize).map(|x| out[x * 3]).collect();
    assert_eq!(row[3], row[4], "a block is one color");
    assert_eq!(row[3], row[5]);
    let pixel = Dither::parse("checker,scale=3,flat=0.5,sample=pixel").unwrap();
    let (out, _) = dither(&img, w, h, &pal, &pixel);
    let row: Vec<u8> = (0..w as usize).map(|x| out[x * 3]).collect();
    assert_eq!((row[3], row[4]), (0, 255), "the edge stays where it was");
}

#[test]
fn wal_dithers_with_the_schemes_colors_and_icons_leave_it_alone() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = std::env::temp_dir().join(format!("rehue-dither-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    gradient_image((64, 48)).save(&wall).unwrap();
    let scheme = fixture("solarized-dark.yaml");
    let run = |args: &[&str]| std::process::Command::new(bin).args(args).output().unwrap();
    let out = dir.join("out");
    for extra in [
        &["--dither"][..],
        &["-l", "pc98"],
        &["-H", "1", "--dither", "checker,scale=2"],
    ] {
        let mut args = vec![
            "wal",
            "-i",
            wall.to_str().unwrap(),
            "-s",
            scheme.to_str().unwrap(),
        ];
        args.extend(["-o", out.to_str().unwrap()]);
        args.extend(extra);
        let o = run(&args);
        assert!(
            o.status.success(),
            "{extra:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let img = image::open(out.join("wallpaper.png")).unwrap().to_rgb8();
        let colors: BTreeSet<[u8; 3]> = img.pixels().map(|p| p.0).collect();
        let scheme_colors: BTreeSet<[u8; 3]> = rehue::scheme::Scheme::parse_file(&scheme)
            .unwrap()
            .slot_hexes()
            .unwrap()
            .iter()
            .map(|h| {
                let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap();
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            })
            .collect();
        assert!(
            colors.is_subset(&scheme_colors),
            "{extra:?}: scheme colors only"
        );
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap())
                .unwrap();
        let covered: u64 = report["slot_coverage"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_u64().unwrap())
            .sum();
        assert_eq!(covered, 64 * 48, "{extra:?}: every pixel is a slot's");
    }

    // a look's [wal] dither doesn't reach icons; asking icons directly
    // dithers them (a single icon at 48 px)
    let svg = dir.join("app.svg");
    std::fs::write(
        &svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4" fill="#e66000"/></svg>"##,
    )
    .unwrap();
    let icons = |extra: &[&str]| {
        let mut args = vec![
            "icons",
            "-i",
            svg.to_str().unwrap(),
            "-s",
            scheme.to_str().unwrap(),
        ];
        args.extend(["-o", out.to_str().unwrap()]);
        args.extend(extra);
        run(&args)
    };
    assert!(icons(&["-l", "pc98"]).status.success());
    let cfg = dir.join("icons.toml");
    std::fs::write(&cfg, "dither = \"checker\"\n").unwrap();
    let o = icons(&["-l", "pc98"]);
    assert!(
        !String::from_utf8_lossy(&o.stdout).contains("dither"),
        "no dither from [wal]"
    );
    assert!(out.join("app.svg").is_file());
    let o = icons(&["-c", cfg.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let png = image::open(out.join("app.png")).unwrap();
    assert_eq!(png.width(), 48);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_style_takes_only_its_own_settings() {
    for (text, needle) in [
        (
            "halftone,kernel=atkinson",
            "halftone takes scale, sample, flat, size, angle",
        ),
        ("diffuse,flat=0.2", "diffuse takes scale, kernel"),
        ("diffuse,sample=pixel", "no setting 'sample'"),
        ("pattern,size=4", "pattern takes scale, sample, flat, tiles"),
        ("halftone,size=2", "3..32"),
        ("halftone,angle=120", "0..90"),
        ("diffuse,kernel=jarvis", "floyd-steinberg or atkinson"),
        ("pattern,tiles=bricks", "hatch or lines"),
    ] {
        let err = Dither::parse(text).unwrap_err();
        assert!(err.contains(needle), "{text}: {err}");
    }
    let h = Dither::parse("halftone").unwrap();
    assert_eq!(
        (h.scale, h.size, h.angle, h.flat),
        (Scale::Fixed(1), 8, 45.0, 0.2),
        "print defaults"
    );
    for spec in [
        "halftone,size=12,angle=15",
        "pattern,tiles=lines,flat=0",
        "diffuse,kernel=atkinson,scale=2",
    ] {
        let d = Dither::parse(spec).unwrap();
        assert_eq!(
            Dither::parse(&d.describe()).unwrap(),
            d,
            "{spec} round-trips"
        );
        let back: Dither = serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(back, d, "{spec} through the config form");
    }
}

#[test]
fn halftone_dots_grow_from_cell_centers_and_tiles_fill_lines_first() {
    use rehue::dither::{Tiles, halftone, tile};
    // the center of a cell is the first to darken, its corners the last
    assert!(halftone(4, 4, 8, 0.0) < halftone(0, 0, 8, 0.0));
    assert!(halftone(4, 4, 8, 0.0) < 0.05 && halftone(0, 0, 8, 0.0) > 0.9);
    // hatch: the (x + y) % 4 == 0 diagonal fills before anything else
    let first: Vec<(u32, u32)> = (0..64)
        .map(|i| (i % 8, i / 8))
        .filter(|&(x, y)| tile(x, y, Tiles::Hatch) < 0.25)
        .collect();
    assert_eq!(first.len(), 16);
    assert!(first.iter().all(|&(x, y)| (x + y) % 4 == 0));
    // lines: every fourth row first
    assert!((0..8).all(|x| tile(x, 0, Tiles::Lines) < 0.25 && tile(x, 1, Tiles::Lines) > 0.25));
}

#[test]
fn continuous_styles_cover_the_whole_range_between_two_inks() {
    let pal = inks(&[[0, 0, 0], [255, 255, 255]]);
    let share = |l: f64, spec: &str| {
        let gray = rehue::color::oklch_to_rgb(&rehue::color::Lch { l, c: 0.0, h: 0.0 });
        let (_, counts) = dither(
            &flat_image(64, 64, gray),
            64,
            64,
            &pal,
            &Dither::parse(spec).unwrap(),
        );
        counts[0] as f64 / (64.0 * 64.0)
    };
    for spec in ["halftone,flat=0", "pattern,flat=0,scale=1"] {
        // darker gray, more ink: a smooth climb, not a jump
        let shares: Vec<f64> = [0.9, 0.7, 0.5, 0.3, 0.1]
            .iter()
            .map(|&l| share(l, spec))
            .collect();
        assert!(shares.windows(2).all(|w| w[0] < w[1]), "{spec}: {shares:?}");
        assert!(
            shares[0] > 0.0 && shares[4] < 1.0,
            "{spec}: both ends mixed: {shares:?}"
        );
    }
    // diffuse keeps the average tone
    let mid = share(0.6, "diffuse,scale=1");
    assert!(mid > 0.2 && mid < 0.8, "diffuse mixes: {mid}");
    let (a, _) = dither(
        &flat_image(32, 32, [128, 128, 128]),
        32,
        32,
        &pal,
        &Dither::parse("diffuse,kernel=atkinson,scale=1").unwrap(),
    );
    let (b, _) = dither(
        &flat_image(32, 32, [128, 128, 128]),
        32,
        32,
        &pal,
        &Dither::parse("diffuse,scale=1").unwrap(),
    );
    assert_ne!(a, b, "the kernels differ");
}

#[test]
fn deband_takes_a_video_and_grain_stays_off_dithers() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = std::env::temp_dir().join(format!("rehue-grain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    gradient_image((32, 24)).save(&wall).unwrap();
    let scheme = fixture("solarized-dark.yaml");
    let fails = |extra: &[&str], says: &str| {
        let run = std::process::Command::new(bin)
            .args(["wal", "-i"])
            .arg(&wall)
            .arg("-s")
            .arg(&scheme)
            .args(extra)
            .arg("-o")
            .arg(dir.join("out"))
            .output()
            .unwrap();
        assert!(!run.status.success(), "{extra:?} should fail");
        let err = String::from_utf8_lossy(&run.stderr);
        assert!(err.contains(says), "{extra:?}: {err}");
    };
    fails(&["--deband"], "--deband takes a video");
    fails(&["--ffmpeg-args", "-crf 20"], "--ffmpeg-args takes a video");
    fails(&["--dither", "--grain"], "--grain doesn't go with --dither");
    fails(&["--grain", "40"], "--grain takes 1..=32");
    let _ = std::fs::remove_dir_all(&dir);
}
