#!/usr/bin/env bash
# Canonical diagram export (copied unchanged into every app). For each <src>/*.drawio writes
#   <out>/svg/<name>.svg  transparent, theme-aware (light-dark()), system fonts (nothing embedded)
#   <out>/png/<name>.png  2x, light theme, opaque white, for places that do not render SVG
# Usage: scripts/export-diagrams.sh [src-dir [out-dir [name...]]]     defaults: docs/diagrams docs/diagrams
# DRAWIO overrides the draw.io executable. Exports run one at a time to keep the machine quiet.
set -euo pipefail
cd "$(dirname "$0")/.."

src="${1:-docs/diagrams}"
out="${2:-$src}"
shift 2 2>/dev/null || shift $# 2>/dev/null || true

drawio="${DRAWIO:-}"
if [[ -z "$drawio" ]]; then
  for c in drawio draw.io "/c/Program Files/draw.io/draw.io.exe" "/Applications/draw.io.app/Contents/MacOS/draw.io"; do
    if command -v "$c" >/dev/null 2>&1 || [[ -x "$c" ]]; then drawio="$c"; break; fi
  done
fi
[[ -n "$drawio" ]] || { echo "draw.io desktop not found; install it or set DRAWIO" >&2; exit 1; }

mkdir -p "$out/svg" "$out/png"
names=("$@")
if [[ ${#names[@]} -eq 0 ]]; then
  for f in "$src"/*.drawio; do names+=("$(basename "$f" .drawio)"); done
fi

for name in "${names[@]}"; do
  f="$src/$name.drawio"
  [[ -f "$f" ]] || { echo "missing $f" >&2; exit 1; }
  "$drawio" --export --format svg --transparent --embed-svg-fonts false --border 16 --output "$out/svg/$name.svg" "$f" --no-sandbox >/dev/null
  # draw.io adds a PNG copy of every HTML label (inside <switch>, after its <foreignObject>) for viewers without
  # foreignObject support. Browsers, Gitea and GitHub render the HTML, so drop the copies: they bloat each file.
  node -e '
    const fs = require("fs"), f = process.argv[1];
    const svg = fs.readFileSync(f, "utf8").replace(/(<\/foreignObject>)<image [^>]*xlink:href="data:image\/png;base64,[^"]*"[^>]*\/>/g, "$1");
    fs.writeFileSync(f, svg);
  ' "$out/svg/$name.svg"
  "$drawio" --export --format png --scale 2 --border 16 --theme light --output "$out/png/$name.png" "$f" --no-sandbox >/dev/null
  echo "exported $name"
done
