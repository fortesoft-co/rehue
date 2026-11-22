# Changelog

All notable changes to rehue. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rehue
uses [semantic versioning](https://semver.org/) (pre-1.0: minor
versions may break).

## [Unreleased]

The vocabulary release: commands, flags and config keys are renamed to
fit the domain. There are no compatibility aliases; old names are
rejected (pre-1.0, see the table below).

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
- Looks: saved grades picked by name, `-l NAME` on `wal` and
  `scheme` (stack with `-l snap,pop`). Built-ins: soft, tight, pop,
  muted, snap, vivid; your own go in `~/.config/rehue/looks/NAME.toml`
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

[0.2.0]: https://github.com/fortesoft-co/rehue/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/fortesoft-co/rehue/releases/tag/v0.1.0
