# Roadmap

Planned and possible work. [CHANGELOG.md](CHANGELOG.md) has what
shipped.

- **Scheme pager**: page `inspect --schemes` through `$PAGER`
  (`less -R`) when stdout is a terminal, as git does, with
  `--no-pager`; pipes stay plain.
- **Accessibility workshop**: opt-in tweaks and modes for
  accessibility, tested like the generation research and kept out of
  default behavior. Among them: keeping red and green apart for
  colorblind readers, higher contrast targets, and a WCAG contrast mode
  for `rehue scheme` (a contrast ratio, e.g. 4.5:1, alongside the
  lightness-gap guard `min-text-contrast`, possibly covering more
  foreground slots than base05).
- **Look book**: a docs page showcasing standout results, each with
  the images and the recipe that made it.
- **Alternative extraction backends**: optional extraction libraries
  alongside the built-in histogram + circular k-means (the options
  themselves are documented in [docs/grading.md](docs/grading.md#extraction)).
- **Enhance expansion**: beyond v1 (Lanczos + Vulkan SR): denoise and
  JPEG-artifact restoration models (Real-CUGAN / realesrnet), and an
  opt-in Nix SR builder (lavapipe for machines without a GPU driver).

## On demand

Left out until someone asks for them.

- **Animated GIF / APNG / WebP**: repaint a loop and keep its format.
- **AVIF and JPEG XL inputs**: AVIF decoding needs a C library (dav1d);
  JPEG XL has a pure-Rust decoder (jxl-oxide).
- **Shader export**: the grade as a LUT texture plus GLSL, with ordered
  dithering, for mpv, glpaper or Hydra.
- **Apply, next steps**: `rehue apply` v1 hands off to tinty.
  - *Native rendering* (evaluation): render tinted-theming templates
    directly (e.g. with the `tinted-builder` library), so apply works
    without tinty and templates can render inside a Nix build. Also
    investigate whether Stylix's per-app targets can be reused, so Nix
    and non-Nix setups style apps the same way.
  - *Hooks* (investigation): tinty already runs per-app hooks; decide
    what, if anything, rehue should own beyond `wallpaper-command`.
  - *Patch* (investigation, opt-in only): rewriting colors in place
    inside existing CSS/config files, as lutgen's `patch` does. Never
    the default path: fidelity is uncertain and in-place writes can
    break files, so at minimum diff-first with explicit confirmation.
- **Nix module**: a home-manager / NixOS module with typed options
  (checked and documented per option) that writes rehue's config and
  wires the builders' outputs into Stylix, alongside today's `lib`
  builders and their `config` attribute set.
- **Loop bridges in auto**: `wal --loop` lists bridges but `auto`
  takes fades, as the seam score can't tell whether `minterpolate`
  will smear. A bridge won on a slow camera move and smeared a city
  skyline; find a signal (camera motion is the first guess) so `auto`
  can take one when it would win.
- **rehue as a library**: a deliberate public API (grading, dithering,
  video and loops), with rustdoc examples and semver promises separate
  from the CLI, so other tools can plug into it and build something
  bigger.
