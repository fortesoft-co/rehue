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
use rehue::{apply, extract, grade_scheme, grade_wal, preview, role, video};

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
/// The 16 slots as rows of swatches, one row per palette (base00 first).
fn render_preview(rows: &[&BTreeMap<String, String>]) -> Result<image::RgbImage, String> {
    let cell: u32 = 24;
    let gap: u32 = 4;
    let width = cell * BASE_SLOTS.len() as u32;
    let height = cell * rows.len() as u32 + gap * (rows.len() as u32 - 1);
    let mut img = image::ImageBuffer::from_pixel(width, height, image::Rgb([34u8, 34, 39]));
    for (row, palette) in rows.iter().enumerate() {
        let y0 = row as u32 * (cell + gap);
        for (idx, slot) in BASE_SLOTS.iter().copied().enumerate() {
            let x0 = idx as u32 * cell + 2;
            let hex = palette
                .get(slot)
                .ok_or_else(|| format!("missing colour {}", slot))?;
            // the first row keeps a pixel's margin above it
            let top = if row == 0 { 1 } else { y0 };
            fill_rect(
                &mut img,
                x0,
                top,
                x0 + cell - 4,
                y0 + cell - 1,
                hex_to_rgb(hex)?,
            );
        }
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
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| thumb(original));
        let right = thumb(remapped);
        (left.join().expect("thumbnail thread"), right)
    });
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
    grays: Option<grade_wal::Grays>,
    harmonize: Option<f64>,
    match_hue: Vec<String>,
    match_chroma: Vec<String>,
    match_lightness: Vec<String>,
    reach: Option<f64>,
    palette: Option<String>,
    max_gray_chroma: Option<f64>,
    dither: Option<String>,
    assign: Vec<String>,
    cycle: Vec<String>,
    lightness: Vec<String>,
    chroma: Vec<String>,
}

impl WalOverrides {
    /// The shared grading flags, plus wal's own (absent for icons).
    fn from_grade(g: rehue::cli::GradeArgs, dither: Option<String>, assign: Vec<String>) -> Self {
        WalOverrides {
            mode: g.mode,
            neutrals: g.neutrals,
            grays: g.grays,
            harmonize: g.harmonize,
            match_hue: g.match_hue,
            match_chroma: g.match_chroma,
            match_lightness: g.match_lightness,
            reach: g.reach,
            palette: g.palette,
            max_gray_chroma: g.max_gray_chroma,
            dither,
            assign,
            cycle: g.cycle,
            lightness: g.lightness,
            chroma: g.chroma,
        }
    }

    fn apply(self, config: &mut grade_wal::WalConfig) -> Result<(), String> {
        if self.mode.is_some() {
            config.mode = self.mode;
        }
        if let Some(n) = self.neutrals {
            config.neutrals = n;
        }
        if let Some(g) = self.grays {
            config.grays = g;
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
        if let Some(text) = self.dither {
            // replaces the config's dither whole, like --palette
            config.dither =
                Some(rehue::dither::Dither::parse(&text).map_err(|e| format!("--dither: {e}"))?);
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
    render_preview(&[&original, &mapped])?
        .save(out.join("preview.png"))
        .map_err(|e| format!("can't write preview.png: {}", e))?;
    let scheme_name = parsed.meta_name();
    let wall_label = preview::image_label(wallpaper);
    let wall_src = preview::relative_link(out, wallpaper);
    let input = preview::Preview {
        flow: preview::PreviewFlow::SchemeFromWallpaper,
        wallpaper_label: Some(&wall_label),
        // a format the browser can't show leaves the wallpaper panel off
        wallpaper_src: preview::browser_shows(wallpaper).then_some(wall_src.as_str()),
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

/// What a `wal` run does beyond its grade: the flags that aren't grading
/// settings.
#[derive(Default)]
struct WalRun<'a> {
    dry_run: bool,
    lut: Option<(LutFormat, u32)>,
    /// ffmpeg, for a video (else found on its own).
    ffmpeg: Option<&'a Path>,
    /// Deband a video's frames before grading.
    deband: bool,
    /// Grain after grading, in levels.
    grain: Option<f64>,
    /// Encoding flags for ffmpeg, as given.
    ffmpeg_args: Option<&'a str>,
}

fn run_wal(
    wallpaper: &Path,
    parsed: &Scheme,
    out: &Path,
    config: grade_wal::WalConfig,
    run: WalRun,
) -> Result<(), String> {
    let WalRun {
        dry_run,
        lut,
        ffmpeg,
        deband,
        grain,
        ffmpeg_args,
    } = run;
    let encode = match ffmpeg_args {
        Some(text) => {
            if !video::is_video(wallpaper) {
                return Err("--ffmpeg-args takes a video (it sets how one is encoded)".to_string());
            }
            video::EncodeArgs::parse(text)?
        }
        None => video::EncodeArgs::default(),
    };
    if let Some(amount) = grain {
        if !(1.0..=rehue::grain::MAX).contains(&amount) {
            return Err(format!(
                "--grain takes 1..={} (got {amount})",
                rehue::grain::MAX
            ));
        }
        if config.dither.is_some() {
            return Err(
                "--grain doesn't go with --dither: a dither draws with the scheme's colors only"
                    .to_string(),
            );
        }
    }
    if deband && !video::is_video(wallpaper) {
        return Err(
            "--deband takes a video (it smooths compression banding in its frames)".to_string(),
        );
    }
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

    // A video is graded from frames sampled across it, so the grade holds
    // still from start to end; its frames go through ffmpeg below.
    let video = if video::is_video(wallpaper) {
        if config.dither.is_some() {
            return Err("--dither doesn't take a video yet; grade it without".to_string());
        }
        let tool = video::resolve_ffmpeg(ffmpeg)?;
        let probe = video::probe(&tool, wallpaper)?;
        println!(
            "rehue wal: video {}x{} at {:.3} fps, {:.1} s; the grade is measured on {} frames across it",
            probe.width,
            probe.height,
            probe.fps_value(),
            probe.duration,
            video::SAMPLE_FRAMES
        );
        Some((tool, probe))
    } else {
        None
    };
    let dynamic = match &video {
        Some((tool, probe)) => {
            image::DynamicImage::ImageRgb8(video::sample(tool, wallpaper, probe, deband)?)
        }
        None => image::ImageReader::open(wallpaper)
            .map_err(|e| format!("can't read wallpaper {}: {}", wallpaper.display(), e))?
            .decode()
            .map_err(|e| format!("can't decode wallpaper {}: {}", wallpaper.display(), e))?,
    };
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
    // the palette and settings a video's frames are graded with
    let mut grade_inputs: Option<(Vec<grade_wal::SlotPalette>, Vec<grade_wal::SlotOptions>)> = None;
    let (remapped_rgb, coverage) = if passive {
        // Contract: output is always wallpaper.png (here = input,
        // re-encoded, or dithered as it is when a dither is set).
        match &config.dither {
            None => (source.clone(), vec![0u64; 16]),
            Some(d) => {
                println!("rehue wal: dither {}", d.describe());
                arranged_hexes = Some(slot_hexes.clone());
                let options = grade_wal::slot_options(&regs, &config)?;
                let (px, counts) = grade_wal::dither_stage(
                    source.as_raw(),
                    source.width(),
                    source.height(),
                    &slots,
                    &options,
                    d,
                );
                let pixels = image::RgbImage::from_raw(source.width(), source.height(), px)
                    .expect("dithering preserves the pixel-buffer size");
                (pixels, counts)
            }
        }
    } else {
        let clusters = wal_clusters(&regs, &dynamic, &config);

        let (seed_hue, seed_light, seed_chroma) = grade_wal::facade_defaults(&config);
        println!(
            "rehue wal: mode {}  neutrals {}  grays {}  harmonize {:.2}  match-hue {:.2}  match-lightness {:.2}  match-chroma {:.2}  reach {}  lightness {:+.3}  chroma {:.2}",
            config.mode.unwrap_or(grade_wal::Mode::Blend).describe(),
            config.neutrals.describe(),
            config.grays.describe(),
            config.harmonize,
            seed_hue,
            seed_light,
            seed_chroma,
            config.reach,
            config.lightness,
            config.chroma
        );
        if config.palette.is_some() {
            println!("  {}", palette_summary(&weights));
        }
        if let Some(d) = &config.dither {
            println!("  dither {}", d.describe());
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
        let options = grade_wal::slot_options(&regs, &config)?;
        if video.is_some() {
            // graded frame by frame below
            grade_inputs = Some((arranged, options));
            (source.clone(), vec![0u64; 16])
        } else {
            let result = grade_wal::apply(
                source.as_raw(),
                source.width(),
                source.height(),
                &arranged,
                &neutral_slots,
                &options,
                &config,
            );
            let pixels = image::RgbImage::from_raw(source.width(), source.height(), result.pixels)
                .expect("remap preserves the pixel-buffer size");
            (pixels, result.coverage)
        }
    };

    // A video: every frame through the grade, and the first frame stands
    // in for the picture in compare.png.
    let (source, remapped_rgb, coverage, video_info, wallpaper_file) = match &video {
        None => {
            let mut remapped_rgb = remapped_rgb;
            if let Some(amount) = grain {
                let width = remapped_rgb.width();
                rehue::grain::apply(&mut remapped_rgb, width, 0, amount);
            }
            (
                source,
                remapped_rgb,
                coverage,
                None,
                "wallpaper.png".to_string(),
            )
        }
        Some((tool, probe)) => {
            let ext = wallpaper
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "mp4".to_string());
            let file = format!("wallpaper.{ext}");
            let table = grade_inputs.as_ref().map(|(arranged, options)| {
                let map = grade_wal::ColorMap::new(arranged, &neutral_slots, options, &config)
                    .with_vivid(vivid);
                grade_wal::ColorTable::new(&map, arranged.len())
            });
            let graded = grade_video(
                tool,
                wallpaper,
                &out.join(&file),
                probe,
                table.as_ref(),
                FrameSteps {
                    deband,
                    grain,
                    encode: &encode,
                },
            )?;
            println!("rehue wal: {} frames through the grade", graded.frames);
            let info = serde_json::json!({
                "frames": graded.frames,
                "width": probe.width,
                "height": probe.height,
                "fps": probe.fps,
                "sampled_frames": source.height() / video::sample_size(probe).1,
                "ffmpeg_args": ffmpeg_args,
            });
            (
                graded.first,
                graded.first_graded,
                graded.coverage,
                Some(info),
                file,
            )
        }
    };

    // The full-size PNG and the comparison thumbnails at the same time
    // (a video is written already).
    std::thread::scope(|scope| {
        let wallpaper = scope.spawn(|| {
            if video.is_some() {
                return Ok(());
            }
            remapped_rgb
                .save(out.join("wallpaper.png"))
                .map_err(|e| format!("can't write wallpaper.png: {}", e))
        });
        let compare = write_compare(out, &source, &remapped_rgb);
        wallpaper.join().expect("wallpaper.png thread")?;
        compare
    })?;
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
        if config.dither.is_some() {
            println!(
                "  the lut holds the grade before dithering (a dither depends on position, so no lut can hold it)"
            );
        }
    }

    let coverage_map: BTreeMap<String, u64> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&coverage)
        .map(|(slot, count)| (slot.to_string(), *count))
        .collect();
    // a dither stage changes every pixel even on a passive grade
    let passthrough = passive && config.dither.is_none() && grain.is_none() && !deband;
    let mut report = serde_json::json!({
        "params": config,
        "vivid_chroma": vivid,
        "passthrough": passthrough,
        "slot_coverage": coverage_map,
    });
    if let Some(info) = video_info {
        report["video"] = info;
    }
    if deband {
        report["deband"] = serde_json::json!(true);
    }
    if let Some(amount) = grain {
        report["grain"] = serde_json::json!(amount);
    }
    write_json_file(&out.join("report.json"), &report)?;

    if let Some(strip_hexes) = &arranged_hexes {
        let scheme_name = parsed.meta_name();
        let wall_label = preview::image_label(wallpaper);
        let input = preview::Preview {
            flow: preview::PreviewFlow::WallpaperFromScheme,
            wallpaper_label: Some(&wall_label),
            // the repainted result is a sibling artifact
            wallpaper_src: Some(&wallpaper_file),
            scheme: &scheme_name,
            hexes: strip_hexes,
            left_out: &left_out,
        };
        std::fs::write(out.join("preview.html"), preview::render(&input)?)
            .map_err(|e| format!("can't write preview.html: {}", e))?;
    }

    if passthrough {
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
                    100.0 * *count as f64 / total as f64
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
    if arranged_hexes.is_none() {
        println!("rehue wal: wrote {wallpaper_file}, compare.png, report.json");
    } else {
        println!("rehue wal: wrote {wallpaper_file}, compare.png, preview.html, report.json");
    }
    Ok(())
}

/// A video run's results.
struct GradedVideo {
    frames: u64,
    /// The first frame, before and after.
    first: image::RgbImage,
    first_graded: image::RgbImage,
    coverage: Vec<u64>,
}

/// What happens to a video's frames around the grade.
struct FrameSteps<'a> {
    /// Deband the frames before grading.
    deband: bool,
    /// Grain after grading, in levels.
    grain: Option<f64>,
    /// How the result is encoded.
    encode: &'a video::EncodeArgs,
}

/// Every frame of `input` through `table` (unchanged without one), with
/// `steps` around it, into `output`.
fn grade_video(
    tool: &Path,
    input: &Path,
    output: &Path,
    probe: &video::Probe,
    table: Option<&grade_wal::ColorTable>,
    steps: FrameSteps,
) -> Result<GradedVideo, String> {
    let FrameSteps {
        deband,
        grain,
        encode,
    } = steps;
    let mut decoder = video::Decoder::open(tool, input, probe, deband)?;
    let mut encoder = video::Encoder::open(tool, input, output, probe, encode)?;
    let mut frame = Vec::new();
    let mut graded = vec![0u8; probe.width as usize * probe.height as usize * 3];
    let mut coverage = vec![0u64; BASE_SLOTS.len()];
    let mut frames = 0u64;
    let mut first = None;
    while decoder.next(&mut frame)? {
        match table {
            Some(table) => table.apply(&frame, &mut graded, &mut coverage),
            None => graded.copy_from_slice(&frame),
        }
        if let Some(amount) = grain {
            rehue::grain::apply(&mut graded, probe.width, frames, amount);
        }
        let out: &[u8] = &graded;
        encoder.write(out)?;
        if first.is_none() {
            first = Some((frame.clone(), out.to_vec()));
        }
        frames += 1;
    }
    encoder.finish()?;
    let (before, after) = first.ok_or_else(|| format!("{}: no frames", input.display()))?;
    let image = |px: Vec<u8>| {
        image::RgbImage::from_raw(probe.width, probe.height, px).expect("frame is the probed size")
    };
    Ok(GradedVideo {
        frames,
        first: image(before),
        first_graded: image(after),
        coverage,
    })
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
            let slots: BTreeMap<String, String> = BASE_SLOTS
                .iter()
                .map(|slot| slot.to_string())
                .zip(hexes.iter().cloned())
                .collect();
            render_preview(&[&slots])?
                .save(dir.join("inspect.png"))
                .map_err(|e| format!("can't write inspect.png: {}", e))?;
            println!("rehue inspect: wrote preview.html, inspect.png");
        }
        return Ok(());
    }
    let wallpaper =
        wallpaper.ok_or_else(|| "pass --wallpaper, --scheme or --schemes".to_string())?;
    let config: grade_scheme::SchemeConfig =
        rehue::config::from_value(rehue::config::read_file(config_path)?).map_err(|e| {
            format!(
                "config {}: {e}",
                config_path
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            )
        })?;
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
/// `rehue icons`: a theme (or one icon) recoloured with the wal grade.
fn run_icons(
    input: &str,
    parsed: &Scheme,
    out: &Path,
    name: Option<&str>,
    config: rehue::icons::IconsConfig,
    dry_run: bool,
) -> Result<(), String> {
    use rehue::icons::{self, Input};
    let source = icons::find_input(input)?;
    let mut folder_grade = config.grade.clone();
    if config.folders.is_some() {
        folder_grade.palette = config.folders.clone();
    }
    let regs = grade_wal::resolved_roles(&config.grade)?;
    let folder_regs = grade_wal::resolved_roles(&folder_grade)?;
    if regs.values().any(|r| !matches!(r.assign, AssignState::Off)) {
        println!("rehue icons: assign needs a wallpaper's families; ignored here");
    }
    let slot_hexes = parsed.slot_hexes()?;
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
    let arranged = grade_wal::arrange_palette(&slots, &[], &regs)?;
    let mut neutral_slots: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    if neutral_slots.is_empty() {
        neutral_slots = (0..16).collect();
    }
    let options = grade_wal::slot_options(&regs, &config.grade)?;
    let folder_options = grade_wal::slot_options(&folder_regs, &folder_grade)?;

    // what "fully vivid" means: the theme's own colours
    let colours = match &source {
        Input::Theme(root) => icons::theme_colours(root)?,
        Input::File(path) => match std::fs::read_to_string(path) {
            Ok(text) => icons::svg_colours(&text).into_iter().flatten().collect(),
            Err(_) => Vec::new(),
        },
    };
    let vivid = if colours.is_empty() {
        grade_wal::DEFAULT_VIVID
    } else {
        grade_wal::vivid_chroma(&colours)
    };
    let painter = icons::Painter {
        icons: grade_wal::ColorMap::new(&arranged, &neutral_slots, &options, &config.grade)
            .with_vivid(vivid),
        folders: grade_wal::ColorMap::new(
            &arranged,
            &neutral_slots,
            &folder_options,
            &folder_grade,
        )
        .with_vivid(vivid),
        dither: config.grade.dither.map(|d| icons::IconDither {
            dither: d,
            icon_inks: grade_wal::palette_inks(&arranged, &options),
            folder_inks: grade_wal::palette_inks(&arranged, &folder_options),
        }),
    };
    println!(
        "rehue icons: painting with '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );
    if config.grade.palette.is_some() {
        println!(
            "  {}",
            palette_summary(&grade_wal::palette_weights(&config.grade)?)
        );
    }
    if let Some(d) = &config.grade.dither {
        println!("  dither {}", d.describe());
        println!(
            "  dithering icons is best effort: SVGs become fixed-size PNGs (no crisp scaling), soft edges keep their transparency but not their smoothness, and the theme grows; symbolic icons and icons of unknown size stay SVGs"
        );
    }
    if config.folders.is_some() {
        println!(
            "  folders:   {}",
            palette_summary(&grade_wal::palette_weights(&folder_grade)?)
                .trim_start_matches("palette")
                .trim()
        );
    }

    let background = oklch_to_rgb(&slots[0].lch);
    let accents: Vec<[u8; 3]> = slots[8..].iter().map(|s| oklch_to_rgb(&s.lch)).collect();
    match source {
        Input::File(path) => {
            let file = path
                .file_name()
                .ok_or_else(|| format!("{} has no file name", path.display()))?;
            let mut dest = out.join(file);
            let bytes =
                std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
            let mut f = |rgb: [u8; 3]| painter.icons.map_rgb(&rgb);
            let mut body = if path.extension().is_some_and(|x| x == "png") {
                icons::recolor_png(&bytes, &mut f)?
            } else {
                let text = String::from_utf8(bytes)
                    .map_err(|_| format!("{} is not a text SVG", path.display()))?;
                icons::recolor_svg(&text, &mut f).into_bytes()
            };
            if let Some(d) = &painter.dither {
                // one icon has no theme to give its size: drawn at 48 px
                let size = icons::SINGLE_ICON_SIZE;
                let img = if path.extension().is_some_and(|x| x == "png") {
                    image::load_from_memory(&body)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .to_rgba8()
                } else {
                    icons::rasterize_svg(&body, path.parent(), size)
                        .ok_or_else(|| format!("can't draw {}", path.display()))?
                };
                let mut png = std::io::Cursor::new(Vec::new());
                icons::dither_icon(&img, &d.dither, &d.icon_inks)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                body = png.into_inner();
                dest.set_extension("png");
            }
            if dry_run {
                println!("rehue icons: dry run (nothing written)");
                return Ok(());
            }
            std::fs::create_dir_all(out)
                .map_err(|e| format!("can't create {}: {e}", out.display()))?;
            std::fs::write(&dest, body)
                .map_err(|e| format!("can't write {}: {e}", dest.display()))?;
            let sheet = icons::preview_sheet(
                &[icons::render_icon(&path, 48)],
                &[icons::render_icon(&dest, 48)],
                background,
                &accents,
            );
            sheet
                .save(out.join("preview.png"))
                .map_err(|e| format!("can't write preview.png: {e}"))?;
            println!(
                "rehue icons: wrote {}, preview.png",
                dest.file_name().unwrap_or(file).to_string_lossy()
            );
        }
        Input::Theme(root) => {
            let index = icons::read_index(&root)?;
            let original = index.name.clone().unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "icons".into())
            });
            let theme_name = name.map_or_else(|| format!("{original}-rehue"), String::from);
            let dir_name: String = theme_name
                .chars()
                .map(|c| {
                    if c == '/' || c.is_whitespace() {
                        '-'
                    } else {
                        c
                    }
                })
                .collect();
            let dest = out.join(&dir_name);
            println!("rehue icons: theme '{original}' -> '{theme_name}'");
            if !dry_run && dest.exists() {
                std::fs::remove_dir_all(&dest)
                    .map_err(|e| format!("can't replace {}: {e}", dest.display()))?;
            }
            let stats = icons::recolor_theme(
                &root,
                (!dry_run).then_some(dest.as_path()),
                &theme_name,
                &painter,
            )?;
            println!(
                "rehue icons: {} svg, {} png recolored ({} colors, {} folder icons); {} copied; {} links kept",
                stats.svg, stats.png, stats.colours, stats.folders, stats.copied, stats.links
            );
            if painter.dither.is_some() {
                println!(
                    "  {} icons dithered to png{}",
                    stats.dithered,
                    if stats.dithered == 0 {
                        " (index.theme gives no sizes for its directories)"
                    } else {
                        ""
                    }
                );
            }
            if stats.brought_in > 0 {
                println!(
                    "  {} files came from links leaving the theme and were copied in",
                    stats.brought_in
                );
            }
            if config.folders.is_some() && stats.folders == 0 {
                let from = index
                    .inherits
                    .iter()
                    .filter(|t| *t != "hicolor")
                    .cloned()
                    .collect::<Vec<_>>();
                if from.is_empty() {
                    println!("  no folder icons in this theme");
                } else {
                    println!(
                        "  no folder icons in this theme: it takes them from {} (recolor that theme for the folders)",
                        from.join(", ")
                    );
                }
            }
            if dry_run {
                println!("rehue icons: dry run (nothing written)");
                return Ok(());
            }
            let files = icons::preview_files(&root)?;
            let before: Vec<_> = files
                .iter()
                .map(|rel| icons::render_icon(&root.join(rel), 48))
                .collect();
            let after: Vec<_> = files
                .iter()
                .map(|rel| {
                    // a dithered icon is a png now
                    let at = dest.join(rel);
                    let at = if at.exists() {
                        at
                    } else {
                        at.with_extension("png")
                    };
                    icons::render_icon(&at, 48)
                })
                .collect();
            icons::preview_sheet(&before, &after, background, &accents)
                .save(out.join("preview.png"))
                .map_err(|e| format!("can't write preview.png: {e}"))?;
            let cache = std::process::Command::new("gtk-update-icon-cache")
                .args(["--force", "--quiet"])
                .arg(&dest)
                .status();
            match cache {
                Ok(s) if s.success() => println!("rehue icons: icon cache built"),
                _ => println!(
                    "rehue icons: no gtk-update-icon-cache; the theme works without one (GTK just looks icons up slower)"
                ),
            }
            println!(
                "rehue icons: wrote {}/, preview.png; install with: cp -r {} ~/.local/share/icons/",
                dest.display(),
                dest.display()
            );
        }
    }
    Ok(())
}

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
                WalRun {
                    dry_run,
                    ..WalRun::default()
                },
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
            grade,
            dither,
            assign,
            lut,
            lut_level,
            ffmpeg,
            deband,
            grain,
            ffmpeg_args,
        } => {
            let (mut wal_config, looks) = rehue::look::resolve::<grade_wal::WalConfig>(
                &config_label(config.as_deref()),
                rehue::config::read_file(config.as_deref())?,
                &look,
                None,
                rehue::look::Target::Wal,
            )?;
            print_looks("wal", &looks);
            WalOverrides::from_grade(grade, dither, assign).apply(&mut wal_config)?;
            let parsed = Scheme::resolve(&scheme, scheme_dir.as_deref())?;
            run_wal(
                &input,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                wal_config,
                WalRun {
                    dry_run,
                    lut: lut.map(|format| (format, lut_level)),
                    ffmpeg: ffmpeg.as_deref(),
                    deband,
                    grain,
                    ffmpeg_args: ffmpeg_args.as_deref(),
                },
            )
        }
        Commands::Icons {
            input,
            scheme,
            out,
            name,
            dry_run,
            scheme_dir,
            config,
            look,
            folders,
            dither,
            grade,
        } => {
            let (mut icons_config, looks) = rehue::look::resolve::<rehue::icons::IconsConfig>(
                &config_label(config.as_deref()),
                rehue::config::read_file(config.as_deref())?,
                &look,
                None,
                rehue::look::Target::Icons,
            )?;
            print_looks("icons", &looks);
            WalOverrides::from_grade(grade, dither, Vec::new()).apply(&mut icons_config.grade)?;
            if let Some(text) = folders {
                icons_config.folders =
                    Some(grade_wal::parse_palette(&text).map_err(|e| format!("--folders: {e}"))?);
            }
            let parsed = Scheme::resolve(&scheme, scheme_dir.as_deref())?;
            run_icons(
                &input,
                &parsed,
                out.as_deref().unwrap_or(Path::new("rehue-out")),
                name.as_deref(),
                icons_config,
                dry_run,
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
