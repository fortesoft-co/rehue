# rehue

*re-hue (verb): to color again differently.*

**rehue** maps **base16 color schemes and wallpapers onto each other**,
deterministically:

- `rehue map-scheme` - **wallpaper → scheme**. Extracts the wallpaper's
  hue families, and paints those hues onto a structural reference scheme
  (Solarized, Gruvbox, Rosé Pine - any tinted-scheme YAML), freezing the
  reference's lightness and chroma per slot, so the scheme's designed
  contrast is inherited by construction. Slots are grouped into registers
  (`bg`, `surfaces`, `fg`, `accents`), each with its own controls.
- `rehue map-wal` - **scheme → wallpaper**. The inverse: repaints a wallpaper
  with a scheme's palette so the image on screen belongs to the palette it
  generated. `harmonize` re-keys hues only (photographic texture survives);
  `quantize` adopts the palette's full tonal shape; both are threshold-gated
  and opt-in. Works with any base16 YAML - a favourite hand-picked scheme
  drives the wallpaper too, no image editor involved.

The intended consumer is [Stylix](https://github.com/nix-community/stylix):
the mapped palette feeds `stylix.base16Scheme`, the remapped wallpaper feeds
`stylix.image`, and GTK/Firefox/Zed/terminals/GNOME all restyle from one
store path. The loop closes: divergence between the scheme and the picture
it came from becomes structurally impossible.

## Guarantees

- **Deterministic** - no RNG, no timestamps, no network in the build; the
  same inputs produce byte-identical outputs. Cluster seeding is
  histogram-driven (no random k-means initialisation).
- **Pure** - all colour work happens inside the build sandbox; nothing
  reads global config state.
- **Legibility preserving** - slots adopt the reference scheme's
  lightness/chroma and only change them through the explicit grade dials;
  a guard restores fg-vs-bg contrast before rendering.

## Usage

### With cargo

The crate is plain cargo, and the image codecs (png/jpeg) are pure Rust -
a stock Rust toolchain is the only requirement.

```sh
git clone https://github.com/fortesoft-co/rehue
cd rehue
cargo run --release -- map-scheme \
  --wallpaper my-photo.jpg \
  --scheme nord.yaml \
  --out mapped
# mapped/scheme.yaml + preview.png + clusters.json

cargo install --path .          # installs the `rehue` binary
rehue map-wal --wallpaper my-photo.jpg --scheme mapped/scheme.yaml --out map-wal
```

```sh
rehue map-scheme --wallpaper wal.jpg --scheme solarized-dark.yaml --out mapped
rehue map-wal    --wallpaper wal.jpg --scheme mapped/scheme.yaml --out map-wal --config map-wal.json
```

`cargo test` runs the test suite: output snapshots and determinism checks.
Scheme inputs
are plain tinted-scheme YAML files - grab one from the
[tinted-theming/schemes](https://github.com/tinted-theming/schemes)
collection. Requires any rustup toolchain at 1.85 or newer (edition 2024);
rustup handles that automatically via [`rust-toolchain.toml`](rust-toolchain.toml).

### With Nix

The flake exposes the same flows plus Stylix-wiring helpers:

```nix
inputs.rehue.url = "github:fortesoft-co/rehue";

# in a module:
let
  mapped = rehue.lib.${system}.mapScheme {
    wallpaper = ./assets/cave-sunset-view.png;
    scheme = "${pkgs.base16-schemes}/share/themes/rose-pine.yaml";
    # registers = { fg = { hue-blend = 0.4; chroma = 0.85; }; };
  };
  remapped = rehue.lib.${system}.mapWal {
    wallpaper = ./assets/cave-sunset-view.png;
    scheme = "${mapped}/scheme.yaml";
    config = { harmonize = 1.0; };
  };
in {
  stylix.base16Scheme = "${mapped}/scheme.yaml";
  stylix.image = "${remapped}/wallpaper.png";
}
```

Outputs: `map-scheme` writes `scheme.yaml` + `preview.png` (before/after
swatches) + `clusters.json`; `map-wal` writes `wallpaper.png` +
`compare.png` (side-by-side thumbnails) + `report.json` (per-slot pixel
coverage).

## Knobs

Registers (`bg` = base00, `surfaces` = base01-03, `fg` = base04-07,
`accents` = base08-0F); an `all` record seeds every register:

| knob | semantics |
| --- | --- |
| `hue-blend` | 0 = reference hue verbatim, 1 = full wallpaper mapping (shortest-arc circular interpolation between) |
| `rotate` | integer; rotates the register's assigned hues across its slots (`1` = one slot right, wraps; no-op while the register's slots share one hue) |
| `family-offset` | integer; which-ranked wallpaper family (heaviest = 0, wraps) seeds the register's hue; no-op for accents, which anchor-match instead |
| `light` | additive OKLCH lightness shift for the register's slots |
| `chroma` | multiplicative OKLCH chroma ratio for the register's slots (0.8 muted, 1.3 vivid) |

Extraction (`map-scheme --config`): `image-max-dimension`,
`chroma-pixel-floor`, `lightness-window`, `max-hues`,
`seed-separation-deg`, `cluster-iterations`, `merge-deg`,
`min-cluster-weight`, `accent-chroma-floor`, `hue-match-threshold-deg`,
`fg-contrast-floor`.

Map-wal (`map-wal --config`), all opt-in: `harmonize` (0..1 strength),
`harmonize-threshold-deg`, `quantize` (0..1 strength),
`quantize-threshold-deg`, `gray-chroma-floor`, then image-wide `light`
(additive) and `chroma` (multiplicative). Absent config = passthrough
(re-encoded).

## Why

Stylix's built-in palette generator is a genetic algorithm over raw image
clusters, with no contrast guarantees - the results are frequently muddy.
rehue separates the two things a theming tool actually needs: *which hues*
come from the wallpaper, and *how the hues are shaped* (lightness, chroma,
contrast) comes from a scheme a designer already built. `rehue map-wal` then
repaints the wallpaper to match whatever palette was derived or chosen.

## Repository

```
src/color.rs     sRGB/OKLab/OKLCH + circular-hue math (one source of truth)
src/scheme.rs    base16 YAML parse/render + slot access
src/extract.rs   chroma^2 hue histogram + deterministic circular k-means
src/register.rs  the register pipeline and its knobs
src/map_wal.rs   per-pixel harmonize/quantize/grade
src/bin/rehue.rs the CLI
tests/golden.rs  output snapshots + determinism checks
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

Parity note: rust's resize kernel (`Lanczos3`) and Pillow's `LANCZOS`
produce slightly different 8-bit pixels, so raw extraction can differ by
fractions of a degree and ±2/255 on affected slots. The engine itself is
pin-tested byte-exact via injected clusters (see `tests/golden.rs`).

## Roadmap

- Dithering for full-strength `quantize` (gradients currently band).
- base24 slot coverage (base10-17).
- A `nixosModule` exposing options wired directly to Stylix.
- Swatch-strip/preview-sheet renderer in the library.

## License

AGPL-3.0-or-later - see [LICENSE](LICENSE).