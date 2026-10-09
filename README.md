# rehue

*re-hue (verb): to color again differently.*

**rehue** maps **base16 color schemes and wallpapers onto each
other**: a terminal palette painted with your wallpaper's hues, or a
wallpaper repainted with the palette your terminal uses. Deterministic
and legibility-preserving, with the two outputs built to close back on
each other.

**`rehue map-scheme`** — scheme + wallpaper → scheme. It takes the *hues* from
your wallpaper and leaves everything that made the scheme readable
(lightness, chroma, contrast) exactly as a designer built it. The
palette feels like your wallpaper but behaves like the scheme.

**`rehue map-wal`** — wallpaper + scheme → wallpaper. The inverse. Any wallpaper,
any base16 scheme — the wallpaper comes back repainted with the
scheme's palette. No more hunting for that perfect wallpaper — the
picture you love is now the one that matches.

And using both together unlocks something interesting: the remapped scheme can
go straight back into the image — repaint the wallpaper with the palette
it generated itself.

## Why

Most wallpaper ↔ color scheme tools try to generate a palette from raw
image clusters, with no contrast guarantees — the results are
frequently muddy. rehue separates the two things a theming tool
actually needs: *which hues* come from the wallpaper, and *how the hues
are shaped* (lightness, chroma, contrast) comes from a scheme a
designer already built — Solarized, Gruvbox, Rosé Pine, any
[tinted-scheme](https://github.com/tinted-theming/schemes) YAML. 
The divergence between the scheme and the picture it came
from becomes structurally impossible.

Guarantees, because a theme generator that can't be trusted is a
hobby, not a tool:

- **Deterministic** — no RNG anywhere (cluster seeding included); the
  same inputs produce byte-identical outputs.
- **Legibility preserving** — slots adopt the reference scheme's
  lightness/chroma and only change them through explicit dials; a guard
  restores fg-vs-bg contrast before rendering.

## Examples

All inputs are in-repo assets (CC0, see
[assets/attribution.md](assets/attribution.md)).

### Rainbow glitter → Gruvbox Light

One dial: `{"harmonize": 1.0}`. The facade re-keys hue *and* chroma of
every pixel toward the palette; lightness stays photographic, so the
glitter keeps its sparkle and loses its rainbow.

```sh
rehue map-wal \
  --wallpaper assets/vidsplay-rainbow-glitter.webp \
  --scheme gruvbox-light.yaml \
  --out glitter \
  --config harmonize.json
```

with `harmonize.json`:

```json
{ "harmonize": 1.0 }
```

![rainbow glitter before/after the gruvbox light repaint](assets/examples/glitter-facade1-pair.webp)

Coverage lands spread wide (the scheme's brightest surface carried 39%
of the pixels, the next slots in single digits) — one palette, still a
photograph.

### Rosé Pine Dawn → Butterfly

Defaults do the work: the butterfly's hues are extracted by
chroma-weighted mass and adopted slot by slot. Rosé Pine Dawn's
lightness, chroma and contrast carry through untouched.

```sh
rehue map-scheme \
  --wallpaper assets/kelly-ishmael-butterfly-closeup.webp \
  --scheme rose-pine-dawn.yaml \
  --out mapped
```

![kelly-ishmael butterfly closeup original](assets/kelly-ishmael-butterfly-closeup.webp)

![rose-pine-dawn before/after, painted with the butterfly's hue families](assets/examples/butterfly-rose-pine-dawn.png)

### The full round trip

Map-scheme first: paint Solarized Dark with the hues of a wallpaper.
Here with distribution sculpting per register — `bg` pinned to the
magenta family, `surfaces` spanning magenta → blue, `fg` pinned to the
olive family, accents on the four-family ramp, all with a chroma lift
so the hue rotation reads in the swatches:

```sh
rehue inspect \
  --wallpaper assets/bango-renders-3d-abstract.webp \
  --out inspect

rehue map-scheme \
  --wallpaper assets/bango-renders-3d-abstract.webp \
  --scheme solarized-dark.yaml \
  --out mapped \
  --config roundtrip.json
```

with `roundtrip.json`:

```json
{
  "registers": {
    "all": {"blend-hue": 1.0},
    "bg": {"distribution": 1, "chroma": 1.6},
    "surfaces": {"distribution": [1, 3], "chroma": 1.6},
    "fg": {"distribution": 0, "chroma": 1.6},
    "accents": {"distribution": [0, 1, 2, 3], "chroma": 0.75}
  }
}
```

Scheme before (top) / after (bottom) — solarized's structure kept,
only hues move:

![solarized-dark before/after, painted with the wallpaper's hue families](assets/examples/bango-schemes-roundtrip.png)

Then feed it back into the image:

```sh
rehue map-wal \
  --wallpaper assets/bango-renders-3d-abstract.webp \
  --scheme mapped/scheme.yaml \
  --out repainted \
  --config harmonize.json
```

![bango before/after repainting through its own recolored scheme](assets/examples/bango-roundtrip-pair.webp)

The loop closed: the palette your terminal uses is now the palette the
picture was painted with.

`map-scheme` writes `scheme.yaml` + `preview.png` (before/after
swatches) + `clusters.json`; `map-wal` writes `wallpaper.png` +
`compare.png` (side-by-side thumbnails) + `report.json` (per-slot pixel
coverage).

## Usage

### With cargo

The crate is plain cargo, and the image codecs (png/jpeg/webp) are pure
Rust — a stock Rust toolchain is the only requirement. Requires 1.85 or
newer (edition 2024); rustup handles that automatically via
[`rust-toolchain.toml`](rust-toolchain.toml).

```sh
git clone https://github.com/fortesoft-co/rehue
cd rehue
cargo run --release -- map-scheme \
  --wallpaper my-photo.jpg \
  --scheme nord.yaml \
  --out mapped
# mapped/scheme.yaml + preview.png + clusters.json

cargo install --path .          # installs the `rehue` binary
rehue map-wal --wallpaper my-photo.jpg --scheme mapped/scheme.yaml --out repainted --harmonize 1

# Everyday options are named flags; register dials accept a target —
# `--chroma bg 1.6`, repeatable per register — while a bare value is the
# seed (map-wal's bare --light/--chroma stay the image-wide grade).
# Ramp/stop distribution sculpting and extraction options still live in
# `--config`:
rehue map-wal --wallpaper my-photo.jpg --scheme mapped/scheme.yaml --out repainted --config remap.json
```

Scheme inputs are plain tinted-scheme YAML files — grab one from the
[tinted-theming/schemes](https://github.com/tinted-theming/schemes)
collection. `cargo test` runs the test suite: snapshot and
determinism checks.

### With Nix

The flake exposes the same flows plus builders a consumer flake can
import directly — the Stylix wiring is four lines: the remapped
palette feeds `stylix.base16Scheme`, the repainted wallpaper feeds
`stylix.image`, and GTK/Firefox/Zed/terminals/GNOME all restyle from
one store path.

```nix
inputs.rehue.url = "github:fortesoft-co/rehue";

# in a module:
let
  mapped = rehue.lib.${system}.mapScheme {
    wallpaper = ./assets/glitter.webp;
    scheme = "${pkgs.base16-schemes}/share/themes/gruvbox-light.yaml";
    # registers = { fg = { blend-hue = 0.4; chroma = 0.85; }; };
  };
  repainted = rehue.lib.${system}.mapWal {
    wallpaper = ./assets/glitter.webp;
    scheme = "${mapped}/scheme.yaml";
    config = { harmonize = 1.0; };
  };
in {
  stylix.base16Scheme = "${mapped}/scheme.yaml";
  stylix.image = "${repainted}/wallpaper.png";
}
```

Every builder's output is a plain store path: the colour work happens
inside the build sandbox (nothing reads global config state, nothing
calls out), and the results — a scheme YAML, a PNG, the extraction
report — commit and diff like the rest of your config.

## Options

Every option is opt-in; absent config is the sane default. Both flows
share registers (`bg` = base00, `surfaces` = base01-03, `fg` =
base04-07, `accents` = base08-0F) with an `all` record seeding every
register. The deep version — per-dial mechanics, visual demos, and
merge semantics — lives in [OPTIONS.md](OPTIONS.md).

### Shared arrangement (both flows)

| option | semantics |
| --- | --- |
| `distribution` | hue-family adoption (JSON syntax: `true` = ramp, `3` = pin, `[0,1,2,3]` = stops) |
| `rotate` | rotate the register's hues across its slots (`1` = one right, wraps) |

### map-scheme (wallpaper → scheme)

| option | semantics |
| --- | --- |
| `blend-hue` | hue adoption from the wallpaper, 0..1 (default 1) |
| `reach-deg` | accent claim gate in hue degrees (default 45) |
| `light` / `chroma` | additive lightness shift / multiplicative chroma ratio, per register |

Extraction (`map-scheme --config` / `rehue inspect --config`):
`image-max-dimension`, `chroma-pixel-floor`, `lightness-window`,
`max-hues`, `seed-separation-deg`, `cluster-iterations`, `merge-deg`,
`min-cluster-weight`, `accent-chroma-floor`, `fg-contrast-floor`.

### map-wal (scheme → wallpaper)

| option | semantics |
| --- | --- |
| `territory` | how pixels see the palette: `soft` influence field (default) / `hard` flat-constant snapping |
| `harmonize` | one-dial facade: seeds `blend-hue` + `blend-chroma` where unset (default 0) |
| `blend-hue` / `blend-chroma` | hue / chroma movement toward the palette (0 = raw pixel, 1 = full snap) |
| `blend-light` | lightness movement, default 0 — photographic |
| `reach-deg` | influence reach in hue degrees (default 45) |
| `gray-chroma-floor` | achromatic threshold (default 0.02) |
| `dithering` / `dithering-mode` | dithering strength 0..1 (0 off) / kernel (default blue-noise) |
| `light` / `chroma` | image-wide grade, applied last (additive L, multiplicative C) |

## Repository

```
src/color.rs     sRGB/OKLab/OKLCH + circular-hue math (one source of truth)
src/scheme.rs    base16 YAML parse/render + slot access
src/extract.rs   chroma^2 hue histogram + deterministic circular k-means
src/register.rs  map-scheme: the register pipeline and its options
src/map_wal.rs   map-wal: per-pixel blend/grade + territory + dithering
src/bluenoise.rs embedded 64x64 void-and-cluster mask (CC0)
src/bin/rehue.rs the CLI
tests/golden.rs  snapshot + determinism + vocabulary-contract checks
```

## Development

```sh
# with your rustup toolchain:
cargo test
cargo build --release

# with Nix (pinned toolchain):
nix develop -c cargo test
nix develop -c cargo build --release
nix build .#rehue
```

Both paths are equivalent; `rust-toolchain.toml` pins the toolchain for
rustup users, the devshell does the same through Nix.

## Roadmap

- **Named theme inputs** — bundle the
  [tinted-schemes](https://github.com/tinted-theming/schemes) set so
  `--scheme gruvbox-light` resolves without a YAML path.
- **Terminal previews** — `inspect` swatches and whole schemes
  rendered as truecolor ANSI, straight in the terminal — no image
  viewer hop.
- **UI previews** — demo renders of the scheme applied to real
  surfaces: a terminal, a code block, a web page, GTK and Qt widgets,
  so a scheme can be judged before it's wired in.
- **base24 coverage** — base10-17 slots.
- **Extraction tuning** — the extraction options documented as their own
  surface, optionally backed by alternative extraction libraries.
- **Scheme generation** — the big one: derive the lightness/chroma
  structure a designer would have built, guided by the wallpaper —
  map-scheme without needing a reference scheme at all.

## License

AGPL-3.0-or-later - see [LICENSE](LICENSE).
