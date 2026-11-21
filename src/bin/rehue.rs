//! `rehue` CLI: the mapping flows, plus `inspect` and `enhance` utilities.

use std::collections::BTreeMap;
use std::path::Path;

use clap::Parser;

use rehue::cli::{Cli, Commands, LutFormat};
use rehue::color::{Lch, hex_to_oklch, hex_to_rgb, oklch_to_hex, oklch_to_rgb, self_test};
use rehue::enhance::{EnhanceConfig, SrModel, resolve_sr_tool};
use rehue::register::DistributionState;
use rehue::scheme::{BASE_SLOTS, Scheme};
use rehue::terminal::{families_strip, scheme_strip};
use rehue::{apply, extract, map_wal, preview, register};

fn read_json<T: serde::de::DeserializeOwned + Default>(path: Option<&Path>) -> Result<T, String> {
    match path {
        None => Ok(T::default()),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .map_err(|e| format!("can't read config {}: {}", p.display(), e))?;
            serde_json::from_str(&text).map_err(|e| format!("bad config {}: {}", p.display(), e))
        }
    }
}

fn fill_rect(img: &mut image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, rgb: [u8; 3]) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            img.put_pixel(x, y, image::Rgb(rgb));
        }
    }
}

/// Cluster table with explicit indices - the vocabulary distribution
/// configs refer to.
fn print_families(verb: &str, clusters: &[extract::Cluster]) {
    if clusters.is_empty() {
        return;
    }
    println!(
        "rehue {}: {} hue family(ies), dominant {:.1} deg",
        verb,
        clusters.len(),
        clusters[0].hue
    );
    for (idx, cluster) in clusters.iter().enumerate() {
        println!(
            "  [{}] hue {:>6.1}  chroma {:.3}  weight {:>5.1}%",
            idx,
            cluster.hue,
            cluster.chroma,
            cluster.weight * 100.0
        );
    }
}

/// The 16-cell before/after swatch image (reference top, mapped bottom).
fn render_preview(
    original: &BTreeMap<String, String>,
    mapped: &BTreeMap<String, String>,
) -> Result<image::RgbImage, String> {
    let cell: u32 = 24;
    let gap: u32 = 4;
    let width = cell * BASE_SLOTS.len() as u32;
    let height = cell * 2 + gap;
    let mut img = image::ImageBuffer::from_pixel(width, height, image::Rgb([34u8, 34, 39]));
    for (idx, slot) in BASE_SLOTS.iter().copied().enumerate() {
        let x0 = idx as u32 * cell + 2;
        let original_hex = original
            .get(slot)
            .ok_or_else(|| format!("missing colour {}", slot))?;
        let mapped_hex = mapped
            .get(slot)
            .ok_or_else(|| format!("missing colour {}", slot))?;
        fill_rect(
            &mut img,
            x0,
            1,
            x0 + cell - 4,
            cell - 1,
            hex_to_rgb(original_hex)?,
        );
        fill_rect(
            &mut img,
            x0,
            cell + gap,
            x0 + cell - 4,
            cell * 2 + gap - 1,
            hex_to_rgb(mapped_hex)?,
        );
    }
    Ok(img)
}

fn thumb(src: &image::RgbImage) -> image::RgbImage {
    let scale = 384.0f64 / f64::from(src.width());
    let height = 1u32.max((f64::from(src.height()) * scale) as u32);
    image::imageops::resize(src, 384, height, image::imageops::FilterType::Lanczos3)
}

/// Original vs remapped, side by side.
fn write_compare(
    out: &Path,
    original: &image::RgbImage,
    remapped: &image::RgbImage,
) -> Result<(), String> {
    let left = thumb(original);
    let right = thumb(remapped);
    let height = left.height().max(right.height()) + 8;
    let mut img = image::ImageBuffer::from_pixel(384 * 2 + 12, height, image::Rgb([22u8, 22, 26]));
    for (x, y, p) in left.enumerate_pixels() {
        img.put_pixel(x, y + 4, *p);
    }
    for (x, y, p) in right.enumerate_pixels() {
        img.put_pixel(396 + x, y + 4, *p);
    }
    img.save(out.join("compare.png"))
        .map_err(|e| format!("can't write compare.png: {}", e))
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).expect("report value always serializes");
    std::fs::write(path, text + "\n").map_err(|e| format!("can't write {}: {}", path.display(), e))
}

/// Parse one `[register] value` dial occurrence: one token is the bare
/// value, two tokens are a register target then the value.
fn dial_arg(dial: &str, tokens: &[String]) -> Result<(Option<String>, String), String> {
    match tokens.len() {
        1 => Ok((None, tokens[0].clone())),
        2 => match tokens[0].as_str() {
            "bg" | "surfaces" | "fg" | "accents" | "all" => {
                Ok((Some(tokens[0].clone()), tokens[1].clone()))
            }
            other => Err(format!(
                "--{dial}: {other} is not a register (bg, surfaces, fg, accents, all)"
            )),
        },
        n => Err(format!("--{dial}: expected [register] value, got {n} args")),
    }
}

fn parse_f64(dial: &str, token: &str) -> Result<f64, String> {
    token
        .parse()
        .map_err(|_| format!("--{dial}: expected a number, got {token}"))
}

fn parse_i64(dial: &str, token: &str) -> Result<i64, String> {
    token
        .parse()
        .map_err(|_| format!("--{dial}: expected an integer, got {token}"))
}

/// Write one dial into the `all` seed record (or the named register).
fn register_dial(
    config: &mut register::MapConfig,
    target: Option<String>,
    set: impl FnOnce(&mut register::RegisterConfig),
) {
    let name = target.unwrap_or_else(|| "all".to_string());
    set(config.registers.entry(name).or_default());
}

/// Write one dial into the `all` seed record (or the named register).
fn wal_register(
    config: &mut map_wal::RemapConfig,
    target: Option<String>,
    set: impl FnOnce(&mut map_wal::WalRegisterConfig),
) {
    let name = target.unwrap_or_else(|| "all".to_string());
    set(config.registers.entry(name).or_default());
}

/// The named-flag subset of map-scheme's options.  `reach-deg` is the
/// config-level gate; the register dials exist only per register, so a
/// bare flag seeds the `all` record (per-register records in `--config`
/// still win) and a targeted `[register] value` occurrence writes the
/// named register directly (CLI beats the file at that key).
#[derive(Debug, Default)]
struct SchemeOverrides {
    reach_deg: Option<f64>,
    distribution: Vec<String>,
    rotate: Vec<String>,
    blend_hue: Vec<String>,
    light: Vec<String>,
    chroma: Vec<String>,
}

impl SchemeOverrides {
    fn apply(self, config: &mut register::MapConfig) -> Result<(), String> {
        if let Some(v) = self.reach_deg {
            config.reach_deg = v;
        }
        for tokens in self.distribution.chunks(2) {
            let (target, value) = dial_arg("distribution", tokens)?;
            let parsed: register::Distribution = serde_json::from_str(&value)
                .map_err(|e| format!("bad --distribution {value}: {e}"))?;
            // `bg` inherits `all` and takes pins only, so a non-pin on
            // the seed path would fail validation downstream.
            if target.is_none() && !matches!(parsed, register::Distribution::Pin(_)) {
                return Err(format!(
                    "--distribution takes a family index when untargeted (got {value}); ramp/stop sculpting needs a register target or --config"
                ));
            }
            register_dial(config, target, move |reg| reg.distribution = Some(parsed));
        }
        for tokens in self.rotate.chunks(2) {
            let (target, value) = dial_arg("rotate", tokens)?;
            let v = parse_i64("rotate", &value)?;
            register_dial(config, target, move |reg| reg.rotate = Some(v));
        }
        for tokens in self.blend_hue.chunks(2) {
            let (target, value) = dial_arg("blend-hue", tokens)?;
            let v = parse_f64("blend-hue", &value)?;
            register_dial(config, target, move |reg| reg.blend_hue = Some(v));
        }
        for tokens in self.light.chunks(2) {
            let (target, value) = dial_arg("light", tokens)?;
            let v = parse_f64("light", &value)?;
            register_dial(config, target, move |reg| reg.light = Some(v));
        }
        for tokens in self.chroma.chunks(2) {
            let (target, value) = dial_arg("chroma", tokens)?;
            let v = parse_f64("chroma", &value)?;
            register_dial(config, target, move |reg| reg.chroma = Some(v));
        }
        Ok(())
    }
}

/// The named-flag subset of map-wal's options.  Global dials are bare
/// flags; register dials are targeted `[register] value` (CLI beats the
/// file at that key), and `--distribution`/`--rotate` seed the `all`
/// record when bare.  A bare `--light`/`--chroma` is the image-wide
/// grade, deliberately not a register seed.
#[derive(Debug, Default)]
struct WalOverrides {
    territory: Option<map_wal::Territory>,
    harmonize: Option<f64>,
    blend_hue: Vec<String>,
    blend_chroma: Vec<String>,
    blend_light: Vec<String>,
    reach_deg: Option<f64>,
    gray_chroma_floor: Option<f64>,
    dithering: Option<f64>,
    dithering_mode: Option<map_wal::DitherMode>,
    distribution: Vec<String>,
    rotate: Vec<String>,
    light: Vec<String>,
    chroma: Vec<String>,
}

impl WalOverrides {
    fn apply(self, config: &mut map_wal::RemapConfig) -> Result<(), String> {
        if self.territory.is_some() {
            config.territory = self.territory;
        }
        if self.dithering_mode.is_some() {
            config.dithering_mode = self.dithering_mode;
        }
        if let Some(v) = self.harmonize {
            config.harmonize = v;
        }
        if let Some(v) = self.reach_deg {
            config.reach_deg = v;
        }
        if let Some(v) = self.gray_chroma_floor {
            config.gray_chroma_floor = v;
        }
        if let Some(v) = self.dithering {
            config.dithering = v;
        }
        for tokens in self.blend_hue.chunks(2) {
            let (target, value) = dial_arg("blend-hue", tokens)?;
            let v = parse_f64("blend-hue", &value)?;
            match target {
                None => config.blend_hue = Some(v),
                Some(name) => wal_register(config, Some(name), move |reg| reg.blend_hue = Some(v)),
            }
        }
        for tokens in self.blend_chroma.chunks(2) {
            let (target, value) = dial_arg("blend-chroma", tokens)?;
            let v = parse_f64("blend-chroma", &value)?;
            match target {
                None => config.blend_chroma = Some(v),
                Some(name) => {
                    wal_register(config, Some(name), move |reg| reg.blend_chroma = Some(v))
                }
            }
        }
        for tokens in self.blend_light.chunks(2) {
            let (target, value) = dial_arg("blend-light", tokens)?;
            let v = parse_f64("blend-light", &value)?;
            match target {
                None => config.blend_light = v,
                Some(name) => {
                    wal_register(config, Some(name), move |reg| reg.blend_light = Some(v))
                }
            }
        }
        for tokens in self.distribution.chunks(2) {
            let (target, value) = dial_arg("distribution", tokens)?;
            let parsed: register::Distribution = serde_json::from_str(&value)
                .map_err(|e| format!("bad --distribution {value}: {e}"))?;
            // `bg` inherits `all` and takes pins only, so a non-pin on
            // the seed path would fail validation downstream.
            if target.is_none() && !matches!(parsed, register::Distribution::Pin(_)) {
                return Err(format!(
                    "--distribution takes a family index when untargeted (got {value}); ramp/stop sculpting needs a register target or --config"
                ));
            }
            wal_register(config, target, move |reg| reg.distribution = Some(parsed));
        }
        for tokens in self.rotate.chunks(2) {
            let (target, value) = dial_arg("rotate", tokens)?;
            let v = parse_i64("rotate", &value)?;
            wal_register(config, target, move |reg| reg.rotate = Some(v));
        }
        for tokens in self.light.chunks(2) {
            let (target, value) = dial_arg("light", tokens)?;
            let v = parse_f64("light", &value)?;
            match target {
                None => config.light = v,
                Some(name) => wal_register(config, Some(name), move |reg| reg.light = Some(v)),
            }
        }
        for tokens in self.chroma.chunks(2) {
            let (target, value) = dial_arg("chroma", tokens)?;
            let v = parse_f64("chroma", &value)?;
            match target {
                None => config.chroma = v,
                Some(name) => wal_register(config, Some(name), move |reg| reg.chroma = Some(v)),
            }
        }
        Ok(())
    }
}

/// Returns the mapped scheme's yaml (written to `out` unless dry-run).
fn run_map_scheme(
    wallpaper: &Path,
    parsed: &Scheme,
    out: &Path,
    config: register::MapConfig,
    dry_run: bool,
) -> Result<String, String> {
    println!(
        "rehue map-scheme: reference '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );

    let slot_hexes = parsed.slot_hexes()?;
    let clusters = extract::extract_hues(wallpaper, &config.extraction)?;
    if clusters.is_empty() {
        println!("rehue map-scheme: no chromatic signal - scheme hues kept");
    } else {
        print_families("map-scheme", &clusters);
        println!("rehue map-scheme: extracted wallpaper families");
        println!("{}", families_strip(&clusters));
    }

    let mapped = register::retint(&slot_hexes, &clusters, &config)?;
    let regs = register::resolved_registers(&config)?;
    for name in register::REGISTER_NAMES {
        let settings = &regs[name];
        println!(
            "  {:<8} distribution {:<12} rotate {}  blend-hue {:.2}  light {:+.3}  chroma {:.2}",
            name,
            settings.distribution.describe(),
            settings.rotate,
            settings.blend_hue,
            settings.light,
            settings.chroma
        );
    }

    let mapped_hexes: Vec<String> = BASE_SLOTS
        .iter()
        .map(|slot| mapped.get(*slot).expect("mapped slot present").clone())
        .collect();
    println!("rehue map-scheme: palette - reference");
    println!("{}", scheme_strip(&slot_hexes));
    println!("rehue map-scheme: palette - mapped");
    println!("{}", scheme_strip(&mapped_hexes));

    let rendered = parsed.render(&mapped);
    if dry_run {
        println!("rehue map-scheme: dry run (nothing written)");
        return Ok(rendered);
    }

    std::fs::create_dir_all(out).map_err(|e| format!("can't create {}: {}", out.display(), e))?;
    std::fs::write(out.join("scheme.yaml"), &rendered)
        .map_err(|e| format!("can't write scheme.yaml: {}", e))?;

    let original: BTreeMap<String, String> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| (slot.to_string(), hex.clone()))
        .collect();
    render_preview(&original, &mapped)?
        .save(out.join("preview.png"))
        .map_err(|e| format!("can't write preview.png: {}", e))?;
    let scheme_name = parsed.meta_name();
    let wall_label = preview::image_label(wallpaper);
    let wall_src = preview::relative_link(out, wallpaper);
    let input = preview::Preview {
        flow: preview::PreviewFlow::SchemeFromWallpaper,
        wallpaper_label: Some(&wall_label),
        wallpaper_src: Some(&wall_src),
        scheme: &scheme_name,
        hexes: &mapped_hexes,
    };
    std::fs::write(out.join("preview.html"), preview::render(&input)?)
        .map_err(|e| format!("can't write preview.html: {}", e))?;

    let report = serde_json::json!({
        "params": config,
        "grayscale_wallpaper": clusters.is_empty(),
        "clusters": clusters,
        "dominant_hue": clusters.first().map(|c| c.hue),
    });
    write_json_file(&out.join("clusters.json"), &report)?;

    println!("rehue map-scheme: wrote scheme.yaml, preview.png, preview.html, clusters.json");
    Ok(rendered)
}

/// Extraction, only when an arrangement dial is live (keeps plain blend
/// runs quiet); shared by the run and its dry-run preview.
/// Bake `f` into `out/lut.png` or `out/lut.cube`; returns the file name.
fn write_lut<F>(
    out: &Path,
    format: LutFormat,
    level: u32,
    title: &str,
    f: F,
) -> Result<&'static str, String>
where
    F: Fn(&[u8; 3]) -> [u8; 3] + Sync,
{
    match format {
        LutFormat::Hald => {
            rehue::lut::hald(level, f)
                .save(out.join("lut.png"))
                .map_err(|e| format!("can't write lut.png: {e}"))?;
            Ok("lut.png")
        }
        LutFormat::Cube => {
            std::fs::write(out.join("lut.cube"), rehue::lut::cube(level, title, f))
                .map_err(|e| format!("can't write lut.cube: {e}"))?;
            Ok("lut.cube")
        }
    }
}

fn wal_clusters(
    regs: &BTreeMap<&'static str, map_wal::WalRegisterSettings>,
    dynamic: &image::DynamicImage,
    config: &map_wal::RemapConfig,
) -> Vec<extract::Cluster> {
    if regs
        .values()
        .any(|r| !matches!(r.distribution, DistributionState::Off))
    {
        let extracted = extract::extract_hues_dynamic(dynamic, &config.extraction);
        if extracted.is_empty() {
            println!("rehue map-wal: no chromatic signal - distribution inert");
        } else {
            print_families("map-wal", &extracted);
        }
        extracted
    } else {
        Vec::new()
    }
}

fn run_map_wal(
    wallpaper: &Path,
    parsed: &Scheme,
    out: &Path,
    config: map_wal::RemapConfig,
    dry_run: bool,
    lut: Option<(LutFormat, u32)>,
) -> Result<(), String> {
    let regs = map_wal::resolved_registers(&config)?;
    if let Some((format, level)) = lut {
        if !rehue::lut::LEVELS.contains(&level) {
            return Err(format!("--lut-level takes 2..=16 (got {level})"));
        }
        // .cube is text: level 8 (64 points per axis) is already 6.8 MB,
        // and level 16 would be over 400 MB.
        if format == LutFormat::Cube && level > rehue::lut::CUBE_MAX_LEVEL {
            return Err(format!(
                "--lut cube takes --lut-level up to {} (64 points per axis); use --lut hald for finer or exact LUTs",
                rehue::lut::CUBE_MAX_LEVEL
            ));
        }
        let dithered =
            config.dithering > 0.0 && config.dithering_mode != Some(map_wal::DitherMode::None);
        if dithered {
            return Err(
                "--lut needs dithering 0: a dithered pixel depends on its position, not just its colour"
                    .to_string(),
            );
        }
    }
    let slot_hexes = parsed.slot_hexes()?;
    println!(
        "rehue map-wal: painting with '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );
    let slots: Vec<map_wal::SlotPalette> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| {
            Ok(map_wal::SlotPalette {
                slot,
                lch: hex_to_oklch(hex)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Arrangement recolours hues only, so neutral-slot membership is
    // resolved on the source palette.
    let mut neutral_slots: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    if neutral_slots.is_empty() {
        neutral_slots = (0..16).collect();
    }

    let dynamic = image::ImageReader::open(wallpaper)
        .map_err(|e| format!("can't read wallpaper {}: {}", wallpaper.display(), e))?
        .decode()
        .map_err(|e| format!("can't decode wallpaper {}: {}", wallpaper.display(), e))?;
    let source = dynamic.to_rgb8();

    if dry_run {
        let clusters = wal_clusters(&regs, &dynamic, &config);
        let arranged = map_wal::arrange_palette(&slots, &clusters, &regs)?;
        let strip: Vec<String> = arranged.iter().map(|s| oklch_to_hex(&s.lch)).collect();
        println!("rehue map-wal: palette - reference");
        println!("{}", scheme_strip(&slot_hexes));
        println!("rehue map-wal: palette - arranged (what would paint the wallpaper)");
        println!("{}", scheme_strip(&strip));
        println!("rehue map-wal: dry run (nothing written)");
        return Ok(());
    }

    std::fs::create_dir_all(out).map_err(|e| format!("can't create {}: {}", out.display(), e))?;

    let passive = map_wal::is_passive(&regs, config.light, config.chroma);
    let mut arranged_hexes: Option<Vec<String>> = None;
    let (remapped_rgb, coverage) = if passive {
        // Contract: output is always wallpaper.png (here = input, re-encoded).
        (source.clone(), vec![0u64; 16])
    } else {
        let clusters = wal_clusters(&regs, &dynamic, &config);

        let (seed_hue, seed_light, seed_chroma) = map_wal::facade_defaults(&config);
        println!(
            "rehue map-wal: territory {}  harmonize {:.2}  blend-hue {:.2}  blend-light {:.2}  blend-chroma {:.2}  reach {}  dithering {:.2} ({})  light {:+.3}  chroma {:.2}",
            config
                .territory
                .unwrap_or(map_wal::Territory::Soft)
                .describe(),
            config.harmonize,
            seed_hue,
            seed_light,
            seed_chroma,
            config.reach_deg,
            config.dithering,
            config
                .dithering_mode
                .unwrap_or(map_wal::DitherMode::BlueNoise)
                .describe(),
            config.light,
            config.chroma
        );
        for name in register::REGISTER_NAMES {
            let s = &regs[name];
            let non_default = !matches!(s.distribution, DistributionState::Off)
                || s.rotate != 0
                || (s.blend_hue - seed_hue).abs() > 1e-9
                || (s.blend_light - seed_light).abs() > 1e-9
                || (s.blend_chroma - seed_chroma).abs() > 1e-9
                || (s.light - config.light).abs() > 1e-9
                || (s.chroma - config.chroma).abs() > 1e-9;
            if non_default {
                println!(
                    "  {:<8} distribution {:<12} rotate {}  blend-hue {:.2}  blend-light {:.2}  blend-chroma {:.2}  light {:+.3}  chroma {:.2}",
                    name,
                    s.distribution.describe(),
                    s.rotate,
                    s.blend_hue,
                    s.blend_light,
                    s.blend_chroma,
                    s.light,
                    s.chroma
                );
            }
        }

        // Arrangement reshapes the palette before per-pixel work.
        let arranged = map_wal::arrange_palette(&slots, &clusters, &regs)?;
        arranged_hexes = Some(arranged.iter().map(|s| oklch_to_hex(&s.lch)).collect());
        let result = map_wal::apply(
            source.as_raw(),
            source.width(),
            source.height(),
            &arranged,
            &neutral_slots,
            &map_wal::slot_options(&regs),
            &config,
        );
        let pixels = image::RgbImage::from_raw(source.width(), source.height(), result.pixels)
            .expect("remap preserves the pixel-buffer size");
        (pixels, result.coverage)
    };

    remapped_rgb
        .save(out.join("wallpaper.png"))
        .map_err(|e| format!("can't write wallpaper.png: {}", e))?;
    write_compare(out, &source, &remapped_rgb)?;
    if let Some((format, level)) = lut {
        // The same palette and dials that painted the wallpaper; passive
        // runs are the identity.
        let lut_file = if passive {
            write_lut(out, format, level, &parsed.meta_name(), |rgb| *rgb)?
        } else {
            let clusters = wal_clusters(&regs, &dynamic, &config);
            let arranged = map_wal::arrange_palette(&slots, &clusters, &regs)?;
            let options = map_wal::slot_options(&regs);
            let map = map_wal::ColorMap::new(&arranged, &neutral_slots, &options, &config);
            write_lut(out, format, level, &parsed.meta_name(), |rgb| {
                map.map_rgb(rgb)
            })?
        };
        println!("rehue map-wal: wrote {lut_file} (hald level {level})");
    }

    let coverage_map: BTreeMap<String, u64> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&coverage)
        .map(|(slot, count)| (slot.to_string(), *count))
        .collect();
    let report = serde_json::json!({
        "params": config,
        "passthrough": passive,
        "slot_coverage": coverage_map,
    });
    write_json_file(&out.join("report.json"), &report)?;

    if let Some(strip_hexes) = &arranged_hexes {
        let scheme_name = parsed.meta_name();
        let wall_label = preview::image_label(wallpaper);
        let input = preview::Preview {
            flow: preview::PreviewFlow::WallpaperFromScheme,
            wallpaper_label: Some(&wall_label),
            // the repainted result is a sibling artifact
            wallpaper_src: Some("wallpaper.png"),
            scheme: &scheme_name,
            hexes: strip_hexes,
        };
        std::fs::write(out.join("preview.html"), preview::render(&input)?)
            .map_err(|e| format!("can't write preview.html: {}", e))?;
    }

    if passive {
        println!("rehue map-wal: passthrough (no options active)");
    } else {
        let total: u64 = coverage.iter().sum();
        println!("rehue map-wal: remapped {} px", total);
        if total == 0 {
            println!("rehue map-wal: no families - nothing remapped");
        }
        let mut ranked: Vec<(&str, u64)> = BASE_SLOTS
            .iter()
            .copied()
            .zip(&coverage)
            .map(|(slot, count)| (slot, *count))
            .collect();
        ranked.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        if total > 0 {
            for (slot, count) in ranked.iter().take(6) {
                println!(
                    "   pixel coverage {}: {:>5.1}%",
                    slot,
                    100.0 * f64::from(*count as u32) / f64::from(total as u32)
                );
            }
        }
    }
    println!("rehue map-wal: palette - reference");
    println!("{}", scheme_strip(&slot_hexes));
    if let Some(strip_hexes) = &arranged_hexes {
        println!("rehue map-wal: palette - arranged (what painted the wallpaper)");
        println!("{}", scheme_strip(strip_hexes));
    }
    if passive {
        println!("rehue map-wal: wrote wallpaper.png, compare.png, report.json");
    } else {
        println!("rehue map-wal: wrote wallpaper.png, compare.png, preview.html, report.json");
    }
    Ok(())
}

/// The extraction preview: indexed family table + the terminal swatch
/// strip (positions match the printed indices); the PNG strip is an
/// opt-in artifact.  Scheme mode shows a scheme's slots in terminal.
fn run_inspect(
    wallpaper: Option<&Path>,
    scheme: Option<&str>,
    scheme_dir: Option<&Path>,
    config_path: Option<&Path>,
    out: Option<&Path>,
) -> Result<(), String> {
    if scheme.is_some() && wallpaper.is_some() {
        return Err("pass --scheme or --wallpaper, not both".to_string());
    }
    if let Some(scheme) = scheme {
        let parsed = Scheme::resolve(scheme, scheme_dir)?;
        println!(
            "rehue inspect: scheme '{}' by {}",
            parsed.meta_name(),
            parsed.meta_author()
        );
        let hexes = parsed.slot_hexes()?;
        print!("{}", rehue::terminal::scheme_strip(&hexes));
        if let Some(dir) = out {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("can't create {}: {}", dir.display(), e))?;
            let scheme_name = parsed.meta_name();
            std::fs::write(
                dir.join("preview.html"),
                preview::render(&preview::Preview {
                    flow: preview::PreviewFlow::SchemeAlone,
                    wallpaper_label: None,
                    wallpaper_src: None,
                    scheme: &scheme_name,
                    hexes: &hexes,
                })?,
            )
            .map_err(|e| format!("can't write preview.html: {}", e))?;
            println!("rehue inspect: wrote preview.html");
        }
        return Ok(());
    }
    let wallpaper =
        wallpaper.ok_or_else(|| "pass --wallpaper, --scheme or --schemes".to_string())?;
    let config = read_json::<register::MapConfig>(config_path)?;
    let clusters = extract::extract_hues(wallpaper, &config.extraction)?;
    if clusters.is_empty() {
        println!("rehue inspect: no chromatic signal (greyscale wallpaper)");
        return Ok(());
    }
    print_families("inspect", &clusters);
    println!("{}", rehue::terminal::families_strip(&clusters));
    if let Some(dir) = out {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("can't create {}: {}", dir.display(), e))?;
        let cell: u32 = 44;
        let gap: u32 = 4;
        let width = 2 + clusters.len() as u32 * (cell + gap) - gap;
        let mut img = image::ImageBuffer::from_pixel(width, cell + 4, image::Rgb([22u8, 22, 26]));
        for (idx, cluster) in clusters.iter().enumerate() {
            let x0 = 2 + idx as u32 * (cell + gap);
            fill_rect(
                &mut img,
                x0,
                2,
                x0 + cell - 1,
                cell + 1,
                oklch_to_rgb(&Lch {
                    l: 0.55,
                    c: cluster.chroma,
                    h: cluster.hue,
                }),
            );
        }
        img.save(dir.join("inspect.png"))
            .map_err(|e| format!("can't write inspect.png: {}", e))?;
        println!("rehue inspect: wrote inspect.png (positions match the [i] indices)");
    }
    Ok(())
}

fn run_enhance(
    wallpaper: &Path,
    out: &Path,
    upscale: u32,
    sr: bool,
    model: SrModel,
    sr_tool: Option<&Path>,
) -> Result<(), String> {
    let tool = if sr {
        let resolved = resolve_sr_tool(sr_tool)?;
        println!(
            "rehue enhance: SR {} (native 4x) via {}",
            model.tool_name(),
            resolved.display()
        );
        Some(resolved)
    } else {
        None
    };
    let artifact = rehue::enhance::run(
        wallpaper,
        out,
        &EnhanceConfig {
            upscale,
            sr,
            model,
            sr_tool: sr_tool.map(|p| p.to_path_buf()),
        },
        tool.as_deref(),
    )?;
    println!("rehue enhance: wrote {}", artifact.display());
    Ok(())
}

/// `inspect --schemes`: every available scheme as a row of swatches.
/// A file in `scheme_dir` shadows an embedded scheme of the same name,
/// matching the lookup order of `Scheme::resolve`.
fn run_inspect_schemes(filter: &str, scheme_dir: Option<&Path>) -> Result<(), String> {
    let mut entries: Vec<(String, bool)> = Vec::new();
    if let Some(dir) = scheme_dir {
        let listing = std::fs::read_dir(dir)
            .map_err(|e| format!("can't read --scheme-dir {}: {}", dir.display(), e))?;
        for entry in listing.flatten() {
            let path = entry.path();
            let is_yaml = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yaml" | "yml")
            );
            if is_yaml
                && path.is_file()
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                entries.push((stem.to_string(), true));
            }
        }
    }
    let custom: std::collections::BTreeSet<String> =
        entries.iter().map(|(n, _)| n.clone()).collect();
    entries.extend(
        rehue::scheme::collection_names()
            .filter(|n| !custom.contains(*n))
            .map(|n| (n.to_string(), false)),
    );
    let needle = filter.to_lowercase();
    entries.retain(|(name, _)| name.to_lowercase().contains(&needle));
    entries.sort();
    if entries.is_empty() {
        return Err(format!("no scheme name contains '{filter}'"));
    }
    let width = entries.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    let custom_count = entries.iter().filter(|(_, c)| *c).count();
    let matching = if filter.is_empty() {
        String::new()
    } else {
        format!(" matching '{filter}'")
    };
    let custom_note = if custom_count > 0 {
        format!(", {custom_count} from --scheme-dir (*)")
    } else {
        String::new()
    };
    println!(
        "rehue inspect: {} scheme(s){matching}{custom_note}",
        entries.len()
    );
    for (name, is_custom) in &entries {
        let mark = if *is_custom { '*' } else { ' ' };
        match Scheme::resolve(name, scheme_dir).and_then(|s| s.slot_hexes()) {
            Ok(hexes) => println!(
                "{mark} {name:<width$}  {}",
                rehue::terminal::scheme_row(&hexes)
            ),
            Err(e) => println!("{mark} {name:<width$}  (unreadable: {e})"),
        }
    }
    Ok(())
}

/// `rehue apply`: map-scheme and/or map-wal per the pipeline, then the
/// scheme to tinty and the wallpaper to the user's command.
fn run_apply(pipeline_path: &Path, dry_run: bool) -> Result<(), String> {
    let pipeline = apply::Pipeline::load(pipeline_path)?;
    let out = pipeline.out_dir();
    let reference = Scheme::resolve(&pipeline.scheme, pipeline.scheme_dir.as_deref())?;

    // The scheme that gets applied: mapped when [map-scheme] runs, else
    // the reference as-is.
    let (scheme_yaml, scheme) = match &pipeline.map_scheme {
        Some(config) => {
            let yaml = run_map_scheme(
                &pipeline.wallpaper,
                &reference,
                &out.join("scheme"),
                config.clone(),
                dry_run,
            )?;
            let mapped = Scheme::parse_text(&yaml)?;
            (yaml, mapped)
        }
        None => (
            Scheme::resolve_text(&pipeline.scheme, pipeline.scheme_dir.as_deref())?,
            reference,
        ),
    };
    let scheme_path = out.join("scheme/scheme.yaml");

    // The wallpaper that gets set: repainted (with the applied scheme)
    // when [map-wal] runs, else the source.
    let wallpaper = match &pipeline.map_wal {
        Some(config) => {
            let dir = out.join("wallpaper");
            run_map_wal(
                &pipeline.wallpaper,
                &scheme,
                &dir,
                config.clone(),
                dry_run,
                None,
            )?;
            dir.join("wallpaper.png")
        }
        None => pipeline.wallpaper.clone(),
    };

    let target = &pipeline.apply;
    if dry_run {
        if pipeline.map_scheme.is_none() {
            println!("rehue apply: would write {}", scheme_path.display());
        }
        if target.tinty {
            println!(
                "rehue apply: would install the scheme as <tinty data dir>/custom-schemes/base16/{}.yaml",
                target.tinty_name
            );
            println!(
                "rehue apply: would run: {}",
                apply::tinty_apply_line(target)
            );
        }
        if let Some(template) = &target.wallpaper_command {
            println!(
                "rehue apply: would run: {}",
                apply::wallpaper_command_line(template, &wallpaper)
            );
        }
        println!("rehue apply: dry run (nothing written, nothing run)");
        return Ok(());
    }

    // map-scheme writes its own scheme.yaml; the reference case needs one
    // too, so `out` always holds what was applied.
    if pipeline.map_scheme.is_none() {
        let dir = out.join("scheme");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("can't create {}: {}", dir.display(), e))?;
        std::fs::write(&scheme_path, &scheme_yaml)
            .map_err(|e| format!("can't write {}: {}", scheme_path.display(), e))?;
    }
    if target.tinty {
        let installed = apply::tinty_apply(target, &scheme_yaml)?;
        println!(
            "rehue apply: tinty applied base16-{} ({})",
            target.tinty_name,
            installed.display()
        );
    }
    if let Some(template) = &target.wallpaper_command {
        let line = apply::wallpaper_command_line(template, &wallpaper);
        apply::run_shell(&line, &pipeline.base_dir, &wallpaper, &scheme_path)?;
        println!("rehue apply: wallpaper set ({})", wallpaper.display());
    }
    println!(
        "rehue apply: '{}' applied; outputs in {}",
        scheme.meta_name(),
        out.display()
    );
    Ok(())
}

fn run(command: Commands) -> Result<(), String> {
    self_test();
    match command {
        Commands::MapScheme {
            wallpaper,
            scheme,
            scheme_dir,
            out,
            config,
            dry_run,
            reach_deg,
            distribution,
            rotate,
            blend_hue,
            light,
            chroma,
        } => {
            let mut map_config = read_json::<register::MapConfig>(config.as_deref())?;
            SchemeOverrides {
                reach_deg,
                distribution,
                rotate,
                blend_hue,
                light,
                chroma,
            }
            .apply(&mut map_config)?;
            let parsed = Scheme::resolve(&scheme, scheme_dir.as_deref())?;
            run_map_scheme(
                &wallpaper,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                map_config,
                dry_run,
            )
            .map(|_| ())
        }
        Commands::MapWal {
            wallpaper,
            scheme,
            scheme_dir,
            out,
            config,
            dry_run,
            territory,
            harmonize,
            blend_hue,
            blend_chroma,
            blend_light,
            reach_deg,
            gray_chroma_floor,
            dithering,
            dithering_mode,
            distribution,
            rotate,
            light,
            chroma,
            lut,
            lut_level,
        } => {
            let mut wal_config = read_json::<map_wal::RemapConfig>(config.as_deref())?;
            WalOverrides {
                territory,
                harmonize,
                blend_hue,
                blend_chroma,
                blend_light,
                reach_deg,
                gray_chroma_floor,
                dithering,
                dithering_mode,
                distribution,
                rotate,
                light,
                chroma,
            }
            .apply(&mut wal_config)?;
            let parsed = Scheme::resolve(&scheme, scheme_dir.as_deref())?;
            run_map_wal(
                &wallpaper,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                wal_config,
                dry_run,
                lut.map(|format| (format, lut_level)),
            )
        }
        Commands::Inspect {
            schemes: Some(filter),
            scheme_dir,
            ..
        } => run_inspect_schemes(&filter, scheme_dir.as_deref()),
        Commands::Inspect {
            wallpaper,
            scheme,
            schemes: None,
            scheme_dir,
            config,
            out,
        } => run_inspect(
            wallpaper.as_deref(),
            scheme.as_deref(),
            scheme_dir.as_deref(),
            config.as_deref(),
            out.as_deref(),
        ),
        Commands::Enhance {
            wallpaper,
            out,
            upscale,
            sr,
            model,
            sr_tool,
        } => run_enhance(
            &wallpaper,
            out.as_deref().unwrap_or(Path::new("rehue-out")),
            upscale,
            sr,
            model.unwrap_or(SrModel::Photos),
            sr_tool.as_deref(),
        ),
        Commands::Schemes => {
            eprintln!("rehue: `schemes` is now `rehue inspect --schemes`");
            run_inspect_schemes("", None)
        }
        Commands::Apply { pipeline, dry_run } => run_apply(&pipeline, dry_run),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli.command) {
        eprintln!("rehue: {}", e);
        std::process::exit(1);
    }
}
