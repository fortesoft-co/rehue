//! `rehue` CLI: the mapping flows, plus `inspect` and `enhance` utilities.

use std::collections::BTreeMap;
use std::path::Path;

use clap::Parser;

use rehue::cli::{Cli, Commands, LutFormat};
use rehue::color::{Lch, hex_to_oklch, hex_to_rgb, oklch_to_hex, oklch_to_rgb, self_test};
use rehue::enhance::{EnhanceConfig, SrModel, resolve_sr_tool};
use rehue::role::AssignState;
use rehue::scheme::{BASE_SLOTS, Scheme};
use rehue::terminal::{families_strip, scheme_strip};
use rehue::{apply, extract, grade_scheme, grade_wal, preview, role};

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

/// Cluster table with explicit indices - the vocabulary assign
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

/// Parse one `[role] value` dial occurrence: one token is the bare
/// value, two tokens are a role target then the value.
fn dial_arg(dial: &str, tokens: &[String]) -> Result<(Option<String>, String), String> {
    match tokens.len() {
        1 => Ok((None, tokens[0].clone())),
        2 => match role::canonical_role(&tokens[0]) {
            Some(name) => Ok((Some(name.to_string()), tokens[1].clone())),
            None => Err(format!(
                "--{dial}: {} is not a role ({})",
                tokens[0],
                role::role_list()
            )),
        },
        n => Err(format!("--{dial}: expected [role] value, got {n} args")),
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

/// Write one dial into the `all` seed record (or the named role).
fn role_dial(
    config: &mut grade_scheme::SchemeConfig,
    target: Option<String>,
    set: impl FnOnce(&mut grade_scheme::SchemeRoleConfig),
) {
    let name = target.unwrap_or_else(|| "all".to_string());
    set(config.roles.entry(name).or_default());
}

/// Write one dial into the `all` seed record (or the named role).
fn wal_role(
    config: &mut grade_wal::WalConfig,
    target: Option<String>,
    set: impl FnOnce(&mut grade_wal::WalRoleConfig),
) {
    let name = target.unwrap_or_else(|| "all".to_string());
    set(config.roles.entry(name).or_default());
}

/// The named-flag subset of scheme's options.  `reach` is the
/// config-level gate; the role dials exist only per role, so a
/// bare flag seeds the `all` record (per-role records in `--config`
/// still win) and a targeted `[role] value` occurrence writes the
/// named role directly (CLI beats the file at that key).
#[derive(Debug, Default)]
struct SchemeOverrides {
    reach: Option<f64>,
    assign: Vec<String>,
    cycle: Vec<String>,
    match_hue: Vec<String>,
    lightness: Vec<String>,
    chroma: Vec<String>,
}

impl SchemeOverrides {
    fn apply(self, config: &mut grade_scheme::SchemeConfig) -> Result<(), String> {
        if let Some(v) = self.reach {
            config.reach = v;
        }
        for tokens in self.assign.chunks(2) {
            let (target, value) = dial_arg("assign", tokens)?;
            let parsed = role::Assign::parse_cli(&value).map_err(|e| format!("--assign: {e}"))?;
            // `bg` inherits `all` and takes pins only, so a non-pin on
            // the seed path would fail validation downstream.
            if target.is_none() && !matches!(parsed, role::Assign::Pin(_)) {
                return Err(format!(
                    "--assign takes a family index when untargeted (got {value}); auto and lists need a role (--assign accents {value})"
                ));
            }
            role_dial(config, target, move |reg| reg.assign = Some(parsed));
        }
        for tokens in self.cycle.chunks(2) {
            let (target, value) = dial_arg("cycle", tokens)?;
            let v = parse_i64("cycle", &value)?;
            role_dial(config, target, move |reg| reg.cycle = Some(v));
        }
        for tokens in self.match_hue.chunks(2) {
            let (target, value) = dial_arg("match-hue", tokens)?;
            let v = parse_f64("match-hue", &value)?;
            role_dial(config, target, move |reg| reg.match_hue = Some(v));
        }
        for tokens in self.lightness.chunks(2) {
            let (target, value) = dial_arg("light", tokens)?;
            let v = parse_f64("light", &value)?;
            role_dial(config, target, move |reg| reg.lightness = Some(v));
        }
        for tokens in self.chroma.chunks(2) {
            let (target, value) = dial_arg("chroma", tokens)?;
            let v = parse_f64("chroma", &value)?;
            role_dial(config, target, move |reg| reg.chroma = Some(v));
        }
        Ok(())
    }
}

/// The named-flag subset of wal's options.  Global dials are bare
/// flags; role dials are targeted `[role] value` (CLI beats the
/// file at that key), and `--assign`/`--cycle` seed the `all`
/// record when bare.  A bare `--light`/`--chroma` is the image-wide
/// grade, deliberately not a role seed.
#[derive(Debug, Default)]
struct WalOverrides {
    mode: Option<grade_wal::Mode>,
    neutrals: Option<grade_wal::Neutrals>,
    harmonize: Option<f64>,
    match_hue: Vec<String>,
    match_chroma: Vec<String>,
    match_lightness: Vec<String>,
    reach: Option<f64>,
    palette: Option<String>,
    max_gray_chroma: Option<f64>,
    dithering: Option<f64>,
    dithering_mode: Option<grade_wal::DitherMode>,
    assign: Vec<String>,
    cycle: Vec<String>,
    lightness: Vec<String>,
    chroma: Vec<String>,
}

impl WalOverrides {
    fn apply(self, config: &mut grade_wal::WalConfig) -> Result<(), String> {
        if self.mode.is_some() {
            config.mode = self.mode;
        }
        if let Some(n) = self.neutrals {
            config.neutrals = n;
        }
        if self.dithering_mode.is_some() {
            config.dithering_mode = self.dithering_mode;
        }
        if let Some(v) = self.harmonize {
            config.harmonize = v;
        }
        if let Some(v) = self.reach {
            config.reach = v;
        }
        if let Some(text) = self.palette {
            // replaces the config's palette: merging subsets would blur
            // which slots are in
            config.palette =
                Some(grade_wal::parse_palette(&text).map_err(|e| format!("--palette: {e}"))?);
        }
        if let Some(v) = self.max_gray_chroma {
            config.max_gray_chroma = v;
        }
        if let Some(v) = self.dithering {
            config.dithering = v;
        }
        for tokens in self.match_hue.chunks(2) {
            let (target, value) = dial_arg("match-hue", tokens)?;
            let v = parse_f64("match-hue", &value)?;
            match target {
                None => config.match_hue = Some(v),
                Some(name) => wal_role(config, Some(name), move |reg| reg.match_hue = Some(v)),
            }
        }
        for tokens in self.match_chroma.chunks(2) {
            let (target, value) = dial_arg("match-chroma", tokens)?;
            let v = parse_f64("match-chroma", &value)?;
            match target {
                None => config.match_chroma = Some(v),
                Some(name) => wal_role(config, Some(name), move |reg| reg.match_chroma = Some(v)),
            }
        }
        for tokens in self.match_lightness.chunks(2) {
            let (target, value) = dial_arg("match-lightness", tokens)?;
            let v = parse_f64("match-lightness", &value)?;
            match target {
                None => config.match_lightness = v,
                Some(name) => {
                    wal_role(config, Some(name), move |reg| reg.match_lightness = Some(v))
                }
            }
        }
        for tokens in self.assign.chunks(2) {
            let (target, value) = dial_arg("assign", tokens)?;
            let parsed = role::Assign::parse_cli(&value).map_err(|e| format!("--assign: {e}"))?;
            // `bg` inherits `all` and takes pins only, so a non-pin on
            // the seed path would fail validation downstream.
            if target.is_none() && !matches!(parsed, role::Assign::Pin(_)) {
                return Err(format!(
                    "--assign takes a family index when untargeted (got {value}); auto and lists need a role (--assign accents {value})"
                ));
            }
            wal_role(config, target, move |reg| reg.assign = Some(parsed));
        }
        for tokens in self.cycle.chunks(2) {
            let (target, value) = dial_arg("cycle", tokens)?;
            let v = parse_i64("cycle", &value)?;
            wal_role(config, target, move |reg| reg.cycle = Some(v));
        }
        for tokens in self.lightness.chunks(2) {
            let (target, value) = dial_arg("light", tokens)?;
            let v = parse_f64("light", &value)?;
            match target {
                None => config.lightness = v,
                Some(name) => wal_role(config, Some(name), move |reg| reg.lightness = Some(v)),
            }
        }
        for tokens in self.chroma.chunks(2) {
            let (target, value) = dial_arg("chroma", tokens)?;
            let v = parse_f64("chroma", &value)?;
            match target {
                None => config.chroma = v,
                Some(name) => wal_role(config, Some(name), move |reg| reg.chroma = Some(v)),
            }
        }
        Ok(())
    }
}

/// Returns the mapped scheme's yaml (written to `out` unless dry-run).
fn run_scheme(
    wallpaper: &Path,
    parsed: &Scheme,
    out: &Path,
    config: grade_scheme::SchemeConfig,
    dry_run: bool,
) -> Result<String, String> {
    println!(
        "rehue scheme: reference '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );

    let slot_hexes = parsed.slot_hexes()?;
    let clusters = extract::extract_hues(wallpaper, &config.extraction)?;
    if clusters.is_empty() {
        println!("rehue scheme: no chromatic signal - scheme hues kept");
    } else {
        print_families("scheme", &clusters);
        println!("rehue scheme: extracted wallpaper families");
        println!("{}", families_strip(&clusters));
    }

    let mapped = grade_scheme::retint(&slot_hexes, &clusters, &config)?;
    let regs = grade_scheme::resolved_roles(&config)?;
    for name in role::ROLE_NAMES {
        let settings = &regs[name];
        println!(
            "  {:<10} assign {:<12} cycle {}  match-hue {:.2}  lightness {:+.3}  chroma {:.2}",
            name,
            settings.assign.describe(),
            settings.cycle,
            settings.match_hue,
            settings.lightness,
            settings.chroma
        );
    }

    let mapped_hexes: Vec<String> = BASE_SLOTS
        .iter()
        .map(|slot| mapped.get(*slot).expect("mapped slot present").clone())
        .collect();
    println!("rehue scheme: palette - reference");
    println!("{}", scheme_strip(&slot_hexes));
    println!("rehue scheme: palette - mapped");
    println!("{}", scheme_strip(&mapped_hexes));

    let rendered = parsed.render(&mapped);
    if dry_run {
        println!("rehue scheme: dry run (nothing written)");
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
        left_out: &[],
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

    println!("rehue scheme: wrote scheme.yaml, preview.png, preview.html, clusters.json");
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
    regs: &BTreeMap<&'static str, grade_wal::WalRoleSettings>,
    dynamic: &image::DynamicImage,
    config: &grade_wal::WalConfig,
) -> Vec<extract::Cluster> {
    if regs.values().any(|r| !matches!(r.assign, AssignState::Off)) {
        let extracted = extract::extract_hues_dynamic(dynamic, &config.extraction);
        if extracted.is_empty() {
            println!("rehue wal: no chromatic signal - assign inert");
        } else {
            print_families("wal", &extracted);
        }
        extracted
    } else {
        Vec::new()
    }
}

/// `palette    base0d 6, base0e 1  (14 left out)`: the slots in, with
/// weights unless they are all equal.
fn palette_summary(weights: &[f64; 16]) -> String {
    let kept: Vec<(&str, f64)> = BASE_SLOTS
        .iter()
        .copied()
        .zip(weights.iter().copied())
        .filter(|(_, w)| *w > 0.0)
        .collect();
    let uniform = kept.iter().all(|(_, w)| (*w - kept[0].1).abs() < 1e-9);
    let list = kept
        .iter()
        .map(|(slot, w)| {
            if uniform {
                slot.to_string()
            } else {
                format!("{slot} {w}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let out = 16 - kept.len();
    if out == 0 {
        format!("palette    {list}")
    } else {
        format!("palette    {list}  ({out} left out)")
    }
}

fn run_wal(
    wallpaper: &Path,
    parsed: &Scheme,
    out: &Path,
    config: grade_wal::WalConfig,
    dry_run: bool,
    lut: Option<(LutFormat, u32)>,
) -> Result<(), String> {
    let regs = grade_wal::resolved_roles(&config)?;
    let weights = grade_wal::palette_weights(&config)?;
    let left_out: Vec<usize> = (0..16).filter(|i| weights[*i] == 0.0).collect();
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
            config.dithering > 0.0 && config.dithering_mode != Some(grade_wal::DitherMode::None);
        if dithered {
            return Err(
                "--lut needs dithering 0: a dithered pixel depends on its position, not just its colour"
                    .to_string(),
            );
        }
    }
    let slot_hexes = parsed.slot_hexes()?;
    println!(
        "rehue wal: painting with '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );
    let slots: Vec<grade_wal::SlotPalette> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| {
            Ok(grade_wal::SlotPalette {
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
    // what blend mode measures colorfulness against (the LUT and the
    // report carry it too)
    let vivid = grade_wal::vivid_chroma(source.as_raw());

    if dry_run {
        let clusters = wal_clusters(&regs, &dynamic, &config);
        let arranged = grade_wal::arrange_palette(&slots, &clusters, &regs)?;
        let strip: Vec<String> = arranged.iter().map(|s| oklch_to_hex(&s.lch)).collect();
        println!("rehue wal: palette - reference");
        println!("{}", scheme_strip(&slot_hexes));
        println!("rehue wal: palette - arranged (what would paint the wallpaper)");
        println!("{}", scheme_strip(&strip));
        println!("rehue wal: dry run (nothing written)");
        return Ok(());
    }

    std::fs::create_dir_all(out).map_err(|e| format!("can't create {}: {}", out.display(), e))?;

    let passive = grade_wal::is_passive(&regs, config.lightness, config.chroma);
    let mut arranged_hexes: Option<Vec<String>> = None;
    let (remapped_rgb, coverage) = if passive {
        // Contract: output is always wallpaper.png (here = input, re-encoded).
        (source.clone(), vec![0u64; 16])
    } else {
        let clusters = wal_clusters(&regs, &dynamic, &config);

        let (seed_hue, seed_light, seed_chroma) = grade_wal::facade_defaults(&config);
        println!(
            "rehue wal: mode {}  neutrals {}  harmonize {:.2}  match-hue {:.2}  match-lightness {:.2}  match-chroma {:.2}  reach {}  dithering {:.2} ({})  lightness {:+.3}  chroma {:.2}",
            config.mode.unwrap_or(grade_wal::Mode::Blend).describe(),
            config.neutrals.describe(),
            config.harmonize,
            seed_hue,
            seed_light,
            seed_chroma,
            config.reach,
            config.dithering,
            config
                .dithering_mode
                .unwrap_or(grade_wal::DitherMode::BlueNoise)
                .describe(),
            config.lightness,
            config.chroma
        );
        if config.palette.is_some() {
            println!("  {}", palette_summary(&weights));
        }
        for name in role::ROLE_NAMES {
            let s = &regs[name];
            let non_default = !matches!(s.assign, AssignState::Off)
                || s.cycle != 0
                || (s.match_hue - seed_hue).abs() > 1e-9
                || (s.match_lightness - seed_light).abs() > 1e-9
                || (s.match_chroma - seed_chroma).abs() > 1e-9
                || (s.lightness - config.lightness).abs() > 1e-9
                || (s.chroma - config.chroma).abs() > 1e-9;
            if non_default {
                println!(
                    "  {:<10} assign {:<12} cycle {}  match-hue {:.2}  match-lightness {:.2}  match-chroma {:.2}  lightness {:+.3}  chroma {:.2}",
                    name,
                    s.assign.describe(),
                    s.cycle,
                    s.match_hue,
                    s.match_lightness,
                    s.match_chroma,
                    s.lightness,
                    s.chroma
                );
            }
        }

        // Arrangement reshapes the palette before per-pixel work.
        let arranged = grade_wal::arrange_palette(&slots, &clusters, &regs)?;
        arranged_hexes = Some(arranged.iter().map(|s| oklch_to_hex(&s.lch)).collect());
        let result = grade_wal::apply(
            source.as_raw(),
            source.width(),
            source.height(),
            &arranged,
            &neutral_slots,
            &grade_wal::slot_options(&regs, &config)?,
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
            let arranged = grade_wal::arrange_palette(&slots, &clusters, &regs)?;
            let options = grade_wal::slot_options(&regs, &config)?;
            let map = grade_wal::ColorMap::new(&arranged, &neutral_slots, &options, &config)
                .with_vivid(vivid);
            write_lut(out, format, level, &parsed.meta_name(), |rgb| {
                map.map_rgb(rgb)
            })?
        };
        println!("rehue wal: wrote {lut_file} (hald level {level})");
    }

    let coverage_map: BTreeMap<String, u64> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&coverage)
        .map(|(slot, count)| (slot.to_string(), *count))
        .collect();
    let report = serde_json::json!({
        "params": config,
        "vivid_chroma": vivid,
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
            left_out: &left_out,
        };
        std::fs::write(out.join("preview.html"), preview::render(&input)?)
            .map_err(|e| format!("can't write preview.html: {}", e))?;
    }

    if passive {
        println!("rehue wal: passthrough (no options active)");
    } else {
        let total: u64 = coverage.iter().sum();
        println!("rehue wal: remapped {} px", total);
        if total == 0 {
            println!("rehue wal: no families - nothing remapped");
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
    println!("rehue wal: palette - reference");
    println!("{}", scheme_strip(&slot_hexes));
    if let Some(strip_hexes) = &arranged_hexes {
        println!("rehue wal: palette - arranged (what painted the wallpaper)");
        println!("{}", scheme_strip(strip_hexes));
    }
    if passive {
        println!("rehue wal: wrote wallpaper.png, compare.png, report.json");
    } else {
        println!("rehue wal: wrote wallpaper.png, compare.png, preview.html, report.json");
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
                    left_out: &[],
                })?,
            )
            .map_err(|e| format!("can't write preview.html: {}", e))?;
            println!("rehue inspect: wrote preview.html");
        }
        return Ok(());
    }
    let wallpaper =
        wallpaper.ok_or_else(|| "pass --wallpaper, --scheme or --schemes".to_string())?;
    let config = read_json::<grade_scheme::SchemeConfig>(config_path)?;
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
/// `inspect --looks`: every look (name, what it grades, description), or
/// with a name, that look's file.
fn run_inspect_looks(name: &str) -> Result<(), String> {
    use rehue::look::{self, Source};
    if !name.is_empty() {
        let found = look::find(name)?;
        match &found.source {
            Source::BuiltIn => println!("# look '{}' (built-in)", found.name),
            Source::File(p) => println!("# look '{}' ({})", found.name, p.display()),
            Source::Override(p) => println!(
                "# look '{}' ({}, replaces the built-in)",
                found.name,
                p.display()
            ),
        }
        print!("{}", found.text);
        return Ok(());
    }
    let looks = look::all()?;
    let width = looks.iter().map(|l| l.name.len()).max().unwrap_or(0);
    let dir = look::user_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "(no config dir)".into());
    println!(
        "rehue inspect: {} look(s); your own go in {dir}",
        looks.len()
    );
    for l in &looks {
        let mark = match l.source {
            Source::BuiltIn => ' ',
            Source::File(_) => '*',
            Source::Override(_) => '+',
        };
        println!(
            "{mark} {:<width$}  {:<12}  {}",
            l.name,
            l.targets(),
            l.description
        );
    }
    if looks.iter().any(|l| l.source != Source::BuiltIn) {
        println!("  (* your own, + yours replacing a built-in)");
    }
    Ok(())
}

/// How errors name the config: its path, or "options" with none.
fn config_label(path: Option<&Path>) -> String {
    path.map_or_else(|| "options".to_string(), |p| p.display().to_string())
}

/// The looks a run used, for the summary.
fn print_looks(command: &str, looks: &[rehue::look::Look]) {
    if !looks.is_empty() {
        let names: Vec<&str> = looks.iter().map(|l| l.name.as_str()).collect();
        println!("rehue {command}: look {}", names.join(", "));
    }
}

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

/// `rehue apply`: scheme and/or wal per the pipeline, then the
/// scheme to tinty and the wallpaper to the user's command.
fn run_apply(pipeline_path: &Path, dry_run: bool) -> Result<(), String> {
    let pipeline = apply::Pipeline::load(pipeline_path)?;
    let out = pipeline.out_dir();
    let reference = Scheme::resolve(&pipeline.scheme, pipeline.scheme_dir.as_deref())?;

    // The scheme that gets applied: mapped when [rehue.scheme] runs, else
    // the reference as-is.
    let (scheme_yaml, scheme) = match &pipeline.rehue.scheme {
        Some(config) => {
            let yaml = run_scheme(
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
    // when [rehue.wal] runs, else the source.
    let wallpaper = match &pipeline.rehue.wal {
        Some(config) => {
            let dir = out.join("wallpaper");
            run_wal(
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
        if pipeline.rehue.scheme.is_none() {
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

    // scheme writes its own scheme.yaml; the reference case needs one
    // too, so `out` always holds what was applied.
    if pipeline.rehue.scheme.is_none() {
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
        Commands::Scheme {
            input,
            wallpaper,
            scheme_dir,
            out,
            config,
            look,
            dry_run,
            reach,
            assign,
            cycle,
            match_hue,
            lightness,
            chroma,
        } => {
            let (mut map_config, looks) = rehue::look::resolve::<grade_scheme::SchemeConfig>(
                &config_label(config.as_deref()),
                rehue::config::read_file(config.as_deref())?,
                &look,
                None,
                rehue::look::Target::Scheme,
            )?;
            print_looks("scheme", &looks);
            SchemeOverrides {
                reach,
                assign,
                cycle,
                match_hue,
                lightness,
                chroma,
            }
            .apply(&mut map_config)?;
            let parsed = Scheme::resolve(&input, scheme_dir.as_deref())?;
            run_scheme(
                &wallpaper,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                map_config,
                dry_run,
            )
            .map(|_| ())
        }
        Commands::Wal {
            input,
            scheme,
            scheme_dir,
            out,
            config,
            look,
            dry_run,
            mode,
            neutrals,
            harmonize,
            match_hue,
            match_chroma,
            match_lightness,
            reach,
            palette,
            max_gray_chroma,
            dithering,
            dithering_mode,
            assign,
            cycle,
            lightness,
            chroma,
            lut,
            lut_level,
        } => {
            let (mut wal_config, looks) = rehue::look::resolve::<grade_wal::WalConfig>(
                &config_label(config.as_deref()),
                rehue::config::read_file(config.as_deref())?,
                &look,
                None,
                rehue::look::Target::Wal,
            )?;
            print_looks("wal", &looks);
            WalOverrides {
                mode,
                neutrals,
                harmonize,
                match_hue,
                match_chroma,
                match_lightness,
                reach,
                palette,
                max_gray_chroma,
                dithering,
                dithering_mode,
                assign,
                cycle,
                lightness,
                chroma,
            }
            .apply(&mut wal_config)?;
            let parsed = Scheme::resolve(&scheme, scheme_dir.as_deref())?;
            run_wal(
                &input,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                wal_config,
                dry_run,
                lut.map(|format| (format, lut_level)),
            )
        }
        Commands::Inspect {
            looks: Some(name), ..
        } => run_inspect_looks(&name),
        Commands::Inspect {
            schemes: Some(filter),
            scheme_dir,
            ..
        } => run_inspect_schemes(&filter, scheme_dir.as_deref()),
        Commands::Inspect {
            wallpaper,
            scheme,
            schemes: None,
            looks: None,
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
            input,
            out,
            upscale,
            sr,
            model,
            sr_tool,
        } => run_enhance(
            &input,
            out.as_deref().unwrap_or(Path::new("rehue-out")),
            upscale,
            sr,
            model.unwrap_or(SrModel::Photos),
            sr_tool.as_deref(),
        ),
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
