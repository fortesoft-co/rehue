//! The CLI's commands, flags and help, as data (the docs render from it).
//!
//! The `rehue` command line, as data: every command, flag and help
//! text.  Lives in the library (not the binary) so the docs generator
//! can render reference tables from the same definitions `--help`
//! prints (`Cli::command()`).
//!
//! The doc comments here are `--help` text first; bracketed examples
//! like `[0,1,2,3]` are values, not rustdoc links.
#![allow(rustdoc::broken_intra_doc_links)]

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
        /// The wallpaper whose hues the scheme takes on: png, jpeg, webp, gif (its first frame), tiff, bmp or qoi.
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
        /// Config file (.json, .toml or .yaml): extraction options,
        /// `min-text-contrast`, `reach` and per-role records.
        ///
        /// e.g. -c scheme.toml
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Start from a look, a saved grade: built-in, or
        /// ~/.config/rehue/looks/NAME.toml (or .yaml, .json).  --config and
        /// flags still win; repeat or comma-list to stack (left to right).
        ///
        /// e.g. -l pop   -l snap,vivid   (rehue inspect --looks lists them)
        #[arg(short, long, value_name = "NAME")]
        look: Vec<String>,
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
        /// The wallpaper to repaint: png, jpeg, webp, gif (its first frame), tiff, bmp or qoi;
        /// or a video (mp4, webm, mkv or mov) through ffmpeg.
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
        /// Config file (.json, .toml or .yaml): extraction options and the
        /// grading options below, globally and per role.
        ///
        /// e.g. -c wal.toml
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Start from a look, a saved grade: built-in, or
        /// ~/.config/rehue/looks/NAME.toml (or .yaml, .json).  --config and
        /// flags still win; repeat or comma-list to stack (left to right).
        ///
        /// e.g. -l pop   -l snap,vivid   (rehue inspect --looks lists them)
        #[arg(short, long, value_name = "NAME")]
        look: Vec<String>,
        #[command(flatten)]
        grade: GradeArgs,
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
        /// Draw the result with the scheme's colors only, as a visible
        /// pattern: the 16-color game look.  Bare = checker.
        ///
        /// STYLE[,key=value...].  Styles: checker (dispersed dots, the
        /// default).  Settings: scale (pattern pixel size, auto = about 640
        /// across), sample (block = each pattern pixel the average of its
        /// block, the default; pixel = each image pixel's own color),
        /// flat (0..1: how much of the area where two colors meet stays
        /// solid, default 0.5; 0 = pattern everywhere, 1 = none), size
        /// (checker's matrix, 2, 4 or 8).
        ///
        /// e.g. --dither   --dither checker,scale=4,flat=0.8
        #[arg(
            long,
            help_heading = "Dithering",
            value_name = "STYLE[,K=V]",
            num_args = 0..=1,
            default_missing_value = "checker"
        )]
        dither: Option<String>,
        /// Also bake the grade into a 3D LUT: `lut.png` (Hald CLUT) or
        /// `lut.cube`.
        ///
        /// Apply it elsewhere: `magick in.jpg lut.png
        /// -hald-clut out.jpg`, ffmpeg's haldclut / lut3d filters, GIMP,
        /// Darktable, Resolve.  With --dither, the LUT holds the grade
        /// before dithering (a dither depends on position, not colour).
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
        /// Path to ffmpeg, for a video wallpaper (-i clip.mp4, .webm,
        /// .mkv or .mov); else $REHUE_FFMPEG / $PATH.
        ///
        /// ffprobe is looked for beside it.  e.g. --ffmpeg ~/bin/ffmpeg
        #[arg(long, help_heading = "Video", value_name = "FFMPEG")]
        ffmpeg: Option<PathBuf>,
        /// Smooth a video's compression banding before grading: ffmpeg's
        /// deband filter, for blocky steps in skies and gradients.
        ///
        /// Video only.  e.g. --deband
        #[arg(long, help_heading = "Banding")]
        deband: bool,
        /// Add fine grain after grading, AMOUNT levels of 255 (1..32,
        /// default 4), to hide what banding is left.
        ///
        /// Monochrome, the same every run, and moving frame to frame on
        /// a video.  Not with --dither.  e.g. --grain 3
        #[arg(
            long,
            help_heading = "Banding",
            value_name = "AMOUNT",
            num_args = 0..=1,
            default_missing_value = "4"
        )]
        grain: Option<f64>,
    },
    /// Recolor an icon theme (or one SVG / PNG icon) with a scheme.
    ///
    /// Every color in the theme goes through the same grade as `wal` (its
    /// flags, looks and configs), with harmonize 1 as the default.  The
    /// recolored theme lands in OUT/NAME, ready to copy to
    /// ~/.local/share/icons; preview.png beside it shows familiar icons
    /// before and after.
    ///
    /// Example: rehue icons -i Papirus-Dark -s gruvbox-light
    ///          rehue icons -i Papirus-Dark -s gruvbox-light --folders base0E
    Icons {
        /// The icon theme: a name found in the usual icon directories
        /// (~/.local/share/icons, /usr/share/icons, ...), a theme directory,
        /// or a single .svg / .png icon.  A '/' makes it a path.
        ///
        /// e.g. -i Papirus-Dark   -i ~/.icons/Tela   -i firefox.svg
        #[arg(short, long, value_name = "THEME")]
        input: String,
        /// The scheme to paint with: a bundled NAME or a path to a .yaml.
        ///
        /// e.g. -s gruvbox-light
        #[arg(short, long)]
        scheme: String,
        /// Output directory (default rehue-out): the theme lands in OUT/NAME.
        ///
        /// e.g. -o icons
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// The new theme's name (index.theme Name=; default THEME-rehue).
        ///
        /// e.g. --name Papirus-Gruvbox
        #[arg(long)]
        name: Option<String>,
        /// Preview only: recolor everything and print the summary, but write
        /// nothing.
        ///
        /// e.g. -n
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Directory that scheme NAMES look up before the bundled set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Config file (.json, .toml or .yaml): the grading options below,
        /// globally and per role, and `folders`.
        ///
        /// e.g. -c icons.toml
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Start from a look, a saved grade: built-in, or
        /// ~/.config/rehue/looks/NAME.toml (or .yaml, .json): its [icons]
        /// section, else its [wal] one.  --config and flags still win; repeat
        /// or comma-list to stack (left to right).
        ///
        /// e.g. -l vivid   -l snap
        #[arg(short, long, value_name = "NAME")]
        look: Vec<String>,
        /// The folder color: which scheme colors folder icons are painted
        /// with, in the --palette form (a slot, a role, weights).  Folders
        /// are the theme's Places icons named folder*, plus user-home,
        /// user-desktop and inode-directory; the rest of the theme is graded
        /// as usual.
        ///
        /// e.g. --folders base0E   --folders base0D,base0C
        #[arg(long, help_heading = "Grading", value_name = "KEY[=W],...")]
        folders: Option<String>,
        /// Dither the icons (best effort): each SVG is drawn at its
        /// directory's size, dithered with the scheme's colors and written
        /// as a PNG.  Bare = checker.
        ///
        /// The same STYLE[,key=value...] as wal --dither.  Symbolic icons
        /// and icons of unknown size stay SVGs; the theme loses crisp
        /// scaling and grows.
        ///
        /// e.g. --dither   --dither diffuse,kernel=atkinson
        #[arg(
            long,
            help_heading = "Dithering",
            value_name = "STYLE[,K=V]",
            num_args = 0..=1,
            default_missing_value = "checker"
        )]
        dither: Option<String>,
        #[command(flatten)]
        grade: GradeArgs,
    },
    /// Show a wallpaper's hue families, a scheme's colors, or every
    /// scheme (`--schemes`) in the terminal.
    ///
    /// Families mode indexes the wallpaper's hue families (the `[i]`
    /// indices that assign configs target).  Scheme mode shows a
    /// scheme's 16 slots as truecolor swatches.  --schemes browses the
    /// whole collection: one row per scheme, name and colours.  --looks
    /// lists the saved grades -l takes.
    ///
    /// Example: rehue inspect --wallpaper butterfly.webp
    ///          rehue inspect --scheme gruvbox-light
    ///          rehue inspect --schemes gruvbox
    ///          rehue inspect --looks
    Inspect {
        /// The image whose hues get extracted (families mode): png, jpeg, webp, gif (its first frame), tiff, bmp or qoi.
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
        /// The looks (saved grades for -l): name, what each grades, and
        /// its description; with a NAME, that look's file.
        ///
        /// e.g. --looks        --looks pop
        #[arg(
            long,
            value_name = "NAME",
            num_args = 0..=1,
            default_missing_value = "",
            conflicts_with_all = ["wallpaper", "scheme", "schemes", "config", "out"]
        )]
        looks: Option<String>,
        /// Directory that scheme NAMES look up before the embedded set.
        ///
        /// e.g. --scheme-dir ~/schemes
        #[arg(long, value_name = "DIR")]
        scheme_dir: Option<PathBuf>,
        /// Config file (.json, .toml or .yaml) with extraction options
        /// (families mode).
        ///
        /// e.g. -c extract.toml
        #[arg(short, long, value_name = "FILE")]
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
    /// Example: rehue enhance -i pic.webp -o up4 --upscale 4
    ///          rehue enhance -i pic.webp -o up4 --upscale 4 --sr
    Enhance {
        /// The picture to scale: png, jpeg, webp, gif (its first frame), tiff, bmp or qoi.
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
        #[arg(long, help_heading = "Upscaling")]
        upscale: u32,
        /// AI super-resolution instead of plain Lanczos.  Needs
        /// realesrgan-ncnn-vulkan: on $PATH, as $REHUE_SR_TOOL, or via
        /// --sr-tool.
        ///
        /// e.g. --sr
        #[arg(long, help_heading = "Upscaling")]
        sr: bool,
        /// SR weights (with --sr):
        /// photos = general x4plus, anime = x4plus-anime.
        ///
        /// e.g. --model photos
        #[arg(long, help_heading = "Upscaling", value_enum, value_name = "MODEL")]
        model: Option<SrModel>,
        /// Path to the SR tool (with --sr); else $REHUE_SR_TOOL / $PATH.
        ///
        /// e.g. --sr-tool ~/tools/realesrgan-ncnn-vulkan
        #[arg(long, help_heading = "Upscaling", value_name = "TOOL")]
        sr_tool: Option<PathBuf>,
    },
    /// Run a pipeline file and apply the result: scheme to tinty,
    /// wallpaper to your command.
    ///
    /// The pipeline is TOML: `wallpaper`, `scheme`, then a
    /// `[rehue.scheme]` and/or `[rehue.wal]` table (the same keys as their
    /// --config files; a table's presence turns its step on) and an
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

  [rehue]
  look = \"pop\"                     # both steps start from a look (optional)

  [rehue.scheme]                   # rehue the scheme (empty = the look / defaults)
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

/// The grading flags `wal` and `icons` share.
#[derive(Debug, Clone, Args)]
pub struct GradeArgs {
    /// How pixels find their palette color: `blend` = mix the nearby
    /// palette colors, weighted by hue distance (default); `snap` = take
    /// the nearest one.
    ///
    /// e.g. --mode snap
    #[arg(long, help_heading = "Grading", value_enum)]
    pub mode: Option<grade_wal::Mode>,
    /// What the scheme's grays (base00-07) do for colorful pixels: `floor`
    /// = only set the lowest chroma in blend, and never catch a color in
    /// snap, so the image keeps its color structure (default); `mix` =
    /// join in like any slot, a calmer cast faded toward the scheme's grays.
    ///
    /// e.g. --neutrals mix
    #[arg(long, help_heading = "Grading", value_enum)]
    pub neutrals: Option<grade_wal::Neutrals>,
    /// What hue gray pixels take on: `keep` their own (default), or
    /// `tint` with the scheme's grays, as far as match-hue goes.
    ///
    /// e.g. --grays tint
    #[arg(long, help_heading = "Grading", value_enum)]
    pub grays: Option<grade_wal::Grays>,
    /// How strongly to harmonize with the scheme, 0..1: sets match-hue and
    /// match-chroma wherever they aren't set (default 0).
    ///
    /// Explicit match dials win; match-lightness never follows it.
    ///
    /// e.g. -H 1
    #[arg(short = 'H', long, help_heading = "Grading", value_name = "0..1")]
    pub harmonize: Option<f64>,
    /// How much of the palette's hue each pixel takes on (0 = keep its
    /// own, 1 = fully adopt).
    ///
    /// e.g. --match-hue 0.9   --match-hue accents 1
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
    pub match_hue: Vec<String>,
    /// How much of the palette's chroma each pixel takes on (0 = keep its
    /// own, 1 = fully adopt).
    ///
    /// e.g. --match-chroma 0.8   --match-chroma bg 0.5
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
    pub match_chroma: Vec<String>,
    /// How much of the palette's lightness each pixel takes on (default 0,
    /// which keeps the image photographic).
    ///
    /// Never set by harmonize: adopting lightness flattens contrast.
    ///
    /// e.g. --match-lightness 0.3   --match-lightness sf 0.1
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] 0..1", num_args = 1..=2, allow_negative_numbers = true)]
    pub match_lightness: Vec<String>,
    /// How far, in hue degrees, a palette color's pull extends (default
    /// 45).
    ///
    /// The falloff width in blend mode; in snap mode, the cutoff beyond
    /// which colors are left alone.
    ///
    /// e.g. --reach 60
    #[arg(long, help_heading = "Grading", value_name = "DEG")]
    pub reach: Option<f64>,
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
    pub palette: Option<String>,
    /// Below this chroma a pixel counts as gray (default 0.02).
    ///
    /// Gray pixels key to the scheme's neutral slots by lightness.
    ///
    /// e.g. --max-gray-chroma 0.05
    #[arg(long, help_heading = "Grading", value_name = "C")]
    pub max_gray_chroma: Option<f64>,
    /// Move each of a role's hues N slots along, wrapping around (slots,
    /// not degrees).
    ///
    /// e.g. --cycle accents 1
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] N", num_args = 1..=2, allow_negative_numbers = true)]
    pub cycle: Vec<String>,
    /// Lightness shift (added).  Bare: the whole-image grade, applied
    /// last; with a role: that role's grade.
    ///
    /// e.g. --lightness 0.02   --lightness bg -0.05
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] L", num_args = 1..=2, allow_negative_numbers = true)]
    pub lightness: Vec<String>,
    /// Chroma scale (multiplied).  Bare: the whole-image grade, applied
    /// last; with a role: that role's grade.
    ///
    /// e.g. --chroma 1.2   --chroma fg 0.9
    #[arg(long, help_heading = "Grading", value_name = "[ROLE] C", num_args = 1..=2, allow_negative_numbers = true)]
    pub chroma: Vec<String>,
}
