# Changelog

All notable changes to rehue. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and rehue
uses [semantic versioning](https://semver.org/) (pre-1.0: minor
versions may break).

## [Unreleased]

### Added

- `map-wal --lut hald|cube [--lut-level 2..16]`: bake the grade into a
  3D LUT (`lut.png` Hald CLUT or `lut.cube`) for ImageMagick, ffmpeg,
  GIMP, Darktable or Resolve. At level 16 ImageMagick reproduces
  rehue's output exactly. Needs dithering 0; `.cube` goes up to level 8.
- Library: `map_wal::ColorMap`, the per-colour remap (`map_rgb`) that
  `map_wal::apply` runs per pixel, and `rehue::lut` (Hald / .cube
  builders, built across threads).
- `inspect --schemes [FILTER]`: every scheme as a row of color
  swatches, filtered by name; schemes in `--scheme-dir` are included
  and marked.

### Changed

- `rehue schemes` is now `rehue inspect --schemes`. The old command
  still works (hidden, with a notice) for this release.

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
