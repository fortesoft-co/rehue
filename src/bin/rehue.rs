//! `rehue` CLI: the mapping flows, plus `inspect` and `enhance` utilities.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use rehue::color::{Lch, hex_to_oklch, hex_to_rgb, oklch_to_hex, oklch_to_rgb, self_test};
use rehue::enhance::{EnhanceConfig, SrModel, resolve_sr_tool};
use rehue::register::DistributionState;
use rehue::scheme::{BASE_SLOTS, Scheme};
use rehue::terminal::{families_strip, scheme_strip};
use rehue::{extract, map_wal, preview, register};

#[derive(Parser)]
#[command(
    name = "rehue",
    version,
    about = "Map base16 colour schemes and wallpapers onto each other"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Map a reference scheme's structure onto a wallpaper's hues.
    ///
    /// Map a base16 scheme with a wallpaper's hues: hue families are
    /// extracted from the image and adopted slot by slot, while each
    /// slot's lightness, chroma and contrast are the reference scheme's.
    ///
    /// Example: rehue map-scheme --wallpaper butterfly.webp --scheme rose-pine-dawn.yaml --out mapped
    ///
    #[command(after_help = MAP_SCHEME_REGISTERS)]
    MapScheme {
        /// The image whose hues get adopted.
        ///
        /// e.g. --wallpaper ~/pictures/butterfly.webp
        #[arg(long)]
        wallpaper: PathBuf,
        /// A base16 (tinted-scheme) YAML path or a scheme name from the
        /// embedded collection (pass `.`-free names to use it).
        ///
        /// e.g. --scheme rose-pine-dawn.yaml
        #[arg(long)]
        scheme: String,
        /// Output directory: scheme.yaml + preview.png + preview.html +
        /// clusters.json.
        ///
        /// e.g. --out mapped
        #[arg(long)]
        out: PathBuf,
        /// Run the pipeline and print the ANSI preview but write
        /// nothing (and not even --out itself).
        ///
        /// e.g. --dry-run
        #[arg(long)]
        dry_run: bool,
        /// Directory that scheme NAMES look up before the embedded set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Map config JSON: extraction options + per-register records.
        ///
        /// e.g. --config roundtrip.json
        #[arg(long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// Accent claim gate in hue degrees (default 45): how far a
        /// wallpaper family may sit from an accent slot's hue and still
        /// claim it.
        ///
        /// e.g. --reach-deg 30
        #[arg(long, value_name = "DEG")]
        reach_deg: Option<f64>,
        /// Hue-family adoption.  VAL: `true` = ramp, `3` = pin,
        /// `[0,1,2,3]` = stops.  Untargeted: pin only (bg takes pins
        /// only).
        ///
        /// e.g. --distribution accents '[0,1,2,3]'
        #[arg(long, value_name = "[REG] VAL", num_args = 1..=2)]
        distribution: Vec<String>,
        /// Rotate the register's hues across its slots (1 = one right,
        /// wraps).
        ///
        /// e.g. --rotate accents 1
        #[arg(long, value_name = "[REG] N", num_args = 1..=2)]
        rotate: Vec<String>,
        /// Hue adoption from the wallpaper, 0..1 (default 1).
        ///
        /// e.g. --blend-hue 0.4   --blend-hue accents 1
        #[arg(long, value_name = "[REG] 0..1", num_args = 1..=2)]
        blend_hue: Vec<String>,
        /// Additive lightness shift for the register's slots.
        ///
        /// e.g. --light 0.05   --light surfaces -0.02
        #[arg(long, value_name = "[REG] L", num_args = 1..=2)]
        light: Vec<String>,
        /// Multiplicative chroma ratio (0.8 muted, 1.3 vivid).
        ///
        /// e.g. --chroma 1.6   --chroma accents 0.75
        #[arg(long, value_name = "[REG] C", num_args = 1..=2)]
        chroma: Vec<String>,
    },
    /// Map a scheme's palette onto a wallpaper's colours (all options opt-in).
    ///
    /// Repaint an image with a base16 scheme's palette: every pixel
    /// moves toward its nearest palette slot — hue and chroma by dial,
    /// lightness stays photographic by default.  All options are opt-in;
    /// with all-zero dials the output is a re-encoded passthrough.
    ///
    /// Example: rehue map-wal --wallpaper glitter.webp --scheme gruvbox-light.yaml --out repainted --harmonize 1
    ///
    #[command(after_help = MAP_WAL_REGISTERS)]
    MapWal {
        /// The image to repaint.
        ///
        /// e.g. --wallpaper ~/pictures/glitter.webp
        #[arg(long)]
        wallpaper: PathBuf,
        /// A base16 (tinted-scheme) YAML path or a scheme name from the
        /// embedded collection (pass `.`-free names to use it).
        ///
        /// e.g. --scheme gruvbox-light.yaml
        #[arg(long)]
        scheme: String,
        /// Output directory: wallpaper.png + compare.png + preview.html
        /// + report.json (preview.html whenever an arrangement is
        /// live).
        ///
        /// e.g. --out repainted
        #[arg(long)]
        out: PathBuf,
        /// Run the pipeline and print the ANSI preview but write
        /// nothing (and not even --out itself).
        ///
        /// e.g. --dry-run
        #[arg(long)]
        dry_run: bool,
        /// Directory that scheme NAMES look up before the embedded set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Remap config JSON: extraction options + per-register records
        /// (records beat the bare-flag seeds).
        ///
        /// e.g. --config remap.json
        #[arg(long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// How pixels see the palette: `soft` = smooth influence field
        /// (default), `hard` = flat-constant snapping; possible values
        /// describe both.
        ///
        /// e.g. --territory hard
        #[arg(long, value_enum)]
        territory: Option<map_wal::Territory>,
        /// One-dial facade 0..1: seeds blend-hue + blend-chroma where
        /// unset; explicit dials win.
        ///
        /// e.g. --harmonize 1
        #[arg(long, value_name = "0..1")]
        harmonize: Option<f64>,
        /// Hue movement toward the palette (0 = raw pixel hue, 1 = full
        /// snap).
        ///
        /// e.g. --blend-hue 0.9   --blend-hue accents 1
        #[arg(long, value_name = "[REG] 0..1", num_args = 1..=2)]
        blend_hue: Vec<String>,
        /// Chroma movement toward the palette (0 = raw, 1 = adopt the
        /// slot's chroma).
        ///
        /// e.g. --blend-chroma 0.8   --blend-chroma bg 0.5
        #[arg(long, value_name = "[REG] 0..1", num_args = 1..=2)]
        blend_chroma: Vec<String>,
        /// Lightness movement toward the palette.  Default 0 —
        /// photographic, and never facade-seeded.
        ///
        /// e.g. --blend-light 0.3   --blend-light surfaces 0.1
        #[arg(long, value_name = "[REG] 0..1", num_args = 1..=2)]
        blend_light: Vec<String>,
        /// Influence reach on the hue wheel in degrees (default 45):
        /// the falloff width in soft territory, the cutoff in hard.
        ///
        /// e.g. --reach-deg 60
        #[arg(long, value_name = "DEG")]
        reach_deg: Option<f64>,
        /// Chroma below which pixels count as achromatic (default
        /// 0.02); they key to the scheme's neutrals by lightness.
        ///
        /// e.g. --gray-chroma-floor 0.05
        #[arg(long, value_name = "C")]
        gray_chroma_floor: Option<f64>,
        /// Dithering strength 0..1 (0 off; inert at zero and at full
        /// adoption, where the target swallows the residual).
        ///
        /// e.g. --dithering 1
        #[arg(long, value_name = "0..1")]
        dithering: Option<f64>,
        /// Dither kernel (default blue-noise; variants described
        /// below).
        ///
        /// e.g. --dithering-mode floyd-steinberg
        #[arg(long, value_enum)]
        dithering_mode: Option<map_wal::DitherMode>,
        /// Hue-family adoption.  VAL: `3` = pin, `true` = ramp,
        /// `[0,1,2,3]` = stops.  Untargeted: pin only (bg takes pins
        /// only).
        ///
        /// e.g. --distribution accents '[0,1,2,3]'
        #[arg(long, value_name = "[REG] VAL", num_args = 1..=2)]
        distribution: Vec<String>,
        /// Rotate the register's hues across its slots (1 = one right,
        /// wraps).
        ///
        /// e.g. --rotate accents 1
        #[arg(long, value_name = "[REG] N", num_args = 1..=2)]
        rotate: Vec<String>,
        /// Additive lightness.  Bare = image-wide grade (applied last);
        /// targeted = that register's tonal grade.
        ///
        /// e.g. --light 0.02   --light bg -0.05
        #[arg(long, value_name = "[REG] L", num_args = 1..=2)]
        light: Vec<String>,
        /// Multiplicative chroma.  Bare = image-wide grade; targeted =
        /// the register's grade.
        ///
        /// e.g. --chroma 1.2   --chroma fg 0.9
        #[arg(long, value_name = "[REG] C", num_args = 1..=2)]
        chroma: Vec<String>,
    },
    /// Print a wallpaper's colour families or a scheme's slots, rendered
    /// in the terminal; the PNG strip is opt-in.
    ///
    /// Families mode indexes the wallpaper's hue families (the `[i]`
    /// indices that distribution configs target).  Scheme mode shows a
    /// scheme's 16 slots as truecolor swatches.
    ///
    /// Example: rehue inspect --wallpaper butterfly.webp
    ///          rehue inspect --scheme gruvbox-light
    Inspect {
        /// The image whose hues get extracted (families mode).
        ///
        /// e.g. --wallpaper ~/pictures/butterfly.webp
        #[arg(long)]
        wallpaper: Option<PathBuf>,
        /// A scheme (NAME or path) whose slots print as truecolor
        /// swatches (scheme mode).
        ///
        /// e.g. --scheme gruvbox-light
        #[arg(long)]
        scheme: Option<String>,
        /// Directory that scheme NAMES look up before the embedded set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Extraction options as JSON.  (families mode)
        ///
        /// e.g. --config extract.json
        #[arg(long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// Output directory: families mode = inspect.png, scheme mode =
        /// preview.html; absent prints the strips only.
        ///
        /// e.g. --out inspect
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Scale a wallpaper up: built-in Lanczos, or AI SR over vulkan.
    ///
    /// enhance works on the SOURCE picture; the palette transform stays
    /// in map-wal.  Lanczos is deterministic and dependency-free.
    /// --sr shells out to realesrgan-ncnn-vulkan (ncnn + vulkan: no
    /// CUDA, works on intel/amd/nvidia/software drivers); the model
    /// runs at its native 4x and the target factor is reached with the
    /// same Lanczos resampler.
    ///
    /// Example: rehue enhance --wallpaper pic.webp --out up4 --upscale 4
    ///          rehue enhance --wallpaper pic.webp --out up4 --upscale 4 --sr
    Enhance {
        /// The picture to scale (input file).
        ///
        /// e.g. --wallpaper ~/pictures/pic.webp
        #[arg(long)]
        wallpaper: PathBuf,
        /// Directory for the scaled PNG, named after the source's stem.
        ///
        /// e.g. --out up4
        #[arg(long)]
        out: PathBuf,
        /// Target scale factor vs the source.  Lanczos takes 2..=8;
        /// SR weights are native 4x, so --sr takes 1..=4 (1 = same-size
        /// restoration).
        ///
        /// e.g. --upscale 4
        #[arg(long)]
        upscale: u32,
        /// AI super-resolution instead of plain Lanczos.  Needs
        /// realesrgan-ncnn-vulkan: on $PATH, as $REHUE_SR_TOOL, or via
        /// --sr-tool.
        ///
        /// e.g. --sr
        #[arg(long)]
        sr: bool,
        /// SR weights (with --sr):
        /// photos = general x4plus, anime = x4plus-anime.
        ///
        /// e.g. --model photos
        #[arg(long, value_enum, value_name = "MODEL")]
        model: Option<SrModel>,
        /// Path to the SR tool (with --sr); else $REHUE_SR_TOOL / $PATH.
        ///
        /// e.g. --sr-tool ~/tools/realesrgan-ncnn-vulkan
        #[arg(long, value_name = "TOOL")]
        sr_tool: Option<PathBuf>,
    },
    /// List the scheme names embedded in the binary (base16 collection).
    Schemes,
}

const MAP_SCHEME_REGISTERS: &str = "
Registers:
  The palette is grouped into four registers: bg (base00),
  surfaces (base01-03), fg (base04-07), accents (base08-0F) - the
  surface groups a desktop theme lives in.

  Register-targetable options: distribution, rotate, blend-hue, light,
  chroma.  Pass `--dial register value`, repeatable per register; a
  bare `--dial value` seeds `all`, and per-register `--config` records
  outrank the seeds.
";

const MAP_WAL_REGISTERS: &str = "
Registers:
  The palette is grouped into four registers: bg (base00),
  surfaces (base01-03), fg (base04-07), accents (base08-0F) - the
  surface groups a desktop theme lives in.

  Register-targetable options: blend-hue, blend-chroma, blend-light,
  distribution, rotate, light, chroma.  Pass `--dial register value`,
  repeatable per register; a bare `--dial value` is the global seed —
  except bare --light/--chroma, which stay the image-wide grade applied
  last.  Per-register `--config` records outrank the seeds.
";

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

fn run_map_scheme(
    wallpaper: &Path,
    scheme: &str,
    scheme_dir: Option<&Path>,
    out: &Path,
    config_path: Option<&Path>,
    dry_run: bool,
    overrides: SchemeOverrides,
) -> Result<(), String> {
    let mut config = read_json::<register::MapConfig>(config_path)?;
    overrides.apply(&mut config)?;
    let parsed = Scheme::resolve(scheme, scheme_dir)?;
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

    if dry_run {
        println!("rehue map-scheme: dry run (nothing written)");
        return Ok(());
    }

    std::fs::create_dir_all(out).map_err(|e| format!("can't create {}: {}", out.display(), e))?;
    std::fs::write(out.join("scheme.yaml"), parsed.render(&mapped))
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
    std::fs::write(
        out.join("preview.html"),
        preview::render(&parsed.meta_name(), &mapped_hexes)?,
    )
    .map_err(|e| format!("can't write preview.html: {}", e))?;

    let report = serde_json::json!({
        "params": config,
        "grayscale_wallpaper": clusters.is_empty(),
        "clusters": clusters,
        "dominant_hue": clusters.first().map(|c| c.hue),
    });
    write_json_file(&out.join("clusters.json"), &report)?;

    println!("rehue map-scheme: wrote scheme.yaml, preview.png, preview.html, clusters.json");
    Ok(())
}

/// Extraction, only when an arrangement dial is live (keeps plain blend
/// runs quiet); shared by the run and its dry-run preview.
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
    scheme: &str,
    scheme_dir: Option<&Path>,
    out: &Path,
    config_path: Option<&Path>,
    dry_run: bool,
    overrides: WalOverrides,
) -> Result<(), String> {
    let mut config = read_json::<map_wal::RemapConfig>(config_path)?;
    overrides.apply(&mut config)?;
    let regs = map_wal::resolved_registers(&config)?;
    let parsed = Scheme::resolve(scheme, scheme_dir)?;
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
        std::fs::write(
            out.join("preview.html"),
            preview::render(&parsed.meta_name(), strip_hexes)?,
        )
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
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
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
            std::fs::write(
                dir.join("preview.html"),
                preview::render(&parsed.meta_name(), &hexes)?,
            )
            .map_err(|e| format!("can't write preview.html: {}", e))?;
            println!("rehue inspect: wrote preview.html");
        }
        return Ok(());
    }
    let wallpaper = wallpaper.expect("one of --scheme/--wallpaper");
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

/// The embedded scheme collection, printed.

fn run_schemes() -> Result<(), String> {
    let names: Vec<&'static str> = rehue::scheme::collection_names().collect();
    println!("rehue schemes: {} base16 scheme(s) embedded", names.len());
    for name in names {
        println!("  {name}");
    }
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
        } => run_map_scheme(
            &wallpaper,
            &scheme,
            scheme_dir.as_deref(),
            &out,
            config.as_deref(),
            dry_run,
            SchemeOverrides {
                reach_deg,
                distribution,
                rotate,
                blend_hue,
                light,
                chroma,
            },
        ),
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
        } => run_map_wal(
            &wallpaper,
            &scheme,
            scheme_dir.as_deref(),
            &out,
            config.as_deref(),
            dry_run,
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
            },
        ),
        Commands::Inspect {
            wallpaper,
            scheme,
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
            &out,
            upscale,
            sr,
            model.unwrap_or(SrModel::Photos),
            sr_tool.as_deref(),
        ),
        Commands::Schemes => run_schemes(),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli.command) {
        eprintln!("rehue: {}", e);
        std::process::exit(1);
    }
}
