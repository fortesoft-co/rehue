#!/usr/bin/env bash
# Refresh the vendored scheme snapshot from upstream tinted-theming/schemes.
#
#   scripts/update-schemes.sh            # upstream HEAD
#   scripts/update-schemes.sh <commit>   # a specific commit or tag
#
# Replaces vendor/tinted-schemes/ (base16 + base24 yaml, the upstream MIT
# LICENSE, and a SOURCE record of the pinned commit), then runs the scheme
# tests so a malformed upstream scheme fails here, not in a user's run.
# Run it by hand before a release and commit the result as one diff.
set -euo pipefail

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
UPSTREAM="https://github.com/tinted-theming/schemes"
REF="${1:-HEAD}"
DEST="$ROOT/vendor/tinted-schemes"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

git -C "$TMP" init -q
git -C "$TMP" remote add origin "$UPSTREAM"
git -C "$TMP" fetch -q --depth 1 origin "$REF"
git -C "$TMP" checkout -q FETCH_HEAD
COMMIT="$(git -C "$TMP" rev-parse HEAD)"
DATE="$(git -C "$TMP" log -1 --format=%cs)"

rm -rf "$DEST"
mkdir -p "$DEST"
for spec in base16 base24; do
  mkdir -p "$DEST/$spec"
  find "$TMP/$spec" -maxdepth 1 -type f \( -name '*.yaml' -o -name '*.yml' \) \
    -exec cp {} "$DEST/$spec/" \;
done
cp "$TMP/LICENSE" "$DEST/LICENSE"
cat >"$DEST/SOURCE" <<EOF
upstream: $UPSTREAM
commit:   $COMMIT
date:     $DATE
specs:    base16 ($(ls "$DEST/base16" | wc -l)), base24 ($(ls "$DEST/base24" | wc -l))
EOF

echo "snapshot: tinted-theming/schemes @ ${COMMIT:0:7} ($DATE)"
cat "$DEST/SOURCE"

cd "$ROOT"
cargo test --quiet --test scheme
