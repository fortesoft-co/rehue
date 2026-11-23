# Roadmap

Planned and possible work. [CHANGELOG.md](CHANGELOG.md) has what
shipped.

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
- **Scheme browser**: pick a scheme by eye from `inspect --schemes`.
  First step: page through `$PAGER` (`less -R`) when stdout is a
  terminal, as git does, with `--no-pager`; pipes stay plain. Later: an
  interactive browser (ratatui) with a live preview of the selected
  scheme (its terminal colors, or the wallpaper repainted).
- **WCAG contrast mode**: a contrast-ratio option for `rehue scheme`
  (e.g. 4.5:1, the ratio theme designers check against) alongside the
  lightness-gap guard `min-text-contrast`, and possibly covering more
  foreground slots than base05.
- **Scheme generation**: the big one. Derive the lightness/chroma/
  contrast structure a designer would have built, guided by the
  wallpaper, so `rehue scheme` works without a reference scheme at all.
- **Advanced examples**: a docs page for the complex combinations
  kept out of the main docs (assign across roles, reach tuning, colorize
  recipes), each with the images to prove it.
- **base24 coverage**: slots base10-17.
- **Media formats**: more inputs beyond png / jpeg / webp (gif, tiff,
  bmp, qoi, avif where a pure-Rust decoder allows), and animated
  GIF / APNG / WebP (LUT export already makes per-frame repaints a
  table lookup).
- **Alternative extraction backends**: optional extraction libraries
  alongside the built-in histogram + circular k-means (the options
  themselves are documented in [docs/grading.md](docs/grading.md#extraction)).
- **Enhance expansion**: beyond v1 (Lanczos + Vulkan SR): denoise and
  JPEG-artifact restoration models (Real-CUGAN / realesrnet), a
  low-strength img2img restyle (sd.cpp) as a creative pass, and an
  opt-in Nix SR builder (lavapipe for machines without a GPU driver).
- **Performance**: the per-pixel wal pass is single-threaded.
  Non-dithered and ordered-dither modes parallelize cleanly (each
  pixel depends only on x, y), which should bring a 33MP remap from
  ~18s to ~2-3s; error-diffusion dithering stays sequential by nature.
  For non-dithered runs, a LUT (already built across threads for
  `--lut`) is the other fast path: build once, then look up.
