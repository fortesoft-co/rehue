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
- **Scheme pager**: page `inspect --schemes` through `$PAGER`
  (`less -R`) when stdout is a terminal, as git does, with
  `--no-pager`; pipes stay plain.
- **WCAG contrast mode**: a contrast-ratio option for `rehue scheme`
  (e.g. 4.5:1, the ratio theme designers check against) alongside the
  lightness-gap guard `min-text-contrast`, and possibly covering more
  foreground slots than base05.
- **Scheme generation**: the big one. Derive the lightness/chroma/
  contrast structure a designer would have built, guided by the
  wallpaper, so `rehue scheme` works without a reference scheme at all.
- **Look book**: a docs page showcasing standout results, each with
  the images and the recipe that made it.
- **Dithered video**: `--dither` on a video wallpaper (ordered styles;
  diffuse with a warning that it shimmers). Which codec settings keep
  a dither pattern intact is under test.
- **Encoder options for video**: a generic way to hand ffmpeg its
  encoding flags (`--ffmpeg-args "-c:v libsvtav1 -crf 30"`, or a
  `ffmpeg-args` key in a config or look), replacing rehue's codec
  defaults while rehue keeps the frame pipe, the color tags and the
  audio mapping. To investigate: which flags rehue must refuse or
  keep (input options, filters, `-pix_fmt` for 10-bit), and an output
  extension that follows the chosen codec.
- **Seamless loops**: `--loop SECONDS` crossfades a video's last
  seconds into its first, so the end flows into the beginning (the
  output is that much shorter), done by ffmpeg (`xfade`, `acrossfade`
  for the audio). rehue picks where to cut: the frame that looks most
  like the opening, so the crossfade has little to hide. To
  investigate: how to score "looks most like" (on the graded frames),
  and how far from the end to search.
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
