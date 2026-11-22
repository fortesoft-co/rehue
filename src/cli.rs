//! The `rehue` command line, as data: every command, flag and help
//! text.  Lives in the library (not the binary) so the docs generator
//! can render reference tables from the same definitions `--help`
//! prints (`Cli::command()`).
//!
//! The doc comments here are `--help` text first; bracketed examples
//! like `[0,1,2,3]` are values, not rustdoc links.
#![allow(rustdoc::broken_intra_doc_links)]

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::enhance::SrModel;
use crate::grade_wal;

#[derive(Parser)]
#[command(
    name = "rehue",
    version,
    about = "Map base16 colour schemes and wallpapers onto each other"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scheme + wallpaper → scheme: adopt the wallpaper's hues, keep the
    /// scheme's structure.
    ///
    /// Rehue a base16 scheme with a wallpaper's hues: hue families are
    /// extracted from the image and adopted slot by slot, while each
    /// slot's lightness, chroma and contrast stay the scheme's own.
    ///
    /// Example: rehue scheme -i rose-pine-dawn -w butterfly.webp -o mapped
    ///
    #[command(after_help = SCHEME_ROLES)]
    Scheme {
        /// The scheme to rehue: a bundled scheme name or a path to a
        /// base16 YAML.
        ///
        /// e.g. -i rose-pine-dawn   -i ~/schemes/mine.yaml
        #[arg(short, long, value_name = "SCHEME")]
        input: String,
        /// The wallpaper whose hues the scheme takes on.
        ///
        /// e.g. -w ~/pictures/butterfly.webp
        #[arg(short, long)]
        wallpaper: PathBuf,
        /// Output directory: scheme.yaml, preview.png, preview.html, clusters.json (absent: `rehue-out`).
        ///
        /// e.g. -o mapped
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Preview only: run everything and print the result, but write
        /// nothing (not even the --out directory).
        ///
        /// e.g. -n
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Directory that scheme NAMES look up before the bundled set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Config JSON: extraction options, `min-text-contrast`, `reach` and per-role records.
        ///
        /// e.g. -c scheme.json
        #[arg(short, long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// How far, in hue degrees, an accent can reach to claim a wallpaper
        /// hue (default 45).
        ///
        /// A wallpaper family claims an accent slot only when its hue sits
        /// within this distance of the slot's hue; widening lets more slots
        /// follow more families.
        ///
        /// e.g. --reach 30
        #[arg(long, help_heading = "Grading", value_name = "DEG")]
        reach: Option<f64>,
        /// Give a role's slots hues taken from the wallpaper's families:
        /// `auto` = spread the heaviest across, `3` = all from family 3,
        /// `0,2,5` = these families across the slots.
        ///
        /// Hue only: the slots keep the scheme's lightness and chroma.  Family
        /// numbers come from `rehue inspect -w`.  Without a role, only a
        /// family index works.
        ///
        /// e.g. --assign accents 0,1,2,3   --assign sf auto
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] VAL", num_args = 1..=2, allow_negative_numbers = true)]
        assign: Vec<String>,
        /// Move each of a role's hues N slots along, wrapping around (slots,
        /// not degrees).
        ///
        /// e.g. --cycle accents 1
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] N", num_args = 1..=2, allow_negative_numbers = true)]
        cycle: Vec<String>,
        /// How much of the wallpaper's hue the slots take on, 0..1 (default 1).
        ///
        /// e.g. --match-hue 0.4   --match-hue accents 1
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
        match_hue: Vec<String>,
        /// Shift a role's lightness (added; -0.1 is darker).
        ///
        /// e.g. --lightness 0.05   --lightness sf -0.02
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] L", num_args = 1..=2, allow_negative_numbers = true)]
        lightness: Vec<String>,
        /// Scale a role's chroma (multiplied; 0.8 muted, 1.3 vivid).
        ///
        /// e.g. --chroma 1.6   --chroma accents 0.75
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] C", num_args = 1..=2, allow_negative_numbers = true)]
        chroma: Vec<String>,
    },
    /// Wallpaper + scheme → wallpaper: repaint the image with the scheme's
    /// palette.
    ///
    /// Repaint an image with a base16 scheme's palette: every pixel
    /// moves toward the palette (hue and chroma by dial; lightness stays
    /// photographic by default).  All options are opt-in; with every
    /// dial at zero the output is a re-encoded passthrough.
    ///
    /// Example: rehue wal -i glitter.webp -s gruvbox-light -o repainted -H 1
    ///
    #[command(visible_alias = "wallpaper", after_help = WAL_ROLES)]
    Wal {
        /// The wallpaper to repaint.
        ///
        /// e.g. -i ~/pictures/glitter.webp
        #[arg(short, long, value_name = "WALLPAPER")]
        input: PathBuf,
        /// The scheme whose palette it takes on: a bundled scheme name or
        /// a path to a base16 YAML.
        ///
        /// e.g. -s gruvbox-light
        #[arg(short, long)]
        scheme: String,
        /// Output directory: wallpaper.png, compare.png, report.json, preview.html (absent: `rehue-out`).
        ///
        /// e.g. -o repainted
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Preview only: run everything and print the result, but write
        /// nothing (not even the --out directory).
        ///
        /// e.g. -n
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Directory that scheme NAMES look up before the bundled set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Config JSON: extraction options and the grading options below, globally and per role.
        ///
        /// e.g. -c wal.json
        #[arg(short, long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// How pixels find their palette color: `blend` = mix the nearby
        /// palette colors, weighted by hue distance (default); `snap` = take
        /// the nearest one.
        ///
        /// e.g. --mode snap
        #[arg(long, help_heading = "Grading", value_enum)]
        mode: Option<grade_wal::Mode>,
        /// How strongly to harmonize with the scheme, 0..1: sets match-hue and
        /// match-chroma wherever they aren't set (default 0).
        ///
        /// Explicit match dials win; match-lightness never follows it.
        ///
        /// e.g. -H 1
        #[arg(short = 'H', long, help_heading = "Grading", value_name = "0..1")]
        harmonize: Option<f64>,
        /// How much of the palette's hue each pixel takes on (0 = keep its
        /// own, 1 = fully adopt).
        ///
        /// e.g. --match-hue 0.9   --match-hue accents 1
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
        match_hue: Vec<String>,
        /// How much of the palette's chroma each pixel takes on (0 = keep its
        /// own, 1 = fully adopt).
        ///
        /// e.g. --match-chroma 0.8   --match-chroma bg 0.5
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
        match_chroma: Vec<String>,
        /// How much of the palette's lightness each pixel takes on (default 0,
        /// which keeps the image photographic).
        ///
        /// Never set by harmonize: adopting lightness flattens contrast.
        ///
        /// e.g. --match-lightness 0.3   --match-lightness sf 0.1
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
        match_lightness: Vec<String>,
        /// How far, in hue degrees, a palette color's pull extends (default
        /// 45).
        ///
        /// The falloff width in blend mode; in snap mode, the cutoff beyond
        /// which colors are left alone.
        ///
        /// e.g. --reach 60
        #[arg(long, help_heading = "Grading", value_name = "DEG")]
        reach: Option<f64>,
        /// Which scheme colors pixels may be pulled toward, and how hard:
        /// slots (base0D), roles (ac) or all, comma-separated, each bare
        /// (weight 1) or KEY=WEIGHT.
        ///
        /// Unlisted slots are left out, and grays stay gray when no neutral
        /// slot is in.  A weight divides the slot's hue distance: weight 2
        /// pulls from twice as far.
        ///
        /// e.g. -p ac   --palette base0D,base0E   --palette all,base0D=6
        #[arg(short, long, help_heading = "Grading", value_name = "KEY[=W],...")]
        palette: Option<String>,
        /// Below this chroma a pixel counts as gray (default 0.02).
        ///
        /// Gray pixels key to the scheme's neutral slots by lightness.
        ///
        /// e.g. --max-gray-chroma 0.05
        #[arg(long, help_heading = "Grading", value_name = "C")]
        max_gray_chroma: Option<f64>,
        /// Dithering strength 0..1 (0 = off).
        ///
        /// Breaks up banding where adoption is partial; inert at zero and at
        /// full adoption, where the target swallows the residual.
        ///
        /// e.g. --dithering 1
        #[arg(long, help_heading = "Grading", value_name = "0..1")]
        dithering: Option<f64>,
        /// Dithering pattern (default blue-noise).
        ///
        /// e.g. --dithering-mode floyd-steinberg
        #[arg(long, help_heading = "Grading", value_enum)]
        dithering_mode: Option<grade_wal::DitherMode>,
        /// Before painting, give a role's slots hues taken from the wallpaper's families:
        /// `auto` = spread the heaviest across, `3` = all from family 3,
        /// `0,2,5` = these families across the slots.
        ///
        /// Hue only: the slots keep the scheme's lightness and chroma.  Family
        /// numbers come from `rehue inspect -w`.  Without a role, only a
        /// family index works.
        ///
        /// e.g. --assign accents 0,1,2,3   --assign sf auto
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] VAL", num_args = 1..=2, allow_negative_numbers = true)]
        assign: Vec<String>,
        /// Move each of a role's hues N slots along, wrapping around (slots,
        /// not degrees).
        ///
        /// e.g. --cycle accents 1
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] N", num_args = 1..=2, allow_negative_numbers = true)]
        cycle: Vec<String>,
        /// Lightness shift (added).  Bare: the whole-image grade, applied
        /// last; with a role: that role's grade.
        ///
        /// e.g. --lightness 0.02   --lightness bg -0.05
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] L", num_args = 1..=2, allow_negative_numbers = true)]
        lightness: Vec<String>,
        /// Chroma scale (multiplied).  Bare: the whole-image grade, applied
        /// last; with a role: that role's grade.
        ///
        /// e.g. --chroma 1.2   --chroma fg 0.9
        #[arg(long, help_heading = "Grading", value_name = "[ROLE] C", num_args = 1..=2, allow_negative_numbers = true)]
        chroma: Vec<String>,
        /// Also bake the grade into a 3D LUT: `lut.png` (Hald CLUT) or
        /// `lut.cube`.
        ///
        /// Apply it elsewhere: `magick in.jpg lut.png
        /// -hald-clut out.jpg`, ffmpeg's haldclut / lut3d filters, GIMP,
        /// Darktable, Resolve.  Needs dithering 0 (a dithered pixel
        /// depends on its position, not just its colour).
        ///
        /// e.g. --lut cube
        #[arg(long, help_heading = "LUT export", value_enum, value_name = "FORMAT")]
        lut: Option<LutFormat>,
        /// LUT resolution as a Hald level, 2..16 (16 = every 8-bit color,
        /// exact; `.cube` goes up to 8).
        ///
        /// Level L samples L² values per channel (8 = 64³ entries).
        ///
        /// e.g. --lut-level 12
        #[arg(
            long,
            help_heading = "LUT export",
            value_name = "2..16",
            default_value_t = 8
        )]
        lut_level: u32,
    },
    /// Show a wallpaper's hue families, a scheme's colors, or every
    /// scheme (`--schemes`) in the terminal.
    ///
    /// Families mode indexes the wallpaper's hue families (the `[i]`
    /// indices that assign configs target).  Scheme mode shows a
    /// scheme's 16 slots as truecolor swatches.  --schemes browses the
    /// whole collection: one row per scheme, name and colours.
    ///
    /// Example: rehue inspect --wallpaper butterfly.webp
    ///          rehue inspect --scheme gruvbox-light
    ///          rehue inspect --schemes gruvbox
    Inspect {
        /// The image whose hues get extracted (families mode).
        ///
        /// e.g. -w ~/pictures/butterfly.webp
        #[arg(short, long)]
        wallpaper: Option<PathBuf>,
        /// A scheme (NAME or path) whose slots print as truecolor
        /// swatches (scheme mode).
        ///
        /// e.g. -s gruvbox-light
        #[arg(short, long)]
        scheme: Option<String>,
        /// Every available scheme, one row each: name, then its 16 slots
        /// as swatches.  An optional FILTER keeps names containing it
        /// (case-insensitive).  Schemes in --scheme-dir are included.
        ///
        /// e.g. --schemes        --schemes gruvbox
        #[arg(
            long,
            value_name = "FILTER",
            num_args = 0..=1,
            default_missing_value = "",
            conflicts_with_all = ["wallpaper", "scheme", "config", "out"]
        )]
        schemes: Option<String>,
        /// Directory that scheme NAMES look up before the embedded set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Extraction options as JSON (families mode).
        ///
        /// e.g. -c extract.json
        #[arg(short, long, value_name = "JSON")]
        config: Option<PathBuf>,
        /// Output directory: families mode = inspect.png, scheme mode =
        /// preview.html; absent prints the strips only.
        ///
        /// e.g. -o inspect
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Upscale a wallpaper: built-in Lanczos, or Vulkan AI
    /// super-resolution.
    ///
    /// enhance works on the SOURCE picture; the palette transform stays
    /// in wal.  Lanczos is deterministic and dependency-free.
    /// --sr shells out to realesrgan-ncnn-vulkan (ncnn + vulkan: no
    /// CUDA, works on intel/amd/nvidia/software drivers); the model
    /// runs at its native 4x and the target factor is reached with the
    /// same Lanczos resampler.
    ///
    /// Example: rehue enhance --wallpaper pic.webp --out up4 --upscale 4
    ///          rehue enhance --wallpaper pic.webp --out up4 --upscale 4 --sr
    Enhance {
        /// The picture to scale.
        ///
        /// e.g. -i ~/pictures/pic.webp
        #[arg(short, long, value_name = "WALLPAPER")]
        input: PathBuf,
        /// Directory for the scaled PNG, named after the source's stem
        /// (absent: `rehue-out`).
        ///
        /// e.g. -o up4
        #[arg(short, long)]
        out: Option<PathBuf>,
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
    /// Run a pipeline file and apply the result: scheme to tinty,
    /// wallpaper to your command.
    ///
    /// The pipeline is TOML: `wallpaper`, `scheme`, then a
    /// `[rehue.scheme]` and/or `[rehue.wal]` table (the same keys as their
    /// JSON --config files; a table's presence turns its step on) and an
    /// `[apply]` table.  The mapped scheme is installed as a tinty custom scheme
    /// and applied with `tinty apply base16-<tinty-name>`; the wallpaper
    /// command runs with `{wallpaper}` filled in.
    ///
    /// Example: rehue apply --pipeline ~/.config/rehue/pipeline.toml
    #[command(after_help = APPLY_PIPELINE)]
    Apply {
        /// The pipeline file.
        ///
        /// e.g. --pipeline ~/.config/rehue/pipeline.toml
        #[arg(long)]
        pipeline: PathBuf,
        /// Preview only: print the previews and what would run; write
        /// nothing, run nothing.
        ///
        /// e.g. -n
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}

const APPLY_PIPELINE: &str = "
Pipeline example:
  wallpaper = \"~/pictures/glitter.webp\"
  scheme = \"gruvbox-light\"        # a bundled name, or a path to yaml
  # out = \"~/.local/share/rehue/current\"   (the default)

  [rehue.scheme]                   # rehue the scheme (empty = defaults)
  [rehue.scheme.roles.accents]
  chroma = 1.2

  [rehue.wal]                      # repaint the wallpaper with it
  harmonize = 1.0

  [apply]
  tinty = true                     # default; tinty-name = \"rehue\"
  wallpaper-command = \"swww img {wallpaper}\"
  # tinty-config / tinty-data-dir: point tinty somewhere else (testing)
";

/// The `--lut` export formats.
#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
pub enum LutFormat {
    /// Hald CLUT image (`lut.png`).
    Hald,
    /// Adobe/Resolve 3D LUT text (`lut.cube`).
    Cube,
}

/// The roles paragraph both grading commands print after `--help`.
macro_rules! roles_help {
    ($tail:literal) => {
        concat!(
            "
Roles:
  The 16 slots fall into four roles: background (bg, base00),
  surfaces (sf, base01-03), foreground (fg, base04-07) and
  accents (ac, base08-0F). Full names and short forms both work.
",
            $tail
        )
    };
}

const SCHEME_ROLES: &str = roles_help!(
    "
  Per-role options: assign, cycle, match-hue, lightness, chroma.
  Pass `--option ROLE value`, repeatable per role; a bare
  `--option value` sets `all`, and per-role `--config` records
  outrank it.
"
);

const WAL_ROLES: &str = roles_help!(
    "
  Per-role options: match-hue, match-chroma, match-lightness, assign,
  cycle, lightness, chroma. Pass `--option ROLE value`, repeatable per
  role; a bare `--option value` is the global setting, except bare
  --lightness / --chroma, which grade the whole image last. Per-role
  `--config` records outrank the global setting.
"
);
