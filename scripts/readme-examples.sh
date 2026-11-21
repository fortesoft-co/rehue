#!/usr/bin/env bash
# Regenerate every image the README shows, from the current code.
#
#   nix develop .#examples -c scripts/readme-examples.sh
#
# Needs: cargo, imagemagick (`magick`), and a chromium/chrome binary for
# the preview.html screenshots (override with CHROME=/path/to/browser).
# The examples devshell provides all three.
#
# Intermediate runs land in .scratch/readme-examples/ (git-ignored
# scratch); the finished images overwrite assets/examples/. Every input
# is an in-repo asset and every scheme is a bundled name, so the output
# depends only on the checkout.
set -euo pipefail

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"

WORK="$ROOT/.scratch/readme-examples"
OUT="$ROOT/assets/examples"
BIN="$ROOT/target/release/rehue"

BG='#232327'
FG='#e8e8ef'
FONT="${FONT:-DejaVu-Sans-Mono}"

need() { command -v "$1" >/dev/null || { echo "missing: $1 (try: nix develop .#examples)" >&2; exit 1; }; }
need cargo
need magick
if [[ -z "${CHROME:-}" ]]; then
  for c in chromium chromium-browser google-chrome-stable google-chrome; do
    if command -v "$c" >/dev/null; then CHROME="$c"; break; fi
  done
fi
[[ -n "${CHROME:-}" ]] || { echo "missing: chromium (or set CHROME=...)" >&2; exit 1; }

cargo build --release --quiet
rm -rf "$WORK"
mkdir -p "$WORK" "$OUT"

A="$ROOT/assets"
GLITTER="$A/vidsplay-rainbow-glitter.webp"
BUTTERFLY="$A/kelly-ishmael-butterfly-closeup.webp"
BANGO="$A/bango-renders-3d-abstract.webp"

rehue() { "$BIN" "$@" >/dev/null; }

# pair OUT.webp WIDTH LEFT_LABEL LEFT RIGHT_LABEL RIGHT
pair() {
  magick montage -background "$BG" -fill "$FG" -font "$FONT" \
    -label "$3" "$4" -label "$5" "$6" \
    -tile 2x1 -geometry "$2x+6+10" "$OUT/$1"
}

# strips OUT.png LABEL IMAGE [LABEL IMAGE ...] : scheme strips stacked
strips() {
  local out="$1"; shift
  local args=() n=0
  while (($#)); do args+=(-label "$1" "$2"); shift 2; n=$((n + 1)); done
  magick montage -background "$BG" -fill "$FG" -font "$FONT" \
    "${args[@]}" -tile "1x$n" -geometry 384x+6+10 "$OUT/$out"
}

# shot DIR VIEW OUT : screenshot DIR/preview.html with VIEW
# (palette|gtk|qt) selected. The views are css-only radio tabs, so the
# copy just moves the `checked` attribute; it sits next to the original
# so the relative wallpaper path still resolves.
shot() {
  local dir="$1" view="$2" out="$3"
  local page="$dir/.shot-$view.html" png="$WORK/shot-$view.png"
  sed -e 's/id="v-palette" checked>/id="v-palette">/' \
      -e "s/id=\"v-$view\">/id=\"v-$view\" checked>/" \
      "$dir/preview.html" >"$page"
  "$CHROME" --headless=new --disable-gpu --no-sandbox --hide-scrollbars \
    --force-device-scale-factor=1 --window-size=1280,1000 \
    --screenshot="$png" "file://$page" >/dev/null 2>&1
  rm -f "$page"
  # trim the empty page below the content, keep a margin
  magick "$png" -fuzz 2% -trim +repage -bordercolor "$(magick "$png" -format '%[pixel:p{1,1}]' info:)" \
    -border 24 "$out"
}

echo "1/5 rainbow glitter -> gruvbox light"
rehue map-wal --wallpaper "$GLITTER" --scheme gruvbox-light --out "$WORK/glitter" --harmonize 1
pair glitter-facade1-pair.webp 960 before "$GLITTER" "gruvbox-light, harmonize 1" "$WORK/glitter/wallpaper.png"

echo "2/5 rose pine dawn -> butterfly (both directions)"
rehue map-scheme --wallpaper "$BUTTERFLY" --scheme rose-pine-dawn --out "$WORK/butterfly-mapped"
cp "$WORK/butterfly-mapped/preview.png" "$OUT/butterfly-rose-pine-dawn.png"
rehue map-wal --wallpaper "$BUTTERFLY" --scheme "$WORK/butterfly-mapped/scheme.yaml" \
  --out "$WORK/butterfly-repainted" --harmonize 1
pair butterfly-roundtrip-pair.webp 480 before "$BUTTERFLY" \
  "rose-pine-dawn (re-hued), harmonize 1" "$WORK/butterfly-repainted/wallpaper.png"

echo "3/5 full round trip: tokyo night dark <-> bango"
rehue map-scheme --wallpaper "$BANGO" --scheme tokyo-night-dark --out "$WORK/bango-mapped"
cp "$WORK/bango-mapped/preview.png" "$OUT/bango-schemes-roundtrip.png"
rehue map-wal --wallpaper "$BANGO" --scheme "$WORK/bango-mapped/scheme.yaml" \
  --out "$WORK/bango-repainted" --harmonize 1
pair bango-roundtrip-pair.webp 480 before "$BANGO" \
  "tokyo-night-dark (re-hued)" "$WORK/bango-repainted/wallpaper.png"
for view in palette gtk qt; do
  shot "$WORK/bango-repainted" "$view" "$WORK/view-$view.png"
done
magick "$WORK/view-palette.png" -quality 90 "$OUT/preview-palette.webp"
# GTK and Qt side by side as one image: markdown renderers disagree on
# laying out two inline images, a single file always sits right
magick "$WORK/view-gtk.png" "$WORK/view-qt.png" \
  -background "$(magick "$WORK/view-gtk.png" -format '%[pixel:p{1,1}]' info:)" \
  -gravity north +append -quality 90 "$OUT/preview-toolkits.webp"

echo "4/5 grading: one color group at a time"
grade() { rehue map-scheme --wallpaper "$BUTTERFLY" --scheme rose-pine-dawn --out "$WORK/grade-$1" "${@:2}"; }
grade defaults
grade accents-chroma --chroma accents 1.6
grade fg-light --light fg -0.1
grade accents-rotate --distribution accents '[0,1,2,3]' --rotate accents 1
strips grading-groups.png \
  "defaults" "$WORK/grade-defaults/preview.png" \
  "--chroma accents 1.6" "$WORK/grade-accents-chroma/preview.png" \
  "--light fg -0.1" "$WORK/grade-fg-light/preview.png" \
  "--distribution accents [0,1,2,3] --rotate accents 1" "$WORK/grade-accents-rotate/preview.png"

echo "5/5 inspect: the butterfly's hue families"
rehue inspect --wallpaper "$BUTTERFLY" --out "$WORK/inspect"
# the strip is tiny (48px swatches); scale it up crisp for the readme
magick "$WORK/inspect/inspect.png" -filter point -resize 300% "$OUT/inspect-families.png"

echo "done: images written to ${OUT#"$ROOT"/}"
