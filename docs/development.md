# Development

## Build and test

```sh
# with your rustup toolchain:
cargo test
cargo build --release

# with Nix (pinned toolchain):
nix develop -c cargo test
nix develop -c cargo build --release
nix build .#rehue
```

`cargo test` runs the snapshot and determinism suites.

Per-pixel work runs across threads (`src/par.rs`): contiguous runs of
whole pixels, rows or block rows, so the output never depends on the
thread count. `REHUE_THREADS=N` pins the count (`1` runs everything on
one thread); `tests/threads.rs` checks 1 and 5 give the same bytes.

## Bundled schemes

The bundled schemes are a vendored snapshot of
[tinted-theming/schemes](https://github.com/tinted-theming/schemes)
(MIT, see `vendor/tinted-schemes/LICENSE`), pinned to the commit
recorded in `vendor/tinted-schemes/SOURCE`. To refresh before a
release (it re-runs the scheme tests, so a malformed upstream scheme
fails here):

```sh
nix develop -c scripts/update-schemes.sh            # upstream HEAD
nix develop -c scripts/update-schemes.sh <commit>   # or a pinned commit/tag
```

## Generated docs

The docs are partly generated, and `cargo test` keeps them honest.
Reference tables, example commands and counts sit between
`<!-- gen:NAME -->` markers and render from the code; everything
outside the markers is hand-written. A test fails when a region is
stale, when the hand-written text names a flag or option that doesn't
exist, or when a link between pages points at a missing file or
heading:

```sh
REHUE_BLESS=1 cargo test --test docs    # re-render every region
```

## Docs images

Every image in the docs, and the command that made it, is defined once
in `examples/docs/examples.toml`. The builder runs the commands and
assembles the images (the `examples` devshell adds imagemagick,
headless chromium and the Papirus icons the icon example recolors);
`examples/docs/examples.lock` records what was built, so a test fails
when an example changes without a rebuild:

```sh
nix develop .#examples -c cargo run --release --example docs-images
nix develop .#examples -c cargo run --release --example docs-images -- wal-mode   # just one
```

## Conventions

### Branches and commits

- Work happens on `develop`. `master` only takes fast-forwards of
  finished work, so it never holds anything in flight.
- Commit messages are one line, lowercase, in the form
  `area: what changed (detail); area2: more`. The area is a command,
  module or surface: `wal`, `icons`, `look`, `docs`, `nix`, `readme`,
  `changelog`. No body and no trailers.
- Split commits by area when a change spans unrelated ones (a docs
  restructure and a grading fix are two commits).

### Before committing

All of these pass:

```sh
nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
RUSTDOCFLAGS="-D warnings" nix develop -c cargo doc --no-deps
nix build
```

Flakes only see files git tracks: `git add` new files before
`nix build`, or the build won't find them.

### Changelog and roadmap

- Every user-visible change gets a CHANGELOG entry under
  `[Unreleased]`, in the Added / Changed / Changed (breaking)
  sections (the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
  format).
- Until 1.0 there are no compatibility aliases: a renamed flag or key
  breaks, and the changelog says so.
- [ROADMAP.md](../ROADMAP.md) lists development work only. An item
  leaves the roadmap when it ships.

### Code

- **Deterministic.** No randomness, no dependence on hash-map
  iteration order in anything written out, no clocks: the same inputs
  give byte-identical outputs. Tests check it.
- **Legibility first.** A grade never changes a slot's lightness or
  chroma unless a dial asks for it, and the contrast check runs last.
- **Color math lives in `src/color.rs`.** Grading works in OKLCH;
  other modules call the helpers there rather than converting
  themselves.
- **Pure Rust where possible.** The image codecs are pure Rust, and
  new dependencies go in without default features unless they're
  needed. The minimum Rust version is 1.88.
- **Module docs open with a one-line summary**, a `//!` paragraph of
  its own. The source-files table below is generated from it, and the
  docs test fails without one.
- **New behavior gets a test** in the suite for its surface (see
  `tests/`), and a grading change gets one that pins the behavior with
  a concrete color.
- **Opt-in for anything risky.** rehue writes into its output
  directory; anything that would edit a user's existing files in place
  is never a default.

### CLI and config

- The CLI is defined in `src/cli.rs` as data. Each flag's help is a
  one-line summary, an optional detail paragraph, and an `e.g.` line
  with a real example. The docs' flag tables render from it.
- Every flag has a config key with the same name, kebab-case, and
  every key that has a flag behaves the same in a config file, a look,
  a pipeline table and on the command line. Unknown keys are errors.
- Settings layer as defaults, then looks (left to right), then the
  config, then the flags, merged key by key. `palette` is the one key
  that replaces instead of merging.
- `-i` is always the thing a command rehues; `-s` and `-w` are the
  other input. Short flags are for everyday options only, and a new one
  must not clash with any command's existing ones.

### Writing docs

- The README covers the essentials: what rehue is, the worked example,
  and a short section per command or topic that links to its page in
  `docs/`. Anything the README says is a subset of what its page says,
  never a second wording of the same detail.
- Docs are direct how-tos: "here's how you set the folder color".
  Describe each use case on its own terms; don't frame a feature
  against alternatives the reader never saw (design discussions stay
  out of the docs).
- User-facing text spells it "color".
- Anything that can go stale (flag tables, the looks table, counts,
  the commands that made an image) goes in a generated region. Never
  edit inside one by hand; re-render with `REHUE_BLESS=1`.
- Links between pages are relative (crates.io rewrites the README's
  against the repository), and the docs test checks every one.
- A new option gets: its flag and config key, an entry in its
  command's page with a ladder, its help `e.g.` line, and a changelog
  entry.

### Docs images

- Every image is built from `examples/docs/examples.toml`; no docs
  image is made by hand.
- A ladder shows one option stepping, starting from the original
  where there is one. Each cell is labelled with the flags that made
  it, and the recipe under the image opens with the inputs it ran on.
- Pick inputs that show the option: an image or window where the
  change is plain to see. The builder flags rungs that barely change
  the picture; either rework the example or say in the text why the
  effect is subtle.
- Use in-repo assets for inputs. Anything else (the Papirus icons) is
  pinned in the `examples` devshell.
- Every input is credited in `assets/attribution.md` with its license,
  author and source, even when the license doesn't require it. Pick
  licenses that allow reuse and modification (CC0 and CC BY are the
  usual ones); leave out ShareAlike and NonCommercial sources, whose
  terms would follow the docs images made from them.
- Choose inputs for what the option shows: flat colors and outlines
  for `checker`, smooth shading for `halftone`, `flat` and diffusion.

## Source files

<!-- gen:source-files -->
| file | what it is |
| --- | --- |
| `src/apply.rs` | `rehue apply`: run a pipeline file and hand the result to the desktop. |
| `src/cli.rs` | The CLI's commands, flags and help, as data (the docs render from it). |
| `src/color.rs` | sRGB, OKLab and OKLCH conversions, and circular-hue math. |
| `src/config.rs` | Config layering, file reading and the unknown-key check. |
| `src/dither.rs` | The dither stage: draw an image with palette colors only, as a pattern. |
| `src/enhance.rs` | `rehue enhance`: Lanczos upscaling and Vulkan super-resolution. |
| `src/extract.rs` | Hue extraction: a chroma²-weighted hue histogram and a deterministic circular k-means. |
| `src/grade_scheme.rs` | `rehue scheme`: the per-role pipeline and its options. |
| `src/grade_wal.rs` | `rehue wal`: the per-color remap (`ColorMap`), its modes, and the dither stage. |
| `src/grain.rs` | Fine grain: a small, deterministic jitter in lightness, after the grade. |
| `src/icons.rs` | `rehue icons`: SVG and PNG recoloring, theme layout, folders, the preview. |
| `src/lib.rs` | rehue: paint base16 color schemes and wallpapers with each other's colors. |
| `src/look.rs` | Looks: saved grades, selectable by name (`-l pop`). |
| `src/looping.rs` | Seamless loops: a video cut so its end flows back into its start. |
| `src/lut.rs` | Hald CLUT and `.cube` export of a color function. |
| `src/par.rs` | Per-pixel work across threads, with results that don't depend on the thread count. |
| `src/preview.rs` | The compiled-in `preview.html`: CSS-only views, GTK and Qt mocks. |
| `src/role.rs` | The four roles, their names, and `assign`, shared by both directions. |
| `src/scheme.rs` | Base16 scheme loading and rendering. |
| `src/terminal.rs` | Truecolor ANSI swatch strips for `inspect`. |
| `src/video.rs` | Video wallpapers: frames in and out through ffmpeg, graded in between. |
| `src/bin/rehue.rs` | `rehue` CLI: the mapping flows, plus `inspect` and `enhance` utilities. |
| `src/generate/analysis.rs` | What generation reads from a wallpaper. |
| `src/generate/finish.rs` | Neutrals, the accents' chroma level, and finishing. |
| `src/generate/math.rs` | Color math the generator works in. |
| `src/generate/methods.rs` | The generation methods' accent candidates. |
| `src/generate/mod.rs` | Scheme generation: a base16 scheme from a wallpaper alone. |
| `src/generate/rswal.rs` | rswal: the image's own colors as accents, as pywal picks them. |
| `build.rs` | Embeds `vendor/tinted-schemes` as the bundled scheme table. |
| `examples/docs-images.rs` | Builds the docs images from `examples/docs/examples.toml`. |
| `tests/apply.rs` | `rehue apply`: pipeline files, the tinty handoff, the wallpaper command. |
| `tests/cli.rs` | Cross-command CLI behaviour (the shared flags). |
| `tests/config.rs` | Config files: JSON, TOML and YAML, picked by extension. |
| `tests/dither.rs` | The dither stage: palette colors only, drawn as a pattern. |
| `tests/docs.rs` | The docs: generated regions, the drift check, links, example images. |
| `tests/enhance.rs` | `rehue enhance`: the model table, parameter checks, scaling. |
| `tests/extraction.rs` | Extraction: determinism on a synthetic gradient. |
| `tests/formats.rs` | Input formats: png, jpeg, webp, gif, tiff, bmp and qoi all read. |
| `tests/generate.rs` | Scheme generation: the guarantees every generated scheme keeps. |
| `tests/grade_scheme.rs` | `rehue scheme`: the role pipeline, assign and cycle. |
| `tests/grade_wal.rs` | `rehue wal`: modes, arrangement, blend dials. |
| `tests/grain.rs` | Grain: small, centered, monochrome, the same every run, new each frame. |
| `tests/icons.rs` | `rehue icons`: colour rewriting, theme layout, folders. |
| `tests/look.rs` | Looks: saved grades layered under the config and the flags. |
| `tests/lut.rs` | LUT export and the per-color remap it bakes. |
| `tests/palette.rs` | wal's `palette` option. |
| `tests/preview.rs` | The compiled-in HTML preview (`preview.html`). |
| `tests/scheme.rs` | Schemes: parsing and rendering, the bundled collection, name resolution. |
| `tests/terminal.rs` | The ANSI strip renderers (the terminal previews). |
| `tests/threads.rs` | The output is the same bytes however many threads the work is cut into. |
| `tests/video.rs` | Video wallpapers through ffmpeg (skipped when ffmpeg isn't installed). |
| `tests/common/mod.rs` | Shared test inputs: repo-fixture paths and synthetic palettes/images. |
<!-- /gen:source-files -->

Each file's description is the first line of its `//!` module doc.
The directories:

- `src/looks/`: the built-in looks, one TOML file each
- `vendor/`: the tinted-theming/schemes snapshot (`scripts/update-schemes.sh`)
- `assets/`: the example images, and the generated docs images in its `examples` folder
- `docs/`: these pages (README.md links to each)
- `examples/docs/`: `examples.toml` (every docs image and its command),
  its lock, and `manifest.rs`, which reads them
- `tests/fixtures/`: schemes and files the tests read
