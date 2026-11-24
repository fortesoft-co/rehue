# Changelog

All notable changes to rehue. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rehue
uses [semantic versioning](https://semver.org/) (pre-1.0: minor
versions may break).

## [0.5.0] - 2026-11-24

The video release: `wal` takes a video wallpaper and grades every
frame through ffmpeg, with deband and grain for the banding a grade
shows, dithering that holds still, your own encoding flags, and
`--loop`, which cuts a clip so its end flows back into its start.
Every setting now has a config key, so the Nix builders can make a
looped video wallpaper too, and `wal` runs across all cores.

### Added

- Video wallpapers: `wal -i clip.mp4` (or `.webm`, `.mkv`, `.mov`)
  grades every frame, through ffmpeg found like `enhance`'s SR tool
  (`--ffmpeg`, `$REHUE_FFMPEG`, then `PATH`). The grade is measured on
  32 frames across the video, so it holds steady from start to end;
  the output keeps the input's container (H.264, or VP9 for WebM) and
  its audio. `compare.png` shows the first frame, `report.json` counts
  every frame, and `preview.html` plays it. `docs/video.md` covers it
  all.
- `wal --deband` (video) and `wal --grain [AMOUNT]`, for banding a
  grade makes visible: deband runs ffmpeg's deband filter on the frames
  before grading; grain adds fine monochrome grain after it (4 levels by
  default), the same every run and moving frame to frame on a video.
  The video docs say which to try for what.
- `wal --ffmpeg-args "..."`: your own ffmpeg encoding flags for a
  video, in place of rehue's codec settings (HEVC, AV1, `-pix_fmt
  yuv444p`, audio flags, ...). Flags that would change the input,
  mapping, filters, length or container are refused.
- `wal --dither` on a video: every frame drawn with the scheme's
  colors. Checker, halftone and pattern hold still while the picture
  moves; diffuse runs with a warning that it shimmers. A dithered video
  is encoded in 4:4:4, so the dots keep their colors: H.264 (VP9 for
  WebM), or AV1 if ffmpeg lacks those; `--ffmpeg-args` picks something
  else. The docs show a plant in the rain in halftone, the encodings
  to pick from for size, speed and playback, and what to do on a
  laptop.
- A video's frame count, speed and time left show on the terminal
  while it's graded.
- `wal --loop [MODE][,key=value...]`: a video cut so its end flows
  back into its start. rehue scans the clip at 160x90, scores every
  join (the OKLab difference between the frames a fade blends, against
  the clip's own motion), lists the best at a quarter, half and
  three-quarters of the clip, and takes one: `auto` (the best fade, or
  a trim when the clip already loops), `ask` (pick from the list),
  `fade`, `bridge` (ffmpeg's `minterpolate`) or `trim`. `seam`, `min`,
  `start` and `end` settle the rest, and the printed flag makes the
  same loop again. The grade is measured on the frames the loop plays,
  the sound is crossfaded with the picture, and `report.json` records
  the join and every candidate. The video docs show a city skyline at
  night as shot, jumping where it wraps, and looped in three of
  tokyo-night-storm's colors.
- Every run setting has a config key: `wal`'s `lut`, `lut-level`,
  `deband`, `grain`, `ffmpeg`, `ffmpeg-args` and `loop` (its string or
  a table), `scheme-dir` on `scheme`, `wal` and `icons`, and `enhance`
  takes `-c` with `upscale`, `sr`, `model` and `sr-tool`. Paths are
  taken from the config's directory; flags still win. The `apply`
  pipeline's `[rehue.wal]` takes them too, and sets a video wallpaper
  when its wallpaper is one.
- Nix: `lib.wal` builds a video wallpaper (with ffmpeg) and every
  setting above through `config`; `lib.enhance` scales a picture up
  (Lanczos). `docs/nix.md` lists every builder, what it takes and what
  it gives.
- `scheme -w` and `inspect -w` take a video too, its hue families
  measured on frames across it. `inspect -s NAME -o DIR` also writes
  `inspect.png`, the scheme's 16 slots as swatches. The video docs show
  a skyline from dusk to night before and after, with its hue families
  and the scheme.
- `--grays keep|tint` (config `grays`): with `tint`, gray pixels take
  the hue of the scheme's grays as they take on its chroma, so whites
  go cream or lavender with the scheme and a dark sky takes the
  background's tint instead of its own murky one. `keep` is the
  default.
- GIF, TIFF, BMP and QOI inputs alongside PNG, JPEG and WebP, for
  `wal`, `scheme`, `inspect` and `enhance`. A GIF paints its first
  frame. `scheme`'s `preview.html` leaves the wallpaper panel out for
  TIFF and QOI, which browsers don't show.

### Changed

- Near-gray pixels blend smoothly from the gray treatment to the color
  one, between `max-gray-chroma` and twice it, instead of switching at
  the line: a night sky or a soft glow no longer splits into
  hard-edged patches of accent color. Most `wal` and `icons` output
  changes slightly.
- `wal --lut`'s docs: with `--dither`, the LUT holds the grade before
  dithering (they said the two don't go together).
- `wal` runs across all cores, and the per-pixel grade does less
  work: an 8K (33 MP) wallpaper paints in about 1.5 s instead of 25 s,
  a 4K one in 0.4 s. Dithering runs across cores too (diffuse's error
  pass stays in order), as do the comparison thumbnails. The output is
  the same bytes as before, whatever the thread count;
  `REHUE_THREADS=N` pins it.

## [0.4.0] - 2026-11-22

The dithering release: `wal --dither` draws a wallpaper with the
scheme's colors only, in checker, halftone, pattern or diffuse styles,
with four new looks built on it, and `icons --dither` does the same for
icon themes. Looks can also be written in YAML or JSON.

### Added

- `wal --dither [STYLE,key=value...]`: draw the result with the
  scheme's colors only, as a visible pattern. It runs last, after any
  grade; every pixel of the result is a scheme color, so
  `report.json`'s coverage is exact. Styles: `checker` (the default,
  dispersed dots from a 2x2, 4x4 or 8x8 Bayer grid: 16-color games),
  `halftone` (round dots on an angled grid: print), `pattern` (drawn
  tiles, `hatch` or `lines`) and `diffuse` (error diffusion,
  Floyd-Steinberg or Atkinson). Settings `scale` (pattern pixel size,
  `auto` = about 640 across), `sample` (`block` averages each pattern
  pixel's block, `pixel` keeps edges sharp), `flat` (how much stays
  solid), and each style's own. In configs, `dither = "checker"` or a
  `[dither]` table. With `--lut`, the LUT holds the grade before
  dithering.
- `icons --dither` (best effort): each icon in a folder whose size
  `index.theme` gives is drawn at that size, dithered with the scheme's
  colors (folders with the `--folders` ones) and written as a PNG in
  its place, links re-pointed, transparency kept; symbolic icons and
  icons of unknown size stay SVGs. A single icon is drawn at 48 px. A
  look's `[wal]` dither still doesn't carry over to icons; an `[icons]`
  section's does.
- Looks `pc98` (harmonize, then checker), `newsprint` (two inks in
  halftone), `onebit` (darkest and lightest in Atkinson) and `hatch`.
- Docs pages for dithering and `enhance`; enhance's flags sit under an
  Upscaling heading in `--help`.
- Looks in YAML or JSON as well as TOML, like configs: a user look is
  `NAME.toml`, `NAME.yaml`, `NAME.yml` or `NAME.json`, and a look given
  by path is read by its extension. Two files for one name are an
  error rather than a silent pick.

### Changed

- The bundled base24 schemes are gone; nothing read them, and the
  binary is about 1 MB smaller.

### Changed (breaking)

- `--dithering` and `--dithering-mode` are gone, replaced by
  `--dither`. They varied how far each pixel moved toward its color
  rather than choosing between palette colors, so they never gave a
  visible dither; `--dither` does.

## [0.3.0] - 2026-11-22

The vocabulary release: commands, flags and config keys are renamed to
fit the domain. There are no compatibility aliases; old names are
rejected (pre-1.0, see the table below). It also brings looks, icon
themes (`rehue icons`), the wal palette, and TOML and YAML configs.

### Added

- `wal --lut hald|cube [--lut-level 2..16]`: bake the grade into a
  3D LUT (`lut.png` Hald CLUT or `lut.cube`) for ImageMagick, ffmpeg,
  GIMP, Darktable or Resolve. At level 16 ImageMagick reproduces
  rehue's output exactly. Needs dithering 0; `.cube` goes up to level 8.
- Library: `grade_wal::ColorMap`, the per-colour remap (`map_rgb`)
  that `grade_wal::apply` runs per pixel, and `rehue::lut` (Hald /
  .cube builders, built across threads).
- `inspect --schemes [FILTER]`: every scheme as a row of color
  swatches, filtered by name; schemes in `--scheme-dir` are included
  and marked.
- `wal --palette` / `-p`: which scheme colors pixels may be pulled toward,
  and how hard. Keys are slots, roles or `all`, bare (weight 1) or
  `key=weight`; unlisted slots are left out (`--palette ac` for
  accents only, `base0D,base0E` for a duotone, `all,base0B=6` to
  favor one). Also `"palette"` in the wal config; left-out slots are
  dimmed in `preview.html`. Library: `grade_wal::palette_weights`,
  `parse_palette`, and `slot_options` now takes the config.
- `rehue icons`: recolor an icon theme (or one SVG / PNG icon) with a
  scheme, through the same grade as `wal` (flags, looks, configs;
  harmonize 1 by default). Rewrites every color in SVGs (hex,
  `rgb()`, stylesheets, embedded PNGs) and PNGs, keeps the theme's
  links, copies in what links outside it, renames it, rebuilds the
  icon cache when it can, and writes a before/after `preview.png`.
  `--folders` sets the folder color (Places icons named `folder*`,
  `user-home`, `user-desktop`, and `inode-directory`); `-p` paints every
  icon in only the colors you name. Themes are found by name in the usual icon
  directories. Nix: `lib.icons` builds a package for `stylix.icons`.
  Looks take an `[icons]` section (else their `[wal]` one).
- Looks: saved grades picked by name, `-l NAME` on `wal` and
  `scheme` (stack with `-l snap,pop`). Built-ins: soft, tight, pop,
  muted, wash, snap, vivid (soft uses `neutrals mix`, snap paints with
  the accents only); your own go in `~/.config/rehue/looks/NAME.toml`
  with `[wal]` / `[scheme]` sections (a built-in's name replaces it, a
  name with `/` is a path). A look is a starting point: the config and
  the flags win over it, key by key. Configs and pipelines take a
  `look` key; `look` under `[rehue]` covers both pipeline steps.
  `inspect --looks [NAME]` lists them or shows one.
- Short flags: `-i` (the input: whatever the command names), `-s` /
  `-w` (the other input), `-o`, `-c`, `-n` (dry run), `-H`
  (harmonize), `-p` (palette), `-l` (look).
- Role short forms `bg`, `sf`, `fg`, `ac`, accepted on the command line
  and as config keys.
- `--assign` takes `auto`, a family index, or a comma list (`0,2,5`).
- `min-text-contrast` is validated (0..1; it is a lightness gap, not a
  WCAG ratio).
- Docs: reference tables, example commands and every docs image are
  generated from the code and `docs/examples.toml`; `cargo test` fails
  when they drift.

- Config files (`-c`) in TOML or YAML as well as JSON, picked by the
  file's extension (`.toml`, `.yaml` / `.yml`, `.json`). YAML is read
  as YAML 1.2, so `no` and `on` stay words. `inspect -c` now checks its
  keys like the other commands.

### Changed

- `wal` blend mode (the default) grades by colorfulness: hue comes
  from the accents only (the scheme's grays no longer steer colors),
  and chroma follows how colorful each pixel is within its image,
  between the scheme's neutral chroma and its accents'. Muted areas
  settle at the scheme's neutral level like a themed desktop's
  surfaces; vivid details reach accent strength. Wallpapers keep their
  saturation structure instead of being dulled toward sepia.
  `report.json` records the image's `vivid_chroma`; library:
  `grade_wal::vivid_chroma`, `ColorMap::with_vivid`.
- `wal --neutrals floor|mix` (config `neutrals`): `mix` brings back the
  previous blend exactly (the scheme's grays join the mix: a calmer
  cast faded toward them).
- `wal --mode snap` takes colorful pixels to the nearest accent only.
  A scheme's grays no longer catch the colors beside their faint hue
  (Gruvbox's warm beiges turned saturated yellows gray);
  `--neutrals mix` lets every slot compete again, as before.
- `icons` preview sheets wrap at 15 icons per row.
- Docs: the README covers the essentials and links to a page per
  topic in `docs/` (scheme, wal, icons, inspect, looks, grading,
  config, nix, apply, development), each holding its options' examples;
  EXAMPLES.md is split across them, and the roadmap moves to
  ROADMAP.md.

### Changed (breaking)

| before | now |
| --- | --- |
| `rehue map-wal` / `rehue map-scheme` | `rehue wal` (alias `wallpaper`) / `rehue scheme` |
| `rehue schemes` | `rehue inspect --schemes` |
| `map-wal --wallpaper` / `map-scheme --scheme` (the subject) | `-i` / `--input` |
| `[map-wal]` / `[map-scheme]` (pipeline) | `[rehue.wal]` / `[rehue.scheme]` |
| `registers`; `bg`, `surfaces`, `fg`, `accents` | `roles`; `background`, `surfaces`, `foreground`, `accents` |
| `territory = soft / hard` | `mode = blend / snap` |
| `reach-deg` | `reach` |
| `blend-hue`, `blend-chroma`, `blend-light` | `match-hue`, `match-chroma`, `match-lightness` |
| `light` (grade) | `lightness` |
| `distribution` (`true` / N / [N]) | `assign` (`"auto"` / N / [N]) |
| `rotate` | `cycle` |
| `gray-chroma-floor` | `max-gray-chroma` |
| `image-max-dimension`, `chroma-pixel-floor`, `lightness-window` | `max-dimension`, `min-pixel-chroma`, `lightness-range` |
| `max-hues`, `seed-separation-deg`, `cluster-iterations` | `max-families`, `seed-spacing`, `iterations` |
| `merge-deg`, `min-cluster-weight`, `accent-chroma-floor` | `merge-within`, `min-family-weight`, `min-accent-chroma` |
| `fg-contrast-floor` (extraction) | `min-text-contrast` (scheme config, top level) |
| Nix `lib.mapScheme` / `lib.mapWal` | `lib.scheme` / `lib.wal` |
| Library `map_wal`, `register`, `RemapConfig`, `MapConfig`, `Territory`, `Distribution` | `grade_wal`, `grade_scheme` + `role`, `WalConfig`, `SchemeConfig`, `Mode`, `Assign` |

Unknown keys in `--config` files and pipeline tables are now errors
(with a "did you mean" hint) instead of being silently ignored.

The underscore spellings of extraction keys (from the old nix wrapper)
are no longer accepted.

## [0.2.0] - 2026-11-21

### Added

- `rehue apply --pipeline <file.toml> [--dry-run]`: runs a whole
  pipeline from one TOML file and applies the result. `[map-scheme]`
  and `[map-wal]` take the same keys as their JSON `--config` files
  (a table being present turns that step on); the scheme is installed
  as a tinty custom scheme and applied with `tinty apply`, and a
  `wallpaper-command` sets the wallpaper. Needs tinty 0.35 or newer.
- Library: `rehue::apply` (pipeline loading, the tinty handoff) and
  `Scheme::resolve_text`.

## [0.1.0] - 2026-11-21

First release.

- `map-scheme` (scheme + wallpaper → scheme) and `map-wal`
  (wallpaper + scheme → wallpaper), with per-group grading dials.
- `inspect`, `enhance` (Lanczos or Vulkan super-resolution), `schemes`.
- ANSI palette strips and a self-contained `preview.html`.
- 375 bundled base16 schemes (tinted-theming snapshot).
- Deterministic, byte-identical output; Nix builders for Stylix.

[0.5.0]: https://github.com/fortesoft-co/rehue/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/fortesoft-co/rehue/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/fortesoft-co/rehue/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/fortesoft-co/rehue/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/fortesoft-co/rehue/releases/tag/v0.1.0
