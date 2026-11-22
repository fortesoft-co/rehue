//! LUT export and the per-color remap it bakes.
//!
//! LUT export and the per-colour remap it bakes: `ColorMap::map_rgb` is
//! exactly what `grade_wal::apply` paints at dithering 0, and the Hald /
//! .cube grids reproduce it.

mod common;

use std::process::Command;

use common::{fixture, gradient_image, solarized};
use rehue::grade_wal::{ColorMap, Mode, WalConfig};
use rehue::lut::{self, grid_color, grid_value, hald, hald_lookup, samples};

fn neutral(slots: &[rehue::grade_wal::SlotPalette]) -> Vec<usize> {
    (0..16).filter(|i| slots[*i].lch.c < 0.08).collect()
}

#[test]
fn color_map_matches_apply_pixel_for_pixel() {
    // The extraction gate: the public per-colour function is the same
    // remap `apply` runs, for both territories.
    let (_, slots) = solarized();
    let neutral = neutral(&slots);
    let rgb = gradient_image((64, 48)).to_rgb8();
    for mode in [Mode::Blend, Mode::Snap] {
        let config = WalConfig {
            harmonize: 1.0,
            match_lightness: 0.4,
            mode: Some(mode),
            ..Default::default()
        };
        let regs = rehue::grade_wal::resolved_roles(&config).unwrap();
        let options = rehue::grade_wal::slot_options(&regs, &config).unwrap();
        let out = rehue::grade_wal::apply(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            &slots,
            &neutral,
            &options,
            &config,
        );
        let map = ColorMap::new(&slots, &neutral, &options, &config)
            .with_vivid(rehue::grade_wal::vivid_chroma(rgb.as_raw()));
        for (i, px) in rgb.pixels().enumerate() {
            let want = &out.pixels[i * 3..i * 3 + 3];
            assert_eq!(map.map_rgb(&px.0), want, "{mode:?} pixel {i}");
        }
    }
}

#[test]
fn grid_layout() {
    assert_eq!(samples(8), 64);
    assert_eq!(grid_value(0, 64), 0);
    assert_eq!(grid_value(63, 64), 255);
    assert_eq!(grid_value(255, 256), 255);
    assert_eq!(grid_value(17, 256), 17, "level 16 is the identity grid");
    // red fastest, then green, then blue
    assert_eq!(grid_color(1, 4), [85, 0, 0]);
    assert_eq!(grid_color(4, 4), [0, 85, 0]);
    assert_eq!(grid_color(16, 4), [0, 0, 85]);
}

#[test]
fn identity_hald_and_cube() {
    let id = hald(2, |rgb| *rgb);
    assert_eq!(id.dimensions(), (8, 8));
    for index in 0..64u32 {
        let px = id.get_pixel(index % 8, index / 8).0;
        assert_eq!(px, grid_color(index, 4));
        assert_eq!(hald_lookup(&id, 2, &px), px);
    }
    let cube = lut::cube(2, "id", |rgb| *rgb);
    let lines: Vec<&str> = cube.lines().collect();
    assert!(lines.contains(&"LUT_3D_SIZE 4"));
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
            .count(),
        64
    );
    assert!(cube.contains("\n1.000000 1.000000 1.000000\n"));
}

#[test]
fn hald_build_is_deterministic() {
    let f = |rgb: &[u8; 3]| [rgb[2], rgb[0], rgb[1]];
    assert_eq!(hald(4, f).into_raw(), hald(4, f).into_raw());
}

/// wal on a wallpaper made only of level-4 grid colours: looking each
/// pixel up in the exported Hald LUT must reproduce wallpaper.png exactly.
#[test]
fn exported_hald_reproduces_the_wallpaper() {
    let dir = std::env::temp_dir().join(format!("rehue-lut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let n = samples(4);
    let wall = image::RgbImage::from_fn(n * 4, n * n / 4, |x, y| {
        image::Rgb(grid_color(y * n * 4 + x, n))
    });
    let wall_path = dir.join("grid.png");
    wall.save(&wall_path).unwrap();

    let out = dir.join("out");
    let status = Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["wal", "-i"])
        .arg(&wall_path)
        .arg("--scheme")
        .arg(fixture("solarized-dark.yaml"))
        .arg("--out")
        .arg(&out)
        .args(["--harmonize", "1", "--lut", "hald", "--lut-level", "4"])
        .status()
        .unwrap();
    assert!(status.success());
    let painted = image::open(out.join("wallpaper.png")).unwrap().to_rgb8();
    let table = image::open(out.join("lut.png")).unwrap().to_rgb8();
    assert_eq!(table.dimensions(), (64, 64));
    for (src, dst) in wall.pixels().zip(painted.pixels()) {
        assert_eq!(hald_lookup(&table, 4, &src.0), dst.0, "colour {:?}", src.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lut_refuses_dithering_and_bad_levels() {
    let dir = std::env::temp_dir().join(format!("rehue-lut-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wall = dir.join("wall.png");
    gradient_image((16, 16)).save(&wall).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rehue"))
            .args(["wal", "-i"])
            .arg(&wall)
            .arg("--scheme")
            .arg(fixture("solarized-dark.yaml"))
            .arg("--out")
            .arg(dir.join("out"))
            .args(["--harmonize", "1"])
            .args(extra)
            .output()
            .unwrap()
    };
    let err = |o: std::process::Output| String::from_utf8_lossy(&o.stderr).into_owned();

    let dithered = run(&["--lut", "hald", "--dithering", "0.5"]);
    assert!(!dithered.status.success());
    assert!(err(dithered).contains("dithering 0"));

    let level = run(&["--lut", "hald", "--lut-level", "17"]);
    assert!(!level.status.success());
    assert!(err(level).contains("2..=16"));

    let big_cube = run(&["--lut", "cube", "--lut-level", "9"]);
    assert!(!big_cube.status.success());
    assert!(err(big_cube).contains("--lut hald"));

    let _ = std::fs::remove_dir_all(&dir);
}
