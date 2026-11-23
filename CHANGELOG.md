# Changelog

All notable changes to rehue. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rehue
uses [semantic versioning](https://semver.org/) (pre-1.0: minor
versions may break).

## [Unreleased]

### Changed

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

[0.4.0]: https://github.com/fortesoft-co/rehue/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/fortesoft-co/rehue/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/fortesoft-co/rehue/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/fortesoft-co/rehue/releases/tag/v0.1.0
